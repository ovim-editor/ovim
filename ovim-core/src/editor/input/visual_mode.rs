//! Visual mode handler
//!
//! Handles all input events in Visual, VisualLine, and VisualBlock modes including:
//! - Visual mode motions (h/j/k/l, w/b/e, etc.)
//! - Visual mode operators (d, c, y, >, <, ~, u, U)
//! - Visual mode text objects (iw, aw, i", a{, etc.)
//! - Visual block operations (I, A, c, r)
//! - Visual mode commands (o to swap cursor, gv to reselect)
//! - Visual mode search (/ and ?)

use crate::editor::{
    BlockInsert, CursorPos, Editor, Motions, PendingChangeRepeat, RegisterType, TextObjectRange,
    TextObjectType,
};
use crate::indentation::leading_char_count;
use crate::mode::Mode;
use crate::repeat_action::{BlockColumn, RepeatAction};
use crate::unicode::{CharCol, GraphemeCol};
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;

use super::char_motion;
use super::helpers;
use super::numbers;
use crate::editor::input_state::{CharMotion, InputState, TextObjectPrefix};

/// Convert the half-open character range into inclusive grapheme endpoints.
/// Subtract in rope space so an end at column zero selects the preceding
/// newline, and a multi-codepoint final grapheme stays intact.
fn apply_text_object(editor: &mut Editor, range: TextObjectRange, linewise: bool) {
    let rope = editor.buffer().rope();
    let start_offset = rope.line_to_char(range.start_line) + range.start_col.0;
    let end_offset = rope.line_to_char(range.end_line) + range.end_col.0;
    if end_offset <= start_offset {
        return;
    }
    let last_offset = end_offset - 1;
    let end_line = rope.char_to_line(last_offset);
    let end_col = CharCol(last_offset - rope.line_to_char(end_line));
    let start_text = editor
        .buffer()
        .line_text(range.start_line)
        .unwrap_or_default();
    let start_col = crate::unicode::char_to_grapheme_col(&start_text, range.start_col);

    editor.set_mode(if linewise {
        Mode::VisualLine
    } else {
        Mode::Visual
    });
    editor.set_visual_start(range.start_line, start_col.0);
    editor.buffer_mut().set_cursor_char_col(end_line, end_col);
}

/// Mirror of normal-mode `cc` for a VisualLine-c selection: delete all
/// selected lines as a single recorded edit, open a blank line at the
/// deletion site preserving the deleted line's indent, and set up
/// PendingChangeRepeat for dot-repeat via `RepeatAction::Change`.
///
/// Vim reference (`vim -N -u NONE`): `VcNEW<Esc>` on
/// `"line one\nline two\nline three\n"` → `"NEW\nline two\nline three\n"`.
/// `j.` after that → `"NEW\nNEW\nline three\n"`.
fn handle_visual_line_change(editor: &mut Editor) -> Result<()> {
    let Some(((start_line, _), (end_line, _))) = editor.visual_selection() else {
        // No selection → nothing to delete; fall through to plain insert.
        return Ok(());
    };
    let line_count = end_line.saturating_sub(start_line) + 1;
    let cursor_before = editor.cursor_position();

    // Capture indent from the first selected line before deleting (matches
    // normal-mode `cc`: the opened blank line inherits the first line's
    // indent). Note: ovim preserves indent unconditionally; Vim only does so
    // with `autoindent`, but this matches the ovim convention established by
    // `handle_cc` / `substitute_line`.
    let indent = editor
        .buffer()
        .line_text(start_line)
        .map(|l| {
            l.chars()
                .take_while(|c| c.is_whitespace() && *c != '\n')
                .collect::<String>()
        })
        .unwrap_or_default();

    // Phase 1: delete selected lines + open blank with indent, atomically.
    let (deleted, edits) = editor.buffer_mut().record(|buf| {
        let line_count_total = buf.line_count();
        let delete_end = (end_line + 1).min(line_count_total);
        let deleted = buf.delete_range(start_line, CharCol::ZERO, delete_end, CharCol::ZERO);
        let insert_at = start_line.min(buf.line_count());
        buf.insert_text_at(insert_at, CharCol::ZERO, &format!("{}\n", indent));
        buf.cursor_mut()
            .set_position(insert_at, GraphemeCol(indent.len()));
        deleted
    });

    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        Some(editor.push_recorded_undo(edits, cursor_before, cursor_after))
    } else {
        None
    };

    if !deleted.is_empty() {
        editor.delete_to_register_with_type(deleted, RegisterType::Line);
    }

    // Phase 2: set up dot-repeat + insert-mode change building.
    let delete_action = RepeatAction::DeleteVisualLine { line_count };
    // Mirror the install that `delete_visual_selection_with_token` would
    // have done, so other consumers that read last_repeat_action observe
    // the same semantic delete.
    editor.set_repeat_action(delete_action.clone());

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action,
        linewise: true,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    Ok(())
}

fn handle_visual_leader_input(
    editor: &mut Editor,
    key_event: KeyEvent,
    keys: &[char],
) -> Result<()> {
    if key_event.code == KeyCode::Esc {
        editor.reset_input_state();
        return Ok(());
    }

    let KeyCode::Char(c) = key_event.code else {
        editor.reset_input_state();
        return Ok(());
    };

    if keys.is_empty() {
        match c {
            // <Space><Space> in visual mode: attach the selection to AI chat.
            ' ' => {
                editor.start_ai_chat_from_visual()?;
                editor.reset_input_state();
            }
            _ => {
                editor.reset_input_state();
            }
        }
        return Ok(());
    }

    editor.reset_input_state();

    Ok(())
}

/// Handles input in Visual mode (Visual, VisualLine, VisualBlock)
pub fn handle_visual_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    editor.visual.key_selection = editor
        .visual_selection()
        .map(|(start, end)| (start, end, editor.mode()));
    let result = handle_visual_key(editor, key_event);
    editor.visual.key_selection = None;
    result
}

fn handle_visual_key(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    // =====================================================================
    // INPUT STATE CHECK (must happen before mode-specific handling)
    // =====================================================================
    // If we're awaiting a character (for f/t/F/T motions), handle that first
    // before processing any visual mode specific keys. This prevents conflicts
    // where the target character (like 'e' in 'fe') would be interpreted as
    // a motion command instead of the search target.
    if let InputState::AwaitingChar { motion, operator } = editor.input_state().clone() {
        return char_motion::handle_char_motion(editor, key_event, motion, operator);
    }
    if let InputState::Leader { ref keys } = editor.input_state().clone() {
        let keys_clone = keys.clone();
        return handle_visual_leader_input(editor, key_event, &keys_clone);
    }

    // Handle pending command prefixes (g, i/a text-objects, etc.)
    if let Some(pending) = editor.input_state().prefix_key() {
        editor.reset_input_state();
        match (pending, key_event.code) {
            ('"', key) => {
                if let KeyCode::Char(register) = key {
                    if crate::editor::RegisterManager::is_valid_name(register) {
                        editor.set_pending_register(register);
                    }
                    return Ok(());
                }
                if key != KeyCode::Esc {
                    return Ok(());
                }
            }
            ('g', KeyCode::Char('g')) => {
                // gg - go to first line (or line specified by count)
                let target_line = if let Some(count) = editor.count() {
                    count.saturating_sub(1)
                } else {
                    0
                };

                let is_visual_block = editor.mode() == Mode::VisualBlock;
                let current_col = editor.buffer().cursor().col();
                let cursor = editor.buffer_mut().cursor_mut();
                cursor.set_line(target_line);

                if !is_visual_block {
                    cursor.set_col(GraphemeCol(0));
                    cursor.update_desired_col(GraphemeCol(0));
                } else {
                    cursor.set_col(current_col);
                    cursor.update_desired_col(current_col);
                }

                helpers::clamp_cursor_to_line(editor);
                editor.clear_count();
                return Ok(());
            }
            ('g', KeyCode::Char(c @ ('a' | 'x')))
                if key_event.modifiers.contains(Modifiers::CONTROL) =>
            {
                // g Ctrl-A / g Ctrl-X: sequential increment/decrement in the selection
                let delta = if c == 'a' { 1 } else { -1 };
                let count = editor.effective_count() as i64;
                numbers::modify_numbers_in_selection(editor, delta * count, true)?;
                editor.clear_count();
                helpers::exit_visual_mode_to_normal(editor);
                return Ok(());
            }
            ('g', KeyCode::Char('n')) => {
                // gn - extend selection to next search match
                if !editor.search_select_next() {
                    editor.set_status_message("Pattern not found".to_string());
                }
                editor.clear_count();
                return Ok(());
            }
            ('g', KeyCode::Char('N')) => {
                // gN - extend selection to previous search match
                if !editor.search_select_prev() {
                    editor.set_status_message("Pattern not found".to_string());
                }
                editor.clear_count();
                return Ok(());
            }
            ('i' | 'a', key) if key != KeyCode::Esc => {
                if let Some(object) = super::text_objects::from_key(editor, key, pending == 'i') {
                    if let Some(range) = object.resolve(editor.buffer()) {
                        let linewise = matches!(object, TextObjectType::Paragraph { .. });
                        apply_text_object(editor, range, linewise);
                    }
                }
                editor.clear_count();
                return Ok(());
            }
            _ => {
                // Unknown pending command, ignore
            }
        }
    }

    match key_event.code {
        KeyCode::Char('"') => editor.set_input_state(InputState::RegisterPending),
        KeyCode::Esc => {
            helpers::exit_visual_mode_to_normal(editor);
        }
        KeyCode::Char(' ') => {
            editor.set_input_state(InputState::Leader { keys: Vec::new() });
        }
        // Ctrl chords are never the plain-letter command (Ctrl-C is not `c`, Ctrl-X not `x`).
        KeyCode::Char(c) if key_event.modifiers.contains(Modifiers::CONTROL) => {
            handle_visual_ctrl(editor, c)?;
        }
        // Text object prefixes in visual mode
        KeyCode::Char(c @ ('i' | 'a')) => {
            // Text object prefix (iw, aw, ip, ap, i{, a{, etc.)
            editor.set_input_state(InputState::TextObjectPending {
                operator: None,
                prefix: TextObjectPrefix::from_char(c).expect("i or a"),
            });
        }
        // Motion keys work in visual mode too
        KeyCode::Char('h') | KeyCode::Left => {
            editor.set_visual_block_dollar(false);
            helpers::move_left(editor);
        }
        KeyCode::Char('j') | KeyCode::Down => {
            helpers::move_down(editor);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            helpers::move_up(editor);
        }
        KeyCode::Char('l') | KeyCode::Right => {
            editor.set_visual_block_dollar(false);
            helpers::move_right(editor);
        }
        KeyCode::Char('w') => {
            editor.set_visual_block_dollar(false);
            let count = editor.effective_count();
            Motions::word_forward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        KeyCode::Char('b') => {
            editor.set_visual_block_dollar(false);
            let count = editor.effective_count();
            Motions::word_backward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        KeyCode::Char('e') => {
            editor.set_visual_block_dollar(false);
            let count = editor.effective_count();
            Motions::word_end_forward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        KeyCode::Char('0') | KeyCode::Home => {
            editor.set_visual_block_dollar(false);
            // If there's already a count, treat `0` as a digit (e.g., "50j")
            // Otherwise (and for <Home>), treat it as a motion to column 0
            if key_event.code == KeyCode::Char('0') && editor.count().is_some() {
                editor.append_count(0);
            } else {
                editor.buffer_mut().cursor_mut().set_col(GraphemeCol(0));
            }
        }
        KeyCode::Char('$') | KeyCode::End => {
            if editor.mode() == Mode::VisualBlock {
                // Set "extend to end-of-line" flag so each line in the block
                // is deleted/yanked to its own end, not to a fixed column.
                editor.set_visual_block_dollar(true);
                let line_idx = editor.buffer().cursor().line();
                if let Some(line) = editor.buffer().line_text(line_idx) {
                    let line_len = line.chars().count();
                    let col = if line_len > 0 { line_len - 1 } else { 0 };
                    let cursor = editor.buffer_mut().cursor_mut();
                    cursor.set_col(GraphemeCol(col));
                    cursor.update_desired_col(GraphemeCol(usize::MAX));
                }
            } else {
                // Characterwise: `$` goes onto the line break, so the
                // selection includes it (`v$d` joins the next line), and
                // `[count]$` goes down `count - 1` lines first.
                let down = editor.effective_count() - 1;
                let max_line = editor.buffer().line_count().saturating_sub(1);
                let line_idx = (editor.buffer().cursor().line() + down).min(max_line);
                let line_len = editor.buffer().line_index(line_idx).grapheme_count();
                let cursor = editor.buffer_mut().cursor_mut();
                cursor.set_position(line_idx, GraphemeCol(line_len));
                // Set desired_col to usize::MAX to indicate "always end of line"
                cursor.update_desired_col(GraphemeCol(usize::MAX));
                editor.clear_count();
            }
        }
        KeyCode::Char('G') => {
            // G - go to last line (or line specified by count)
            let target_line = if let Some(count) = editor.count() {
                count.saturating_sub(1)
            } else {
                editor.buffer().line_count().saturating_sub(1)
            };
            let is_visual_block = editor.mode() == Mode::VisualBlock;
            let current_col = editor.buffer().cursor().col();
            let cursor = editor.buffer_mut().cursor_mut();
            cursor.set_line(target_line);

            if !is_visual_block {
                cursor.set_col(GraphemeCol(0));
                cursor.update_desired_col(GraphemeCol(0));
            } else {
                cursor.set_col(current_col);
                cursor.update_desired_col(current_col);
            }

            helpers::clamp_cursor_to_line(editor);
            editor.clear_count();
        }
        KeyCode::Char('g') => {
            // g - first key of gg (go to first line with optional count)
            editor.set_input_state(InputState::GPrefix { operator: None });
        }
        // Find character forward (f)
        KeyCode::Char('f') => {
            editor.set_input_state(InputState::AwaitingChar {
                motion: CharMotion::Find,
                operator: None,
            });
        }
        // Find character backward (F)
        KeyCode::Char('F') => {
            editor.set_input_state(InputState::AwaitingChar {
                motion: CharMotion::FindBack,
                operator: None,
            });
        }
        // Till character forward (t)
        KeyCode::Char('t') => {
            editor.set_input_state(InputState::AwaitingChar {
                motion: CharMotion::Till,
                operator: None,
            });
        }
        // Till character backward (T)
        KeyCode::Char('T') => {
            editor.set_input_state(InputState::AwaitingChar {
                motion: CharMotion::TillBack,
                operator: None,
            });
        }
        // Jump to mark exact position (`)
        KeyCode::Char('`') => {
            editor.set_input_state(InputState::AwaitingChar {
                motion: CharMotion::JumpMarkExact,
                operator: None,
            });
        }
        // Jump to mark line (')
        KeyCode::Char('\'') => {
            editor.set_input_state(InputState::AwaitingChar {
                motion: CharMotion::JumpMarkLine,
                operator: None,
            });
        }
        // Repeat the last find motion in visual modes (`;`/`,`).
        KeyCode::Char(';') => {
            editor.repeat_last_find(false);
        }
        KeyCode::Char(',') => {
            editor.repeat_last_find(true);
        }
        // Paragraph motions
        KeyCode::Char(key @ ('}' | '{')) => {
            editor.set_visual_block_dollar(false);
            let count = editor.effective_count();
            editor.record_jump_if_moved(|editor| {
                if key == '}' {
                    Motions::paragraph_forward(editor.buffer_mut(), count)
                } else {
                    Motions::paragraph_backward(editor.buffer_mut(), count)
                }
            });
            editor.clear_count();
        }
        // Jump to matching bracket (%)
        KeyCode::Char('%') => {
            Motions::jump_to_matching_bracket(editor.buffer_mut());
            editor.clear_count();
        }
        // The command line on the selected lines (`:'<,'>`)
        KeyCode::Char(':') => {
            helpers::exit_visual_mode_to_normal(editor);
            editor.set_command_line("'<,'>");
            editor.set_mode(Mode::Command);
        }
        // Search forward in visual mode
        KeyCode::Char('/') => {
            // Save visual search state for extending selection after search
            if let Some((anchor_line, anchor_col)) = editor.visual_start() {
                let mode = editor.mode();
                editor.set_visual_search_state((anchor_line, anchor_col), mode);
            }
            editor.begin_search(true);
        }
        // Search backward in visual mode
        KeyCode::Char('?') => {
            // Save visual search state for extending selection after search
            if let Some((anchor_line, anchor_col)) = editor.visual_start() {
                let mode = editor.mode();
                editor.set_visual_search_state((anchor_line, anchor_col), mode);
            }
            editor.begin_search(false);
        }
        // Search next in visual mode
        KeyCode::Char('n') => {
            editor.search_next();
        }
        // Search previous in visual mode
        KeyCode::Char('N') => {
            editor.search_prev();
        }
        // Search forward for selected text (* in visual mode)
        KeyCode::Char('*') => {
            if !helpers::search_visual_selection_forward(editor) {
                editor.set_status_message("Pattern not found".to_string());
            }
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Search backward for selected text (# in visual mode)
        KeyCode::Char('#') => {
            if !helpers::search_visual_selection_backward(editor) {
                editor.set_status_message("Pattern not found".to_string());
            }
            helpers::exit_visual_mode_to_normal(editor);
        }
        // `s` is `c`; `Y`, `D`, `C`, `X`, `S` and `R` act on whole lines, except in
        // Visual-block mode, where `D` and `C` go to the end of the line and `X` and
        // `Y` are `d` and `y` (Vim's `v_visop`).
        KeyCode::Char(key @ ('s' | 'Y' | 'D' | 'C' | 'X' | 'S' | 'R')) => {
            let block = editor.mode() == Mode::VisualBlock;
            let register = editor.pending_register();
            if block && matches!(key, 'D' | 'C') {
                let end_of_line = KeyEvent::new(KeyCode::Char('$'), Modifiers::NONE);
                handle_visual_key(editor, end_of_line)?;
            } else if key != 's' && (!block || matches!(key, 'S' | 'R')) {
                editor.set_visual_block_dollar(false);
                editor.set_mode(Mode::VisualLine);
                if let Some(register) = register {
                    editor.set_pending_register(register);
                }
            }
            let operator = match key {
                'Y' => 'y',
                'D' | 'X' => 'd',
                _ => 'c',
            };
            return handle_visual_key(
                editor,
                KeyEvent::new(KeyCode::Char(operator), Modifiers::NONE),
            );
        }
        // Delete selection
        KeyCode::Char('d') | KeyCode::Char('x') | KeyCode::Delete => {
            helpers::delete_visual_selection(editor)?;
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Yank selection
        KeyCode::Char('y') => {
            let selection = editor.visual_selection();
            let mode = editor.mode();
            helpers::yank_visual_selection(editor)?;
            // Save the complete selection before moving to its start; `gv`
            // must restore the yanked range, not the collapsed cursor position.
            helpers::exit_visual_mode_to_normal(editor);
            if let Some(((start_line, start_col), (end_line, end_col))) = selection {
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(start_line, GraphemeCol(start_col));
                // Flash the yanked region
                if mode == Mode::VisualLine {
                    editor.set_yank_flash_lines(start_line, end_line);
                } else {
                    editor.set_yank_flash_range(
                        start_line,
                        GraphemeCol(start_col),
                        end_line,
                        GraphemeCol(end_col),
                    );
                }
            }
        }
        // Change selection
        KeyCode::Char('c') => {
            let mode_before = editor.mode();
            editor.visual.block_insert = None;

            // VisualLine-c must mirror normal-mode `cc`: delete the whole
            // line(s), open a blank line at the deletion site (preserving the
            // deleted line's indent), and enter insert mode there. The naive
            // path of `delete_visual_selection_with_token` deletes the
            // trailing newline(s) too, which would fuse the inserted text
            // with the following line. See dot_repeat_test.rs
            // `test_dot_after_visual_line_change_multichar` and Vim's
            // behavior (`vim -N -u NONE` on `VcNEW<Esc>`).
            if mode_before == Mode::VisualLine {
                handle_visual_line_change(editor)?;
                helpers::save_and_clear_visual(editor);
                editor.set_mode(Mode::Insert);
                return Ok(());
            }

            // For visual block mode, need to track the block for multi-line insert
            let visual_block_state = if mode_before == Mode::VisualBlock {
                editor
                    .visual_selection()
                    .map(|((start_line, start_col), (end_line, end_col))| {
                        let width = end_col.saturating_sub(start_col) + 1;
                        (start_line, end_line, start_col, width)
                    })
            } else {
                None
            };

            let delete_token = helpers::delete_visual_selection_with_token(editor)?;

            if let Some((start_line, end_line, start_col, width)) = visual_block_state {
                // Replicated onto the other block lines when Insert mode ends.
                editor.visual.block_insert = Some(BlockInsert {
                    start_line,
                    end_line,
                    left_col: start_col,
                    column: BlockColumn::Insert(0),
                    change: Some((width, delete_token)),
                });
                let cursor_before = CursorPos::new(start_line, GraphemeCol(start_col));
                // The delete clamps the cursor onto the last character; when the
                // block reached the end of the line, insertion continues after it.
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(start_line, GraphemeCol(start_col));
                editor.start_change_building(cursor_before);
            } else if delete_token.is_some() {
                // Regular visual (v) with a non-empty selection: route the
                // change through PendingChangeRepeat + RepeatAction::Change
                // so that the full inserted text is captured for dot-repeat
                // (matching cw/cc/C semantics).
                //
                // delete_visual_selection_with_token has already installed
                // DeleteVisualChar as last_repeat_action; clone it as the
                // delete template for the Change action.
                let delete_action = editor
                    .buffer()
                    .change_manager()
                    .last_repeat_action
                    .clone()
                    .expect(
                        "delete_visual_selection_with_token installs a RepeatAction for \
                         non-empty selections",
                    );
                editor.set_pending_change_repeat(PendingChangeRepeat {
                    delete_action,
                    linewise: false,
                    delete_token,
                });
                editor.start_change_building(editor.cursor_position());
            }
            // else: empty selection — fall through to plain insert mode (no
            // dot-repeat template set; matches the pre-fix behavior for an
            // empty visual-c).

            helpers::save_and_clear_visual(editor);
            editor.set_mode(Mode::Insert);
        }
        // Join lines
        KeyCode::Char('J') => {
            if let Some(((start_line, _), (end_line, _))) = editor.visual_selection() {
                // Calculate expected cursor position after join
                // The cursor should be at the last space inserted (before the last line)
                let mut cursor_col = 0;
                for line_idx in start_line..end_line {
                    // Note: end_line not included
                    if let Some(line_text) = editor.buffer().line_text(line_idx) {
                        cursor_col += line_text.chars().count();
                        if line_idx < end_line - 1 {
                            cursor_col += 1; // Space after this line
                        }
                    }
                }

                // Join all lines in the selection
                let count = (end_line - start_line) + 1;
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(start_line, GraphemeCol(0));
                helpers::join_lines(editor, count)?;

                // Position cursor at the last inserted space
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(start_line, GraphemeCol(cursor_col));
            }
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Move to other end of selection
        KeyCode::Char('o') => {
            if let Some(visual_start) = editor.visual_start() {
                let cursor = editor.buffer().cursor();
                let cursor_pos = (cursor.line(), cursor.col().0);

                if editor.mode() == Mode::VisualBlock {
                    // For visual block mode, flip to diagonally opposite corner
                    // Swap line from one with column from the other
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(visual_start.0, GraphemeCol(cursor_pos.1));
                    editor.set_visual_start(cursor_pos.0, visual_start.1);
                } else {
                    // For other visual modes, swap positions normally
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(visual_start.0, GraphemeCol(visual_start.1));
                    editor.set_visual_start(cursor_pos.0, cursor_pos.1);
                }
            }
        }
        // Flip horizontally (uppercase O) - swap columns only
        KeyCode::Char('O') => {
            if let Some(visual_start) = editor.visual_start() {
                let cursor = editor.buffer().cursor();
                let cursor_pos = (cursor.line(), cursor.col().0);

                if editor.mode() == Mode::VisualBlock {
                    // For visual block mode, flip horizontally (swap columns only, keep line)
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(cursor_pos.0, GraphemeCol(visual_start.1));
                    editor.set_visual_start(visual_start.0, cursor_pos.1);
                } else {
                    // For other visual modes, same as 'o'
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(visual_start.0, GraphemeCol(visual_start.1));
                    editor.set_visual_start(cursor_pos.0, cursor_pos.1);
                }
            }
        }
        // Switch to other visual modes
        KeyCode::Char('v') => {
            if editor.mode() == Mode::Visual {
                helpers::exit_visual_mode_to_normal(editor);
            } else {
                // Switching to Visual mode from VisualLine or VisualBlock
                editor.set_mode(Mode::Visual);
            }
        }
        KeyCode::Char('V') => {
            if editor.mode() == Mode::VisualLine {
                helpers::exit_visual_mode_to_normal(editor);
            } else {
                // Switching to VisualLine mode
                if let Some((anchor_line, _)) = editor.visual_start() {
                    editor.set_visual_start(anchor_line, 0);
                } else {
                    let cursor = editor.buffer().cursor();
                    editor.set_visual_start(cursor.line(), 0);
                }
                editor.set_mode(Mode::VisualLine);
            }
        }
        // Visual block insert/append
        KeyCode::Char('I') => {
            if editor.mode() == Mode::VisualBlock {
                // Insert at the block's left edge on each line.
                if let Some(((start_line, start_col), (end_line, _))) = editor.visual_selection() {
                    let block = BlockInsert {
                        start_line,
                        end_line,
                        left_col: start_col,
                        column: BlockColumn::Insert(0),
                        change: None,
                    };
                    begin_block_insert(editor, block, start_col);
                }
            } else if let Some(((start_line, _), _)) = editor.visual_selection() {
                // Char/line visual: vim makes `I` linewise, inserting at
                // column 0 of the first selected line (nvim: `vjIX` puts X
                // at the very start of the first line only).
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(start_line, GraphemeCol::ZERO);
                editor.clear_visual_start();
                editor.start_change_building(editor.cursor_position());
                editor.set_mode(Mode::Insert);
            }
        }
        KeyCode::Char('A') => {
            if editor.mode() == Mode::VisualBlock {
                // Append after the block on each line.
                if let Some(((start_line, start_col), (end_line, end_col))) =
                    editor.visual_selection()
                {
                    // Clamp to the first line so a ragged block still appends
                    // right after that line's text.
                    let line_len = editor
                        .buffer()
                        .line_text(start_line)
                        .map(|l| l.chars().count())
                        .unwrap_or(0);
                    let append_col = end_col.min(line_len.saturating_sub(1)) + 1;
                    // A block created "to end of line" — via `$` inside block
                    // mode, or a `$` before entering it (sticky MAXCOL) —
                    // appends at each line's own end; a fixed-column block at
                    // the block column (padding short lines).
                    let column = if editor.visual_block_dollar() {
                        BlockColumn::EndOfLine
                    } else {
                        BlockColumn::Append(append_col.saturating_sub(start_col))
                    };
                    let block = BlockInsert {
                        start_line,
                        end_line,
                        left_col: start_col,
                        column,
                        change: None,
                    };
                    begin_block_insert(editor, block, append_col);
                }
            } else if let Some((_, (end_line, end_col))) = editor.visual_selection() {
                // Char/line visual: append after the end of the selection.
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(end_line, GraphemeCol(end_col + 1));
                editor.clear_visual_start();
                editor.start_change_building(editor.cursor_position());
                editor.set_mode(Mode::Insert);
            }
        }
        // Replace in visual mode (all visual variants)
        KeyCode::Char('r') => {
            // r{char} in any visual mode - wait for replacement character via input state.
            editor.set_input_state(InputState::AwaitingChar {
                motion: CharMotion::Replace,
                operator: None,
            });
        }
        // Case operations in visual mode
        KeyCode::Char('~') => {
            helpers::toggle_case_visual_selection(editor)?;
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Paste in visual mode (replace selection)
        KeyCode::Char(c @ ('p' | 'P')) => visual_put(editor, c == 'P')?,
        // Uppercase in visual mode
        KeyCode::Char('U') => {
            helpers::uppercase_visual_selection(editor)?;
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Lowercase in visual mode
        KeyCode::Char('u') => {
            helpers::lowercase_visual_selection(editor)?;
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Indent/dedent in visual mode
        KeyCode::Char('>') => {
            if let Some(((start_line, _), (end_line, _))) = editor.visual_selection() {
                let cursor = editor.buffer().cursor();
                let cursor_before = CursorPos::new(cursor.line(), cursor.col());
                let is_visual_block = editor.mode() == Mode::VisualBlock;
                let original_col = cursor_before.col.0;
                let old_indent_chars = editor
                    .buffer()
                    .line_text(end_line)
                    .map(|line| leading_char_count(&line))
                    .unwrap_or(0);

                helpers::indent_lines_with_tracking(
                    editor,
                    start_line,
                    end_line + 1,
                    cursor_before,
                )?;

                // For visual block mode, move cursor to end line at adjusted column
                if is_visual_block {
                    let new_indent_chars = editor
                        .buffer()
                        .line_text(end_line)
                        .map(|line| leading_char_count(&line))
                        .unwrap_or(0);
                    let adjusted_col = original_col
                        .saturating_sub(old_indent_chars)
                        .saturating_add(new_indent_chars);
                    let cursor = editor.buffer_mut().cursor_mut();
                    cursor.set_position(end_line, GraphemeCol(adjusted_col));
                }
            }
            helpers::exit_visual_mode_to_normal(editor);
        }
        KeyCode::Char('<') => {
            if let Some(((start_line, _), (end_line, _))) = editor.visual_selection() {
                let cursor = editor.buffer().cursor();
                let cursor_before = CursorPos::new(cursor.line(), cursor.col());
                let is_visual_block = editor.mode() == Mode::VisualBlock;

                helpers::dedent_lines_with_tracking(
                    editor,
                    start_line,
                    end_line + 1,
                    cursor_before,
                )?;

                // For visual block mode, move cursor to start position (start_line, 0)
                if is_visual_block {
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(start_line, GraphemeCol(0));
                }
            }
            helpers::exit_visual_mode_to_normal(editor);
        }
        KeyCode::Char('=') => {
            if let Some(((start_line, _), (end_line, _))) = editor.visual_selection() {
                let options = editor.indent_options();
                let cursor_before = editor.cursor_position();
                let ((), edits) = editor.buffer_mut().record(|buf| {
                    let _ = helpers::auto_indent_lines(buf, start_line, end_line + 1, options);
                });
                if !edits.is_empty() {
                    let cursor_after = editor.cursor_position();
                    // push_recorded_undo() calls mark_buffer_modified() internally
                    editor.push_recorded_undo(edits, cursor_before, cursor_after);
                }
            }
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Count prefix (for motions like 5j, 10w)
        KeyCode::Char(c) if c.is_ascii_digit() => {
            let digit = c.to_digit(10).unwrap() as usize;
            // 0 is handled separately above as a motion
            if digit != 0 || editor.count().is_some() {
                editor.append_count(digit);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Visual-mode Ctrl chords. Vim's that this editor supports run here; every other
/// chord is a no-op that leaves the selection alone (it must never fall through to
/// the plain-letter command, e.g. Ctrl-C deleting the selection as `c`).
fn handle_visual_ctrl(editor: &mut Editor, c: char) -> Result<()> {
    match c {
        // Ctrl-C / Ctrl-[ leave Visual mode like Esc.
        'c' | '[' => {
            helpers::exit_visual_mode_to_normal(editor);
            return Ok(());
        }
        // Ctrl-V / Ctrl-Q: switch to (or leave) blockwise Visual mode.
        'v' | 'q' => {
            if editor.mode() == Mode::VisualBlock {
                helpers::exit_visual_mode_to_normal(editor);
            } else {
                editor.set_mode(Mode::VisualBlock);
            }
            return Ok(());
        }
        'a' | 'x' => {
            let count = editor.effective_count() as i64;
            let delta = if c == 'a' { count } else { -count };
            numbers::modify_numbers_in_selection(editor, delta, false)?;
            editor.clear_count();
            helpers::exit_visual_mode_to_normal(editor);
            return Ok(());
        }
        // Half-page scroll.
        'd' => {
            let half_page = editor.half_page_scroll();
            let count = editor.count().unwrap_or(half_page);
            let max_line = editor.buffer().line_count().saturating_sub(1);

            let cursor = editor.buffer_mut().cursor_mut();
            let new_line = (cursor.line() + count).min(max_line);
            cursor.set_line(new_line);
            helpers::clamp_cursor_to_line(editor);
        }
        'u' => {
            let half_page = editor.half_page_scroll();
            let count = editor.count().unwrap_or(half_page);
            let cursor = editor.buffer_mut().cursor_mut();
            let new_line = cursor.line().saturating_sub(count);
            cursor.set_line(new_line);
            helpers::clamp_cursor_to_line(editor);
        }
        // Line scroll and page scroll move the view (the cursor follows).
        'e' => editor.scroll_viewport_down(editor.effective_count()),
        'y' => editor.scroll_viewport_up(editor.effective_count()),
        'f' => (0..editor.effective_count()).for_each(|_| editor.scroll_page_down()),
        'b' => (0..editor.effective_count()).for_each(|_| editor.scroll_page_up()),
        // Ctrl-N / Ctrl-J are `j`, Ctrl-P is `k`, Ctrl-H is `h`.
        'n' | 'j' => helpers::move_down(editor),
        'p' => helpers::move_up(editor),
        'h' => {
            editor.set_visual_block_dollar(false);
            helpers::move_left(editor);
        }
        _ => {}
    }
    editor.clear_count();
    Ok(())
}

/// Visual `p` / `P`: replace the selection with the register contents.
///
/// The contents are captured *before* the selection is deleted — the delete
/// overwrites the unnamed register and, under `clipboard=unnamedplus`, the
/// clipboard — and that capture is what gets put. Afterwards `p` leaves the
/// replaced text in the unnamed register/clipboard (so a second `p` swaps back);
/// `P` deletes into the black hole and leaves them alone (nvim).
fn visual_put(editor: &mut Editor, keep_registers: bool) -> Result<()> {
    let register = editor.pending_register();
    let (text, reg_type) = editor.get_from_register_with_type();
    let selection = editor.visual_selection();
    let mode = editor.mode();
    let Some(((start_line, _), (end_line, _))) = selection.filter(|_| !text.is_empty()) else {
        // Nothing to put (vim: E353) — leave the selection's text alone.
        helpers::exit_visual_mode_to_normal(editor);
        return Ok(());
    };
    let reached_buffer_end = end_line + 1 >= editor.buffer().line_count();

    // Delete and put undo together.
    let undo_mark = editor.buffer().change_manager().undo_mark();
    if keep_registers {
        editor.set_pending_register('_');
    }
    helpers::delete_visual_selection(editor)?;
    let replaced = (!keep_registers).then(|| {
        let (text, reg_type) = editor.registers().get_default_with_type();
        (text.to_string(), reg_type)
    });

    if mode == Mode::VisualLine {
        // A characterwise/blockwise register put over whole lines becomes lines.
        let text = if text.ends_with('\n') || reg_type != RegisterType::Character {
            text
        } else {
            format!("{text}\n")
        };
        let emptied = editor.buffer().line_count() == 1
            && editor.buffer().line_text(0).is_none_or(|l| l.is_empty());
        if reached_buffer_end && start_line > 0 && !emptied {
            // The deleted lines were last: the cursor sits on the line above.
            helpers::paste_text_after(editor, text, RegisterType::Line, register, 1)?;
        } else {
            helpers::paste_text_before(editor, text, RegisterType::Line, register, 1)?;
        }
    } else if reg_type == RegisterType::Line && mode == Mode::Visual {
        // Linewise text goes between the two halves of the split line.
        split_line_at_cursor(editor);
        helpers::paste_text_after(editor, text, reg_type, register, 1)?;
    } else {
        // Character: paste at the position the selection started. After
        // deleting the selection the cursor sits on the surviving char at
        // that column. paste_after inserts *after* the cursor, so step
        // back one column first — but at column 0 there's nothing to step
        // back over, so paste_before (which inserts AT the cursor column)
        // is required to avoid a one-char misplacement.
        let cursor_col = editor.buffer().cursor().col().0;
        if cursor_col > 0 {
            editor
                .buffer_mut()
                .cursor_mut()
                .set_col(GraphemeCol(cursor_col - 1));
            helpers::paste_text_after(editor, text, reg_type, register, 1)?;
        } else {
            helpers::paste_text_before(editor, text, reg_type, register, 1)?;
        }
    }

    editor
        .buffer_mut()
        .change_manager_mut()
        .group_since(undo_mark);
    if let Some((text, reg_type)) = replaced {
        if !editor.options.clipboard.is_empty() {
            editor.registers_mut().set_clipboard(text.clone());
        }
        editor.registers_mut().set_with_type(None, text, reg_type);
    }
    helpers::exit_visual_mode_to_normal(editor);
    Ok(())
}

/// Breaks the cursor line in two at the cursor, leaving the cursor on the first
/// half (the second half starts with the character that was under the cursor).
fn split_line_at_cursor(editor: &mut Editor) {
    let cursor_before = editor.cursor_position();
    let line = cursor_before.line;
    let col = editor.buffer().cursor_char_col();
    let ((), edits) = editor.buffer_mut().record(|buf| {
        buf.insert_text_at(line, col, "\n");
    });
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(line, cursor_before.col);
    if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        editor.push_recorded_undo(edits, cursor_before, cursor_after);
    }
}

/// Enter Insert mode for a visual-block `I` / `A` at `col` on the first
/// block line; the typed text is replicated when Insert mode ends.
fn begin_block_insert(editor: &mut Editor, block: BlockInsert, col: usize) {
    let cursor = CursorPos::new(block.start_line, GraphemeCol(col));
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(cursor.line, cursor.col);
    editor.visual.block_insert = Some(block);
    editor.set_visual_block_dollar(false);
    editor.clear_visual_start();
    editor.start_change_building(cursor);
    editor.set_mode(Mode::Insert);
}
