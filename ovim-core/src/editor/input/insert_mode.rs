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

use crate::editor::{Change, Editor, InsertEntryMode, Range};
use crate::mode::Mode;
use crate::repeat_action::RepeatAction;
use crate::unicode::GraphemeCol;
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;

use super::helpers;

fn is_completion_trigger_char(c: char) -> bool {
    matches!(c, '.')
}

fn is_completion_ident_char(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

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
    if let Some(line) = editor.buffer().line(current_line_idx) {
        let line_without_newline = line.trim_end_matches('\n');
        // Check if line is non-empty but only whitespace
        if !line_without_newline.is_empty()
            && line_without_newline.chars().all(|c| c.is_whitespace())
        {
            // Delete the whitespace, leaving just the newline
            let whitespace_len = line_without_newline.chars().count();
            let cursor_before = (current_line_idx, whitespace_len);
            let deleted_text = line_without_newline.to_string();
            let range = Range::new((current_line_idx, 0), (current_line_idx, whitespace_len));

            // Create and apply the delete change (records for undo)
            let change = Change::delete(range, deleted_text, cursor_before);
            if !editor.apply_change_and_record(change) {
                return false;
            }

            // Move cursor to column 0 since we removed the whitespace
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(current_line_idx, GraphemeCol::ZERO);
            return true;
        }
    }
    false
}

/// Shared logic for exiting insert mode (Esc, Ctrl-[, Ctrl-C)
fn exit_insert_mode(editor: &mut Editor) {
    // Save last insert position BEFORE moving cursor (this is where we can continue inserting)
    let cursor = editor.buffer().cursor();
    editor.editing.last_insert_position = Some((cursor.line(), cursor.col().0));

    // Cleanup whitespace-only lines before finalizing changes
    cleanup_whitespace_only_line(editor);

    // Track whether finalize actually pushed an insert-mode undo entry.
    // For cases like `cw<Esc>`/`C<Esc>` where no text was typed, finalize
    // pushes nothing and we must not pop unrelated history.
    let undo_len_before_finalize = editor.buffer().change_manager().undo_stack.len();
    editor.finalize_change_building();
    let insert_change_pushed =
        editor.buffer().change_manager().undo_stack.len() > undo_len_before_finalize;

    // Check for pending change repeat (cc, C, s, S, cj, ck, cw, cgn, etc.)
    if let Some(pending) = editor.take_pending_change_repeat() {
        // Pop insert composite only if this insert session actually pushed one.
        let insert_undo = if insert_change_pushed {
            editor.pop_last_change()
        } else {
            None
        };
        let inserted_text = insert_undo
            .as_ref()
            .map(|c| c.get_inserted_text())
            .unwrap_or_default();
        // Pop delete undo only if the delete phase actually produced edits
        let delete_undo = pending
            .delete_token
            .and_then(|token| editor.pop_by_token(token));

        // Merge into single undo unit
        let cursor_before = delete_undo
            .as_ref()
            .map(|c| c.cursor_before())
            .or_else(|| insert_undo.as_ref().map(|c| c.cursor_before()))
            .unwrap_or_else(|| editor.cursor_position());
        let cursor_after = editor.cursor_position();

        let mut merged = vec![];
        if let Some(d) = delete_undo {
            merged.push(d);
        }
        if let Some(i) = insert_undo {
            merged.push(i);
        }
        if !merged.is_empty() {
            editor
                .buffer_mut()
                .change_manager_mut()
                .push_change(Change::composite(merged, cursor_before, cursor_after));
        }

        // Set semantic repeat action
        editor.set_repeat_action(RepeatAction::Change {
            delete: Box::new(pending.delete_action),
            inserted_text,
            linewise: pending.linewise,
        });
    }

    // For o/O insert sessions, use RepeatAction instead of Composite::repeat
    // special-casing so dot-repeat follows Pattern B semantics.
    let open_line_repeat = editor.last_change().and_then(|change| match change {
        Change::Composite {
            changes,
            entry_mode: InsertEntryMode::OpenBelow,
            ..
        } => {
            let mut inserted_text = String::new();
            for ch in changes.iter().skip(1) {
                inserted_text.push_str(&ch.get_inserted_text());
            }
            Some(RepeatAction::OpenLine {
                above: false,
                inserted_text,
                shift_width: editor.options.shift_width,
                expand_tab: editor.options.expand_tab,
            })
        }
        Change::Composite {
            changes,
            entry_mode: InsertEntryMode::OpenAbove,
            ..
        } => {
            let mut inserted_text = String::new();
            for ch in changes.iter().skip(1) {
                inserted_text.push_str(&ch.get_inserted_text());
            }
            Some(RepeatAction::OpenLine {
                above: true,
                inserted_text,
                shift_width: editor.options.shift_width,
                expand_tab: editor.options.expand_tab,
            })
        }
        _ => None,
    });

    // Update the . register with the last inserted text
    editor.update_last_inserted_register();
    if let Some(action) = open_line_repeat {
        editor.set_repeat_action(action);
    }

    // If we were in visual block insert/append mode, replay the changes on all other lines.
    // For visual-block change (`Ctrl-V ... c ...`), also capture a semantic repeat template.
    let pending_visual_block_change = editor.take_pending_visual_block_change_repeat();
    let pending_visual_block_delete_token = editor
        .editing
        .pending_visual_block_change_delete_token
        .take();
    let mut visual_block_change_inserted_text: Option<String> = None;
    let should_move_to_end_line = if let Some((start_line, end_line, col, is_append, move_to_end)) =
        editor.visual_block_insert_state()
    {
        // Get the text that was inserted and the first line's change
        if let Some(last_change) = editor.last_change() {
            let inserted_text = last_change.get_inserted_text();
            if !is_append && !move_to_end {
                visual_block_change_inserted_text = Some(inserted_text.clone());
            }
            let mut all_changes = vec![last_change.clone()];

            // Get cursor_before from first change
            let cursor_before = last_change.cursor_before();

            // Replay on lines start_line+1 through end_line
            for line_idx in (start_line + 1)..=end_line {
                if is_append {
                    // Append mode: insert at end of line
                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        let line_len = line_text.chars().count();
                        let version_before = editor.buffer().version();
                        editor
                            .buffer_mut()
                            .insert_text_at(line_idx, line_len, &inserted_text);
                        if editor.buffer().version() != version_before {
                            // Track this insertion as a change
                            let change = Change::insert(
                                (line_idx, line_len),
                                inserted_text.clone(),
                                cursor_before,
                            );
                            all_changes.push(change);
                        }
                    }
                } else {
                    // Insert mode: insert at column
                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        let insert_col = col.min(line_text.chars().count());
                        let version_before = editor.buffer().version();
                        editor
                            .buffer_mut()
                            .insert_text_at(line_idx, insert_col, &inserted_text);
                        if editor.buffer().version() != version_before {
                            // Track this insertion as a change
                            let change = Change::insert(
                                (line_idx, insert_col),
                                inserted_text.clone(),
                                cursor_before,
                            );
                            all_changes.push(change);
                        }
                    }
                }
            }

            // If multiple lines were affected, wrap in composite for proper undo
            if all_changes.len() > 1 {
                // Remove the last change (first line's change) from undo stack
                editor.pop_last_change();

                // Create composite for all insert changes
                let insert_composite = Change::composite(all_changes, cursor_before, cursor_before);

                // Only visual-block change (`c`) has a preceding delete entry.
                // Redeem it by token so we never pop unrelated history.
                let final_change = if let Some(token) = pending_visual_block_delete_token {
                    if let Some(prev_change) = editor.pop_by_token(token) {
                        Change::composite(
                            vec![prev_change, insert_composite],
                            cursor_before,
                            cursor_before,
                        )
                    } else {
                        insert_composite
                    }
                } else {
                    insert_composite
                };
                editor
                    .buffer_mut()
                    .change_manager_mut()
                    .push_change(final_change);
            }
        }

        // Clear the visual block insert state
        editor.set_visual_block_insert_state(None);
        Some((start_line, end_line, col, is_append, move_to_end))
    } else {
        None
    };

    if let (Some((line_count, width)), Some(inserted_text)) = (
        pending_visual_block_change,
        visual_block_change_inserted_text,
    ) {
        editor.set_repeat_action(RepeatAction::ChangeVisualBlock {
            line_count,
            width,
            inserted_text,
        });
    }

    // Mark buffer modified for LSP didChange — placed after visual block replay
    // so the server sees ALL changes (first line + replayed lines).
    editor.mark_buffer_modified();

    // Clear insert-normal flag on full exit
    editor.editing.insert_normal_pending = false;

    editor.set_mode(Mode::Normal);

    // Move cursor left when exiting insert mode (unless at column 0)

    // If we were in visual block mode, move cursor to appropriate line
    if let Some((start_line, end_line, _col, is_append, move_to_end)) = should_move_to_end_line {
        // For visual block, calculate the correct final cursor position
        let target_line = if move_to_end { end_line } else { start_line };

        if is_append {
            // For append mode, position cursor on the last character of target line
            if let Some(line) = editor.buffer().line(target_line) {
                let line_text = line.trim_end_matches('\n');
                let line_len = line_text.chars().count();
                let final_col = if line_len > 0 { line_len - 1 } else { 0 };
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(target_line, GraphemeCol(final_col));
            }
        } else {
            // For insert mode, use the same column as on the first line
            let cursor = editor.buffer().cursor();
            let current_col = cursor.col().0;
            let inserted_col = if current_col > 0 { current_col - 1 } else { 0 };
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(target_line, GraphemeCol(inserted_col));
        }
    } else {
        let cursor = editor.buffer_mut().cursor_mut();
        if cursor.col().0 > 0 {
            cursor.move_left(1);
        }
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

    match key_event.code {
        KeyCode::Esc => {
            if editor.completion_menu().is_visible() {
                editor.hide_completion_menu();
            }
            exit_insert_mode(editor);
        }
        // Ctrl-[ is equivalent to Esc
        KeyCode::Char('[') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            if editor.completion_menu().is_visible() {
                editor.hide_completion_menu();
            }
            exit_insert_mode(editor);
        }
        // Ctrl-C exits insert mode (like Esc but without triggering InsertLeave)
        KeyCode::Char('c') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            if editor.completion_menu().is_visible() {
                editor.hide_completion_menu();
            }
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
            editor.editing.insert_normal_pending = true;
            editor.set_mode(Mode::Normal);
        }
        // Ctrl-N - Next completion item
        KeyCode::Char('n') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            if editor.completion_menu().is_visible() {
                editor.completion_next();
            }
        }
        // Ctrl-P - Previous completion item
        KeyCode::Char('p') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            if editor.completion_menu().is_visible() {
                editor.completion_previous();
            }
        }
        // Ctrl-Y - Accept completion (Vim behavior)
        KeyCode::Char('y') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            if editor.completion_menu().is_visible() {
                editor.accept_completion();
            }
        }
        // Tab - Accept completion if menu is visible, otherwise insert tab
        KeyCode::Tab if editor.completion_menu().is_visible() => {
            editor.accept_completion();
        }
        KeyCode::Tab => {
            helpers::insert_tab(editor)?;
        }
        KeyCode::Char(c) => {
            helpers::insert_char(editor, c)?;
            // Basic autocomplete:
            // - Trigger on '.' (member access) immediately
            // - Trigger on '::' after typing the second ':' (Rust/C++ style paths)
            // - Trigger when typing an identifier prefix of length >= 2
            // - If menu is already visible, keep it updated while typing
            if editor.completion_menu().is_visible() {
                if is_completion_trigger_char(c) || is_completion_ident_char(c) {
                    let (_, prefix) = editor.completion_trigger_context();
                    editor.completion_menu_mut().filter(&prefix);
                    editor.request_completion();
                } else {
                    editor.hide_completion_menu();
                }
            } else if is_completion_trigger_char(c) {
                editor.request_completion();
            } else if c == ':' {
                let cursor = editor.buffer().cursor();
                if cursor.col().0 >= 2 {
                    let line_text = editor
                        .buffer()
                        .line(cursor.line())
                        .unwrap_or_default()
                        .trim_end_matches('\n')
                        .to_string();
                    if crate::unicode::grapheme_at_index(&line_text, cursor.col().0.saturating_sub(1))
                        == Some(":")
                        && crate::unicode::grapheme_at_index(
                            &line_text,
                            cursor.col().0.saturating_sub(2),
                        ) == Some(":")
                    {
                        editor.request_completion();
                    }
                }
            } else if is_completion_ident_char(c) {
                let (_, prefix) = editor.completion_trigger_context();
                if prefix.chars().count() >= 2 {
                    editor.request_completion();
                }
            }
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
            helpers::delete_char_before_cursor(editor)?;
            if editor.completion_menu().is_visible() {
                let (_, prefix) = editor.completion_trigger_context();
                if prefix.is_empty() {
                    // If we deleted back to the trigger column, keep showing member completions only
                    editor.request_completion();
                } else {
                    editor.completion_menu_mut().filter(&prefix);
                    editor.request_completion();
                }
            }
        }
        KeyCode::Left => {
            if editor.completion_menu().is_visible() {
                editor.hide_completion_menu();
            }
            let cursor = editor.buffer_mut().cursor_mut();
            if cursor.col().0 > 0 {
                cursor.move_left(1);
            }
        }
        KeyCode::Right => {
            if editor.completion_menu().is_visible() {
                editor.hide_completion_menu();
            }
            helpers::move_right(editor);
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::PendingChangeRepeat;

    #[test]
    fn exit_insert_mode_pending_change_repeat_no_insert_no_delete_keeps_prior_undo() {
        let mut editor = Editor::with_content("line\n");

        // Seed history so an accidental pop/replace is observable.
        let cursor = editor.cursor_position();
        assert!(editor.apply_change_and_record(Change::insert(cursor, "X".to_string(), cursor)));
        let undo_len_before = editor.buffer().change_manager().undo_stack.len();

        // Simulate a no-op change operator (e.g., C at EOL) entering insert mode,
        // then immediate <Esc> (no delete edits + no insert edits).
        editor.set_pending_change_repeat(PendingChangeRepeat {
            delete_action: RepeatAction::DeleteToEndOfLine,
            linewise: false,
            delete_token: None,
        });
        editor.start_change_building((0, 0));
        editor.set_mode(Mode::Insert);

        exit_insert_mode(&mut editor);

        let undo_stack = &editor.buffer().change_manager().undo_stack;
        assert_eq!(undo_stack.len(), undo_len_before);
        assert!(matches!(undo_stack.last(), Some(Change::InsertText { .. })));
    }
}
