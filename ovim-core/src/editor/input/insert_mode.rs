//! Insert mode handler
//!
//! Handles all input events in Insert mode including:
//! - Character insertion
//! - Backspace/Delete handling
//! - Ctrl+W (delete word backward)
//! - Ctrl+U (delete to line start)
//! - Ctrl+N/Ctrl+P (completion navigation)
//! - Visual block insert state handling
//! - Tab/auto-indent

use crate::change::ChangeToken;
use crate::editor::{BlockInsert, Change, CompletionAcceptMode, Editor, InsertEntryMode};
use crate::mode::Mode;
use crate::repeat_action::RepeatAction;
use crate::unicode::{CharCol, GraphemeCol};
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;

use super::helpers;
use crate::editor::editing_state::{LiteralKind, PendingLiteral};

/// Cleans up whitespace-only lines before exiting insert mode.
///
/// Vim behavior: if the current line contains only whitespace when exiting insert mode,
/// remove the whitespace (e.g., o<Esc> should leave an empty line, not an indented one).
///
/// This must be called BEFORE finalize_change_building() so it's part of the undo group.
///
/// Returns true if cleanup was performed (which means cursor shouldn't move left).
fn cleanup_whitespace_only_line(editor: &mut Editor) -> bool {
    let current_line_idx = editor.buffer().cursor().line();
    if let Some(line) = editor.buffer().line_text(current_line_idx) {
        let line_without_newline = line;
        // Check if line is non-empty but only whitespace
        if !line_without_newline.is_empty()
            && line_without_newline.chars().all(|c| c.is_whitespace())
        {
            // Delete the whitespace, leaving just the newline.
            // Whitespace is ASCII, so char count == grapheme count here.
            let whitespace_len = line_without_newline.chars().count();

            // Record the deletion for undo. `delete_range_positioning_cursor`
            // lands the cursor at char col 0 (== grapheme col 0).
            if !editor.record_session_edit(|buf| {
                buf.delete_range_positioning_cursor(
                    current_line_idx,
                    CharCol::ZERO,
                    current_line_idx,
                    CharCol(whitespace_len),
                )
                .0
            }) {
                return false;
            }
            return true;
        }
    }
    false
}

/// Shared logic for exiting insert mode (Esc, Ctrl-[, Ctrl-C)
fn exit_insert_mode(editor: &mut Editor) {
    finish_insert_mode(editor, false);
}

/// Close the current recording before a temporary normal command can mutate
/// the buffer. Ctrl-O keeps the insertion position and starts a fresh undo
/// unit when the normal command completes.
fn finish_insert_mode(editor: &mut Editor, temporary: bool) {
    editor.end_snippet_session();
    editor.clear_signature_help();
    // Save last insert position BEFORE moving cursor (this is where we can continue inserting)
    let cursor = editor.buffer().cursor();
    editor.editing.last_insert_position = Some((cursor.line(), cursor.col().0));

    // Cleanup whitespace-only lines before finalizing changes
    if !temporary {
        cleanup_whitespace_only_line(editor);
    }

    let session = editor.finalize_change_building();

    // Check for pending change repeat (cc, C, s, S, cj, ck, cw, cgn, etc.)
    if let Some(pending) = editor.take_pending_change_repeat() {
        // The insert session's `Recorded`, if the session typed anything.
        let insert_undo = session.and_then(|token| editor.pop_by_token(token));
        let inserted_text = insert_undo
            .as_ref()
            .map(|c| c.get_inserted_text())
            .unwrap_or_default();
        let insert_cursor_before = insert_undo.as_ref().map(|c| c.cursor_before());
        let insert_edits = insert_undo.and_then(|c| c.into_edits()).unwrap_or_default();

        // Pop delete undo only if the delete phase actually produced edits.
        let delete_undo = pending
            .delete_token
            .and_then(|token| editor.pop_by_token(token));

        let cursor_before = delete_undo
            .as_ref()
            .map(|c| c.cursor_before())
            .or(insert_cursor_before)
            .unwrap_or_else(|| editor.cursor_position());
        let cursor_after = editor.cursor_position();

        let delete_edits = delete_undo.and_then(|c| c.into_edits()).unwrap_or_default();
        let mut merged = delete_edits;
        merged.extend(insert_edits);
        if !merged.is_empty() {
            editor
                .buffer_mut()
                .change_manager_mut()
                .push_change(Change::recorded(merged, cursor_before, cursor_after));
        }

        // Set semantic repeat action
        editor.set_repeat_action(RepeatAction::Change {
            delete: Box::new(pending.delete_action),
            inserted_text,
            linewise: pending.linewise,
        });
    }

    // For o/O insert sessions, promote dot-repeat to RepeatAction::OpenLine
    // so the replay opens a new line at the current cursor instead of
    // replaying the original session's newline-insert edit verbatim.
    let open_line_repeat = match editor.buffer().change_manager().last_repeat_action.as_ref() {
        Some(RepeatAction::InsertSession {
            entry_mode: mode @ (InsertEntryMode::OpenBelow | InsertEntryMode::OpenAbove),
            edits,
            ..
        }) => {
            // Skip the first edit — that's the synthetic newline created by
            // `insert_line_below` / `insert_line_above` before the user's
            // keystrokes. `RepeatAction::OpenLine` will recreate its own.
            let inserted_text = crate::edit::surviving_inserted_text(&edits[1..]);
            Some(RepeatAction::OpenLine {
                above: matches!(mode, InsertEntryMode::OpenAbove),
                inserted_text,
                options: editor.indent_options(),
            })
        }
        _ => None,
    };
    if let Some(action) = open_line_repeat {
        editor.set_repeat_action(action);
    }

    let block = editor.visual.block_insert.take();
    if let Some(block) = &block {
        replicate_block_insert(editor, block, session);
    }

    // Mark buffer modified for LSP didChange — placed after visual block replay
    // so the server sees ALL changes (first line + replayed lines). The
    // sibling replay is wrapped in `buffer.record()` so `edit_log` already
    // includes its edits — no fixup needed.
    editor.mark_buffer_modified();

    // Clear insert-normal flag on full exit
    editor.editing.insert_normal_pending = false;

    editor.set_mode(Mode::Normal);
    if temporary {
        return;
    }

    match block {
        // vim: a visual-block I / A leaves the cursor at the block's top-left.
        Some(BlockInsert {
            start_line,
            left_col,
            change: None,
            ..
        }) => {
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(start_line, GraphemeCol(left_col));
            editor.buffer_mut().validate_cursor_position();
        }
        // Otherwise the cursor steps back onto the last inserted character.
        _ => {
            let cursor = editor.buffer_mut().cursor_mut();
            if cursor.col().0 > 0 {
                cursor.move_left(1);
            }
        }
    }
}

/// Visual-block I / A / c: replay the text typed on the first block line on
/// the other lines, make the whole block (and the `c` delete) one undo step
/// and install the block repeat for `.`.
fn replicate_block_insert(editor: &mut Editor, block: &BlockInsert, session: Option<ChangeToken>) {
    let delete_width = block.change.map_or(0, |(width, _)| width);
    let session = session.and_then(|token| editor.pop_by_token(token));
    let inserted_text = session
        .as_ref()
        .map(Change::get_inserted_text)
        .unwrap_or_default();

    if let Some(session) = session {
        // Undo returns to the block's top-left, where the session began.
        let cursor_before = session.cursor_before();
        let mut edits = session.into_edits().unwrap_or_default();
        let column = block.column.offset_by(block.left_col);
        let ((), sibling_edits) = editor.buffer_mut().record(|buf| {
            column.insert_on_lines(
                buf,
                block.start_line + 1..block.end_line + 1,
                &inserted_text,
            )
        });
        edits.extend(sibling_edits);
        let delete_token = block.change.and_then(|(_, token)| token);
        if let Some(delete) = delete_token.and_then(|token| editor.pop_by_token(token)) {
            let mut merged = delete.into_edits().unwrap_or_default();
            merged.extend(edits);
            edits = merged;
        }
        let cursor_after = editor.cursor_position();
        editor
            .buffer_mut()
            .change_manager_mut()
            .push_change(Change::recorded(edits, cursor_before, cursor_after));
    } else if delete_width == 0 {
        // An I / A that typed nothing changed nothing; `.` keeps its target.
        return;
    }

    editor.set_repeat_action(RepeatAction::VisualBlockInsert {
        line_count: block.end_line - block.start_line + 1,
        delete_width,
        column: block.column,
        inserted_text,
    });
}

/// Whether `key` leaves a completion session alone: typing into the word (the
/// menu filters), Backspace, the keys that move through or accept the menu,
/// and Ctrl-Space.
fn keeps_completion_menu(key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Char(c) if key.modifiers.contains(Modifiers::CONTROL) => {
            matches!(c, 'n' | 'p' | 'y' | ' ')
        }
        KeyCode::Char(_)
        | KeyCode::Backspace
        | KeyCode::Tab
        | KeyCode::Enter
        | KeyCode::Up
        | KeyCode::Down => true,
        _ => false,
    }
}

/// Handles input in Insert mode
pub fn handle_insert_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    // Handle pending register insert (Ctrl-R {reg})
    if editor.editing.pending_register_insert {
        editor.editing.pending_register_insert = false;
        if let KeyCode::Char(c) = key_event.code {
            let text = editor.registers().get(Some(c));
            if !text.is_empty() {
                for ch in text.chars() {
                    if ch == '\n' {
                        helpers::insert_newline(editor)?;
                    } else {
                        helpers::insert_char(editor, ch)?;
                    }
                }
            }
        }
        return Ok(());
    }

    if let Some(pending) = editor.editing.pending_literal.take() {
        return handle_literal_key(editor, pending, key_event);
    }

    // Any key that is not typing into the word or driving the menu moves the
    // cursor or rewrites the line (Ctrl-O, Ctrl-W, Ctrl-T...): the menu was
    // built for the text before the cursor as it was, so it must go.
    if !keeps_completion_menu(&key_event) {
        editor.dismiss_completion();
    }

    let signature_help_was_active = editor.signature_help_active();
    if !matches!(
        key_event.code,
        KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Tab | KeyCode::BackTab
    ) {
        editor.snippet_clear_pending();
    }
    match key_event.code {
        KeyCode::Esc => {
            editor.dismiss_completion();
            exit_insert_mode(editor);
        }
        // Ctrl-[ is equivalent to Esc
        KeyCode::Char('[') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.dismiss_completion();
            exit_insert_mode(editor);
        }
        // Ctrl-C exits insert mode (like Esc but without triggering InsertLeave)
        KeyCode::Char('c') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.dismiss_completion();
            exit_insert_mode(editor);
        }
        // Ctrl-W - Delete word backward
        KeyCode::Char('w') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            helpers::delete_word_backward_insert(editor)?;
        }
        // Ctrl-U - Delete to start of line
        KeyCode::Char('u') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            helpers::delete_to_line_start_insert(editor)?;
        }
        // Ctrl-T - Indent current line in insert mode
        KeyCode::Char('t') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            helpers::indent_line_insert(editor)?;
        }
        // Ctrl-D - Dedent current line in insert mode
        KeyCode::Char('d') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            helpers::dedent_line_insert(editor)?;
        }
        // Ctrl-H is equivalent to Backspace
        KeyCode::Char('h') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            helpers::delete_char_before_cursor(editor)?;
        }
        // Ctrl-R - Insert register contents
        KeyCode::Char('r') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.editing.pending_register_insert = true;
        }
        // Ctrl-Space - Request code completion
        KeyCode::Char(' ') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.request_completion();
        }
        // Ctrl-O - Execute one normal mode command, then return to insert
        KeyCode::Char('o') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            let cursor = *editor.buffer().cursor();
            let len = editor.buffer().line_index(cursor.line()).grapheme_count();
            let at_eol = cursor.col().0 >= len;
            editor.editing.insert_normal_eol_line = at_eol.then(|| cursor.line());
            // A goal column of "end of line" (`$`) survives the clamp as well.
            let goal = if cursor.desired_col() == usize::MAX {
                usize::MAX
            } else {
                len
            };
            editor.editing.insert_normal_eol_goal = at_eol.then_some(goal);
            finish_insert_mode(editor, true);
            editor.editing.insert_normal_pending = true;
        }
        // Ctrl-N - Next completion item
        KeyCode::Char('n') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            if editor.completion_menu().is_visible() {
                editor.completion_next();
            } else {
                editor.request_completion();
            }
        }
        // Ctrl-P - Previous completion item
        KeyCode::Char('p') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            if editor.completion_menu().is_visible() {
                editor.completion_previous();
            } else {
                editor.request_completion();
            }
        }
        // Ctrl-Y - Accept completion (Vim behavior)
        KeyCode::Char('y') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            if editor.completion_menu().is_visible() {
                editor.accept_completion();
            }
        }
        // Tab - Accept the completion (replacing the identifier under the
        // cursor, IntelliJ style) if the menu is visible; otherwise jump to
        // the next snippet tab stop; otherwise insert a tab.
        KeyCode::Tab if editor.completion_menu().is_visible() => {
            editor.accept_completion_with(CompletionAcceptMode::Replace);
        }
        KeyCode::Tab => {
            if !(editor.snippet_active() && editor.snippet_jump(true)) {
                helpers::insert_tab(editor)?;
            }
        }
        // Shift-Tab - previous snippet tab stop
        KeyCode::BackTab => {
            if editor.snippet_active() {
                editor.snippet_jump(false);
            }
        }
        // Ctrl-A - insert the text of the previous insert (the `.` register)
        KeyCode::Char('a') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.dismiss_completion();
            let text = editor.registers().get_last_inserted().to_string();
            for ch in text.chars() {
                if ch == '\n' {
                    helpers::insert_newline(editor)?;
                } else {
                    helpers::insert_char(editor, ch)?;
                }
            }
        }
        // Ctrl-V / Ctrl-Q - insert the next key (or a number) literally
        KeyCode::Char('v' | 'q') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.dismiss_completion();
            editor.editing.pending_literal = Some(PendingLiteral::Start);
        }
        // Ctrl-M / Ctrl-J - same as Enter
        KeyCode::Char('m' | 'j') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.dismiss_completion();
            helpers::insert_newline(editor)?;
        }
        // Ctrl-I - same as Tab
        KeyCode::Char('i') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            helpers::insert_tab(editor)?;
        }
        // Any other Ctrl chord is not text: it does nothing instead of inserting its letter.
        KeyCode::Char(_) if key_event.modifiers.contains(Modifiers::CONTROL) => {}
        KeyCode::Char(c) => {
            // A commit character accepts the highlighted item before it is
            // typed itself (`foo.` accepts `foo` when the server says so).
            if editor.completion_menu().is_visible() {
                editor.try_commit_completion(c);
            }
            // First keystroke over a freshly entered snippet placeholder
            // replaces its default text.
            editor.snippet_replace_pending_placeholder();
            helpers::electric_dedent_close_bracket(editor, c)?;
            helpers::insert_char(editor, c)?;
            // Auto-popup: keep an open menu filtered, or start one on
            // identifier typing / server trigger characters.
            editor.completion_after_typed_char(c);
        }
        KeyCode::Enter => {
            // If completion menu is visible, accept the selected completion
            if editor.completion_menu().is_visible() {
                editor.accept_completion();
            } else {
                helpers::insert_newline(editor)?;
            }
        }
        KeyCode::Backspace => {
            if !editor.snippet_replace_pending_placeholder() {
                helpers::delete_char_before_cursor(editor)?;
            }
            editor.completion_after_backspace();
        }
        KeyCode::Left => {
            editor.dismiss_completion();
            let cursor = editor.buffer_mut().cursor_mut();
            if cursor.col().0 > 0 {
                cursor.move_left(1);
            }
        }
        KeyCode::Right => {
            editor.dismiss_completion();
            helpers::move_right(editor);
        }
        KeyCode::Delete => {
            editor.dismiss_completion();
            helpers::delete_char_at_cursor_insert(editor)?;
        }
        KeyCode::Home => {
            editor.dismiss_completion();
            editor.buffer_mut().cursor_mut().set_col(GraphemeCol::ZERO);
        }
        KeyCode::End => {
            editor.dismiss_completion();
            let line = editor.buffer().cursor().line();
            let len = editor.buffer().line_index(line).grapheme_count();
            editor.buffer_mut().cursor_mut().set_col(GraphemeCol(len));
        }
        KeyCode::Up => {
            if editor.completion_menu().is_visible() {
                editor.completion_previous();
            } else {
                helpers::move_up(editor);
            }
        }
        KeyCode::Down => {
            if editor.completion_menu().is_visible() {
                editor.completion_next();
            } else {
                helpers::move_down(editor);
            }
        }
        _ => {}
    }
    editor.snippet_after_key();
    request_signature_help_after_key(editor, &key_event, signature_help_was_active);
    Ok(())
}

impl LiteralKind {
    fn max_digits(self) -> usize {
        match self {
            Self::Decimal | Self::Octal => 3,
            Self::Hex(digits) => digits,
        }
    }

    fn radix(self) -> u32 {
        match self {
            Self::Decimal => 10,
            Self::Octal => 8,
            Self::Hex(_) => 16,
        }
    }

    /// Whether `digit` continues the number spelled so far.
    fn extends(self, digits: &str, digit: char) -> bool {
        digit.is_digit(self.radix()) && digits.len() < self.max_digits()
    }

    /// The largest character code the kind can spell (`<C-v>256` is 255).
    fn max_value(self) -> u32 {
        match self {
            Self::Decimal => 255,
            Self::Octal => 0o377,
            Self::Hex(_) => u32::MAX,
        }
    }
}

/// What a `<C-v>` sequence has typed so far becomes (`None` when it spells no
/// insertable character: no digit at all, NUL, and line breaks are not inserted).
fn literal_char(kind: LiteralKind, digits: &str) -> Option<char> {
    let value = u32::from_str_radix(digits, kind.radix()).ok()?;
    char::from_u32(value.min(kind.max_value())).filter(|c| !matches!(c, '\0' | '\n' | '\r'))
}

/// Insert-mode `<C-v>`: the next key is inserted as is (`<C-v><Tab>` is a real tab,
/// `<C-v><C-a>` a control character), or starts a number (`<C-v>065`, `<C-v>x41`,
/// `<C-v>u20ac`) that ends at its last digit or at the first key that is not one.
fn handle_literal_key(
    editor: &mut Editor,
    pending: PendingLiteral,
    key_event: KeyEvent,
) -> Result<()> {
    let ctrl = key_event.modifiers.contains(Modifiers::CONTROL);
    match pending {
        PendingLiteral::Start => match key_event.code {
            KeyCode::Char(c) if ctrl => {
                if c.is_ascii_alphabetic() {
                    helpers::insert_char(editor, char::from(c.to_ascii_lowercase() as u8 & 0x1f))?;
                }
            }
            KeyCode::Char(c) => {
                let kind = match c {
                    '0'..='9' => Some((LiteralKind::Decimal, String::from(c))),
                    'o' | 'O' => Some((LiteralKind::Octal, String::new())),
                    'x' | 'X' => Some((LiteralKind::Hex(2), String::new())),
                    'u' => Some((LiteralKind::Hex(4), String::new())),
                    'U' => Some((LiteralKind::Hex(8), String::new())),
                    _ => None,
                };
                match kind {
                    Some((kind, digits)) => {
                        editor.editing.pending_literal =
                            Some(PendingLiteral::Digits { kind, digits });
                    }
                    None => helpers::insert_char(editor, c)?,
                }
            }
            KeyCode::Tab => helpers::insert_char(editor, '\t')?,
            KeyCode::Esc => helpers::insert_char(editor, '\x1b')?,
            _ => {}
        },
        PendingLiteral::Digits { kind, mut digits } => {
            let next_digit = match key_event.code {
                KeyCode::Char(c) if !ctrl && kind.extends(&digits, c) => Some(c),
                _ => None,
            };
            if let Some(digit) = next_digit {
                digits.push(digit);
                if digits.len() < kind.max_digits() {
                    editor.editing.pending_literal = Some(PendingLiteral::Digits { kind, digits });
                } else if let Some(c) = literal_char(kind, &digits) {
                    helpers::insert_char(editor, c)?;
                }
            } else {
                // The number ends here; the key that ended it is typed as usual.
                if let Some(c) = literal_char(kind, &digits) {
                    helpers::insert_char(editor, c)?;
                }
                return handle_insert_mode(editor, key_event);
            }
        }
    }
    Ok(())
}

/// Parameter hints: `(` and `,` open the popup; while it is open every edit or
/// cursor move re-asks the server so the active parameter follows the cursor
/// (the server answers with nothing once the cursor leaves the call).
fn request_signature_help_after_key(editor: &mut Editor, key_event: &KeyEvent, was_active: bool) {
    if editor.mode() != Mode::Insert {
        return;
    }
    let retrigger = match key_event.code {
        KeyCode::Char('(') | KeyCode::Char(',')
            if !key_event.modifiers.contains(Modifiers::CONTROL) =>
        {
            true
        }
        KeyCode::Char(_)
        | KeyCode::Backspace
        | KeyCode::Delete
        | KeyCode::Left
        | KeyCode::Right
        | KeyCode::Up
        | KeyCode::Down
        | KeyCode::Enter => was_active,
        _ => false,
    };
    // Moving the cursor into an unfinished call (or jumping between the
    // placeholders of an expanded method snippet) brings the popup back.
    if !retrigger
        && matches!(
            key_event.code,
            KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Tab
                | KeyCode::BackTab
        )
    {
        editor.request_signature_help_if_in_call();
        return;
    }
    if retrigger {
        editor.request_signature_help();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{ApplyPos, CursorPos, PendingChangeRepeat};

    fn type_key(editor: &mut Editor, code: KeyCode) {
        handle_insert_mode(editor, KeyEvent::new(code, Modifiers::NONE)).unwrap();
    }

    /// OV-00451: `(` and `,` open signature help; while it is open every edit
    /// retriggers (the active parameter follows the cursor); Esc dismisses.
    #[test]
    fn signature_help_triggers_retriggers_and_dismisses() {
        let mut editor = Editor::with_content("");
        editor.start_change_building(editor.cursor_position());
        editor.set_mode(Mode::Insert);

        type_key(&mut editor, KeyCode::Char('f'));
        assert!(
            !editor.lsp.intents.signature_help,
            "plain letters do not trigger"
        );

        type_key(&mut editor, KeyCode::Char('('));
        assert!(editor.lsp.intents.signature_help, "`(` triggers");
        editor.lsp.intents.signature_help = false;

        // Simulate the popup being visible.
        editor.lsp.state.signature_help = Some(Box::new(crate::editor::SignatureHelpState {
            label: "f(int a, int b)".into(),
            active_param: Some((2, 7)),
            active_param_index: Some(0),
            signature_index: 0,
            signature_count: 1,
            documentation: None,
            parameter_documentation: None,
            anchor: (0, 2),
        }));
        type_key(&mut editor, KeyCode::Char('1'));
        assert!(
            editor.lsp.intents.signature_help,
            "typing retriggers while open"
        );
        editor.lsp.intents.signature_help = false;
        type_key(&mut editor, KeyCode::Char(','));
        assert!(editor.lsp.intents.signature_help, "`,` retriggers");
        editor.lsp.intents.signature_help = false;
        type_key(&mut editor, KeyCode::Backspace);
        assert!(
            editor.lsp.intents.signature_help,
            "backspace retriggers while open"
        );
        editor.lsp.intents.signature_help = false;

        type_key(&mut editor, KeyCode::Esc);
        assert!(editor.signature_help().is_none(), "Esc dismisses the popup");
        assert!(!editor.lsp.intents.signature_help);
    }

    /// OV-00474: moving the cursor (Left/Right/Up/Down/Tab) into the argument
    /// list of an unfinished call shows the popup again, like VS Code; moving
    /// out of it, or into a plain condition, does not.
    #[test]
    fn moving_back_into_an_unfinished_call_shows_signature_help() {
        let mut editor = Editor::with_content("run(a, b)");
        editor.start_change_building(editor.cursor_position());
        editor.set_mode(Mode::Insert);
        editor
            .buffer_mut()
            .set_cursor_char_col(0, crate::unicode::CharCol(9));

        // After the `)`: not inside the call.
        type_key(&mut editor, KeyCode::Right);
        assert!(!editor.lsp.intents.signature_help);
        // One step left is between `b` and `)`: inside the argument list.
        type_key(&mut editor, KeyCode::Left);
        assert!(editor.lsp.intents.signature_help, "Left into the call");
        editor.lsp.intents.signature_help = false;
        // Leaving the call again (Right, after the `)`) asks nothing new.
        type_key(&mut editor, KeyCode::Right);
        assert!(!editor.lsp.intents.signature_help);

        // A condition is not a call.
        let mut editor = Editor::with_content("if (a > b) {");
        editor.start_change_building(editor.cursor_position());
        editor.set_mode(Mode::Insert);
        editor
            .buffer_mut()
            .set_cursor_char_col(0, crate::unicode::CharCol(9));
        type_key(&mut editor, KeyCode::Left);
        assert!(!editor.lsp.intents.signature_help);
    }

    #[test]
    fn signature_help_scan_follows_call_syntax() {
        use crate::editor::lsp_integration::lsp_modules::signature_help::text_ends_inside_call;
        for inside in [
            "foo(",
            "foo(a, ",
            "obj.method(1, bar(2), ",
            "new Foo<>(x, (y + ",
            "call (a,\n    b, ",
            "f(g(1)(",
            "list.<String>of(",
            "{ foo(",
        ] {
            assert!(text_ends_inside_call(inside), "{inside:?}");
        }
        for outside in [
            "foo(a)",
            "foo(a); bar",
            "if (a > ",
            "while (x",
            "= (a + b",
            "foo(a) {",
            "x = 1;",
            "",
        ] {
            assert!(!text_ends_inside_call(outside), "{outside:?}");
        }
    }

    #[test]
    fn exit_insert_mode_pending_change_repeat_no_insert_no_delete_keeps_prior_undo() {
        let mut editor = Editor::with_content("line\n");

        // Seed history so an accidental pop/replace is observable. Opens a
        // throwaway session around the seed edit because `record_session_edit`
        // requires an active recording session post-Signal-A cleanup.
        let cursor = editor.cursor_position();
        let apply = ApplyPos::new(cursor.line, CharCol(cursor.col.0));
        editor.start_change_building(cursor);
        assert!(editor.record_session_edit(|buf| {
            buf.insert_text_at_positioning_cursor(apply.line, apply.col, "X")
        }));
        editor.finalize_change_building();
        let undo_len_before = editor.buffer().change_manager().undo_stack.len();

        // Simulate a no-op change operator (e.g., C at EOL) entering insert mode,
        // then immediate <Esc> (no delete edits + no insert edits).
        editor.set_pending_change_repeat(PendingChangeRepeat {
            delete_action: RepeatAction::DeleteToEndOfLine,
            linewise: false,
            delete_token: None,
        });
        editor.start_change_building(CursorPos::ZERO);
        editor.set_mode(Mode::Insert);

        exit_insert_mode(&mut editor);

        let undo_stack = &editor.buffer().change_manager().undo_stack;
        assert_eq!(undo_stack.len(), undo_len_before);
        // After step 4.3 the direct-path push is a `Recorded`, not `InsertText`.
        assert!(matches!(
            undo_stack.last().map(|entry| &entry.change),
            Some(Change::Recorded { .. })
        ));
    }
}
