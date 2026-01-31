//! Pending command handlers in normal mode.
//!
//! Multi-key sequences that start with a single character:
//! g*, z*, Z*, "*, m*, '*, `*, q*, @*, f/F/t/T, [*, ]*, W* (Ctrl-W), r*

use crate::editor::input::helpers;
use crate::editor::{Editor, FindDirection, FindType, Motions, Operator, PendingSemanticChange, Range};
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

/// Try to handle a pending command.
///
/// Returns `Ok(true)` if the key was handled, `Ok(false)` otherwise.
pub fn try_handle(editor: &mut Editor, key_event: KeyEvent) -> Result<bool> {
    let pending = match editor.pending_command() {
        Some(p) => p,
        None => return Ok(false),
    };

    editor.clear_pending_command();

    match (pending, key_event.code) {
        // =====================================================================
        // 'r' - Replace character
        // =====================================================================
        ('r', KeyCode::Char(ch)) => {
            handle_replace_char(editor, ch)?;
        }

        // =====================================================================
        // 'g' - Go commands
        // =====================================================================
        ('g', KeyCode::Char('g')) => {
            editor.add_jump();
            let target_line = editor.count().unwrap_or(1).saturating_sub(1);
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(target_line, 0);
            Motions::first_non_blank(editor.buffer_mut());
            editor.add_jump();
            editor.clear_count();
        }
        ('g', KeyCode::Char('d')) => {
            editor.request_goto_definition();
            editor.clear_count();
        }
        ('g', KeyCode::Char('D')) => {
            editor.request_goto_implementation();
            editor.clear_count();
        }
        ('g', KeyCode::Char('y')) => {
            editor.request_goto_type();
            editor.clear_count();
        }
        ('g', KeyCode::Char('R')) => {
            editor.request_find_references();
            editor.clear_count();
        }
        ('g', KeyCode::Char('c')) => {
            editor.request_code_actions();
            editor.clear_count();
        }
        ('g', KeyCode::Char('q')) => {
            editor.request_format_document();
            editor.clear_count();
        }
        ('g', KeyCode::Char('J')) => {
            let count = editor.effective_count();
            helpers::join_lines_no_space(editor, count)?;
            editor.clear_count();
        }
        ('g', KeyCode::Char('e')) => {
            let count = editor.effective_count();
            Motions::word_end_backward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        ('g', KeyCode::Char('E')) => {
            let count = editor.effective_count();
            Motions::word_end_backward_big(editor.buffer_mut(), count);
            editor.clear_count();
        }
        ('g', KeyCode::Char('_')) => {
            Motions::last_non_blank(editor.buffer_mut());
            editor.clear_count();
        }
        ('g', KeyCode::Char('u')) => {
            editor.set_pending_operator(Operator::Lowercase);
        }
        ('g', KeyCode::Char('U')) => {
            editor.set_pending_operator(Operator::Uppercase);
        }
        ('g', KeyCode::Char('~')) => {
            editor.set_pending_operator(Operator::ToggleCase);
        }
        ('g', KeyCode::Char('r')) => {
            // gr prefix for LSP commands
            editor.set_pending_command('R');
        }
        ('g', KeyCode::Char('i')) => {
            // gi - go to last insert position and enter insert mode
            if let Some((line, col)) = editor.editing.last_insert_position {
                editor.buffer_mut().cursor_mut().set_position(line, col);
            }
            let cursor_before = (
                editor.buffer().cursor().line(),
                editor.buffer().cursor().col(),
            );
            editor.start_change_building(cursor_before);
            editor.set_mode(Mode::Insert);
        }
        ('g', KeyCode::Char('v')) => {
            // gv - reselect last visual selection
            editor.restore_last_visual_selection();
        }
        ('g', KeyCode::Char('I')) => {
            // gI - insert at column 0
            editor.buffer_mut().cursor_mut().set_col(0);
            let cursor_before = (
                editor.buffer().cursor().line(),
                editor.buffer().cursor().col(),
            );
            editor.start_change_building(cursor_before);
            editor.set_mode(Mode::Insert);
        }
        ('g', KeyCode::Char(';')) => {
            // g; - jump to last change position
            if let Some(change) = editor.last_change() {
                let pos = change.cursor_before();
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(pos.0, pos.1);
            }
            editor.clear_count();
        }
        ('g', KeyCode::Char('t')) => {
            // gt - go to next tab
            if let Some(count) = editor.count() {
                editor.goto_tab(count.saturating_sub(1));
            } else {
                editor.next_tab();
            }
            editor.clear_count();
        }
        ('g', KeyCode::Char('T')) => {
            // gT - go to previous tab
            editor.previous_tab();
            editor.clear_count();
        }
        ('g', KeyCode::Char('n')) => {
            // gn - select next search match
            // Save pending operator before search_select_next, as set_mode() clears it
            let saved_operator = editor.pending_operator();

            if editor.search_select_next() {
                // If we have a pending operator, apply it to the visual selection
                if let Some(op) = saved_operator {
                    apply_operator_to_visual_selection(editor, op)?;
                    editor.clear_pending_operator();
                }
            }
            editor.clear_count();
        }
        ('g', KeyCode::Char('N')) => {
            // gN - select previous search match
            // Save pending operator before search_select_prev, as set_mode() clears it
            let saved_operator = editor.pending_operator();

            if editor.search_select_prev() {
                // If we have a pending operator, apply it to the visual selection
                if let Some(op) = saved_operator {
                    apply_operator_to_visual_selection(editor, op)?;
                    editor.clear_pending_operator();
                }
            }
            editor.clear_count();
        }
        ('g', KeyCode::Char('j')) => {
            // gj - move down one visual (display) line
            // With soft wrap, moves within a wrapped line's visual rows
            let count = editor.count().unwrap_or(1);
            if let Some(wrap_map) = editor.wrap_map() {
                let cursor = editor.buffer().cursor();
                let line = cursor.line();
                let char_col = cursor.col();
                let tab_width = editor.options.tab_width;

                // Convert char col to display col for wrap map operations
                let line_text = editor
                    .buffer()
                    .line(line)
                    .map(|l| l.trim_end_matches('\n').to_string())
                    .unwrap_or_default();
                let disp_col = crate::display::char_col_to_display_col(&line_text, char_col, tab_width);

                let (visual_row, _) = wrap_map.cursor_to_visual(line, disp_col);
                let target_row = visual_row + count;
                let (new_line, sub_line) = wrap_map.visual_to_logical(
                    target_row.min(wrap_map.total_visual_lines().saturating_sub(1)),
                );
                // Compute target display col and convert back to char col
                let target_disp_col = sub_line * wrap_map.wrap_width() + (disp_col % wrap_map.wrap_width());
                let new_line_text = editor
                    .buffer()
                    .line(new_line)
                    .map(|l| l.trim_end_matches('\n').to_string())
                    .unwrap_or_default();
                let new_col = crate::display::display_col_to_char_col(&new_line_text, target_disp_col, tab_width);
                // Clamp col to line length
                let max_col = new_line_text.chars().count().saturating_sub(1);
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(new_line, new_col.min(max_col));
            } else {
                // No wrap map - fall back to regular j motion
                for _ in 0..count {
                    helpers::move_down(editor);
                }
            }
            editor.clear_count();
        }
        ('g', KeyCode::Char('k')) => {
            // gk - move up one visual (display) line
            let count = editor.count().unwrap_or(1);
            if let Some(wrap_map) = editor.wrap_map() {
                let cursor = editor.buffer().cursor();
                let line = cursor.line();
                let char_col = cursor.col();
                let tab_width = editor.options.tab_width;

                let line_text = editor
                    .buffer()
                    .line(line)
                    .map(|l| l.trim_end_matches('\n').to_string())
                    .unwrap_or_default();
                let disp_col = crate::display::char_col_to_display_col(&line_text, char_col, tab_width);

                let (visual_row, _) = wrap_map.cursor_to_visual(line, disp_col);
                let target_row = visual_row.saturating_sub(count);
                let (new_line, sub_line) = wrap_map.visual_to_logical(target_row);
                let target_disp_col = sub_line * wrap_map.wrap_width() + (disp_col % wrap_map.wrap_width());
                let new_line_text = editor
                    .buffer()
                    .line(new_line)
                    .map(|l| l.trim_end_matches('\n').to_string())
                    .unwrap_or_default();
                let new_col = crate::display::display_col_to_char_col(&new_line_text, target_disp_col, tab_width);
                let max_col = new_line_text.chars().count().saturating_sub(1);
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(new_line, new_col.min(max_col));
            } else {
                // No wrap map - fall back to regular k motion
                for _ in 0..count {
                    helpers::move_up(editor);
                }
            }
            editor.clear_count();
        }

        // =====================================================================
        // 'R' - LSP gr commands (pending after 'gr')
        // =====================================================================
        ('R', KeyCode::Char('r')) => {
            editor.request_find_references();
            editor.clear_count();
        }
        ('R', KeyCode::Char('n')) => {
            // grn - LSP rename with pre-filled word under cursor
            let word = editor.buffer().word_under_cursor().map(|(w, _, _)| w);
            let word = word.unwrap_or_default();
            let cursor_pos = word.len();
            editor.set_rename_buffer(word);
            editor.set_rename_cursor(cursor_pos);
            editor.set_mode(Mode::RenameInput);
        }
        ('R', KeyCode::Char('a')) => {
            editor.request_code_actions();
            editor.clear_count();
        }
        ('R', KeyCode::Char('i')) => {
            editor.request_goto_implementation();
            editor.clear_count();
        }
        ('R', KeyCode::Char('t')) => {
            editor.request_goto_type();
            editor.clear_count();
        }

        // =====================================================================
        // 'z' - Fold/scroll commands
        // =====================================================================
        ('z', KeyCode::Char('o')) => {
            let line = editor.buffer().cursor().line();
            editor.buffer_mut().open_fold(line);
        }
        ('z', KeyCode::Char('c')) => {
            let line = editor.buffer().cursor().line();
            editor.buffer_mut().close_fold(line);
        }
        ('z', KeyCode::Char('a')) => {
            let line = editor.buffer().cursor().line();
            editor.buffer_mut().toggle_fold(line);
        }
        ('z', KeyCode::Char('R')) => {
            editor.buffer_mut().fold_manager_mut().open_all();
        }
        ('z', KeyCode::Char('M')) => {
            editor.buffer_mut().fold_manager_mut().close_all();
        }
        ('z', KeyCode::Char('d')) => {
            let line = editor.buffer().cursor().line();
            editor.buffer_mut().fold_manager_mut().delete_fold_at(line);
        }
        ('z', KeyCode::Char('E')) => {
            editor.buffer_mut().clear_folds();
        }
        ('z', KeyCode::Char('f')) => {
            editor.set_pending_operator(Operator::Fold);
        }
        ('z', KeyCode::Char('z')) => {
            editor.center_cursor_in_viewport();
            editor.clear_count();
        }
        ('z', KeyCode::Char('t')) => {
            editor.move_cursor_line_to_top();
            editor.clear_count();
        }
        ('z', KeyCode::Char('b')) => {
            editor.move_cursor_line_to_bottom();
            editor.clear_count();
        }
        ('z', KeyCode::Char('s')) => {
            // zs - scroll horizontally to put cursor at start (left edge)
            editor.scroll_cursor_to_left_edge();
            editor.clear_count();
        }
        ('z', KeyCode::Char('e')) => {
            // ze - scroll horizontally to put cursor at end (right edge)
            editor.scroll_cursor_to_right_edge();
            editor.clear_count();
        }
        ('z', KeyCode::Enter) => {
            editor.move_cursor_line_to_top();
            Motions::first_non_blank(editor.buffer_mut());
            editor.clear_count();
        }
        ('z', KeyCode::Char('-')) => {
            editor.move_cursor_line_to_bottom();
            Motions::first_non_blank(editor.buffer_mut());
            editor.clear_count();
        }
        ('z', KeyCode::Char('.')) => {
            editor.center_cursor_in_viewport();
            Motions::first_non_blank(editor.buffer_mut());
            editor.clear_count();
        }

        // =====================================================================
        // 'Z' - Save/quit commands
        // =====================================================================
        ('Z', KeyCode::Char('Z')) => {
            if editor.buffer().file_path().is_some()
                && tokio::runtime::Handle::try_current().is_ok()
            {
                let _ = editor.buffer_mut().save();
            }
            editor.quit();
        }
        ('Z', KeyCode::Char('Q')) => {
            editor.quit();
        }

        // =====================================================================
        // '"' - Register selection
        // =====================================================================
        ('"', KeyCode::Char(ch)) if ch.is_ascii_alphanumeric() || ch == '"' || ch == '_' || ch == '+' || ch == '*' => {
            editor.set_pending_register(ch);
        }

        // =====================================================================
        // 'm' - Set mark
        // =====================================================================
        ('m', KeyCode::Char(ch)) if ch.is_ascii_lowercase() || ch.is_ascii_uppercase() => {
            editor.set_mark(ch);
        }

        // =====================================================================
        // '\'' - Jump to mark line
        // =====================================================================
        ('\'', KeyCode::Char(ch)) if ch.is_ascii_lowercase() || ch.is_ascii_uppercase() => {
            editor.add_jump();
            editor.jump_to_mark_line(ch);
        }
        ('\'', KeyCode::Char('\'')) => {
            editor.jump_back();
        }

        // =====================================================================
        // '`' - Jump to mark exact position
        // =====================================================================
        ('`', KeyCode::Char(ch)) if ch.is_ascii_lowercase() || ch.is_ascii_uppercase() => {
            editor.add_jump();
            editor.jump_to_mark(ch);
        }
        ('`', KeyCode::Char('`')) => {
            editor.jump_back();
        }

        // =====================================================================
        // 'q' - Start macro recording
        // =====================================================================
        ('q', KeyCode::Char(ch)) if ch.is_ascii_lowercase() => {
            editor.start_macro_recording(ch);
        }

        // =====================================================================
        // '@' - Play macro
        // =====================================================================
        ('@', KeyCode::Char(ch)) if ch.is_ascii_lowercase() => {
            if let Some(events) = editor.get_macro(ch) {
                let events = events.clone();
                for event in events {
                    crate::editor::input::InputHandler::handle_key_event(editor, event)?;
                }
            }
        }

        // =====================================================================
        // 'f', 'F', 't', 'T' - Find character (legacy handlers for pending_command)
        // =====================================================================
        ('f', KeyCode::Char(ch)) => {
            let count = editor.effective_count();
            if Motions::find_char_forward(editor.buffer_mut(), ch, count) {
                editor.set_last_find(ch, FindType::Find, FindDirection::Forward);
            }
            editor.clear_count();
        }
        ('F', KeyCode::Char(ch)) => {
            let count = editor.effective_count();
            if Motions::find_char_backward(editor.buffer_mut(), ch, count) {
                editor.set_last_find(ch, FindType::Find, FindDirection::Backward);
            }
            editor.clear_count();
        }
        ('t', KeyCode::Char(ch)) => {
            let count = editor.effective_count();
            if Motions::till_char_forward(editor.buffer_mut(), ch, count) {
                editor.set_last_find(ch, FindType::Till, FindDirection::Forward);
            }
            editor.clear_count();
        }
        ('T', KeyCode::Char(ch)) => {
            let count = editor.effective_count();
            if Motions::till_char_backward(editor.buffer_mut(), ch, count) {
                editor.set_last_find(ch, FindType::Till, FindDirection::Backward);
            }
            editor.clear_count();
        }

        // =====================================================================
        // '[' - Section/bracket backward navigation
        // =====================================================================
        ('[', KeyCode::Char('[')) => {
            let count = editor.effective_count();
            Motions::section_backward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        ('[', KeyCode::Char(']')) => {
            let count = editor.effective_count();
            Motions::section_end_backward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        ('[', KeyCode::Char('{')) => {
            let count = editor.effective_count();
            Motions::unmatched_brace_backward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        ('[', KeyCode::Char('(')) => {
            let count = editor.effective_count();
            Motions::unmatched_paren_backward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        ('[', KeyCode::Char('m')) => {
            let count = editor.effective_count();
            Motions::method_backward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        ('[', KeyCode::Char('M')) => {
            let count = editor.effective_count();
            Motions::method_end_backward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        ('[', KeyCode::Char('d')) => {
            editor.goto_prev_diagnostic();
            editor.clear_count();
        }

        // =====================================================================
        // ']' - Section/bracket forward navigation
        // =====================================================================
        (']', KeyCode::Char(']')) => {
            let count = editor.effective_count();
            Motions::section_forward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        (']', KeyCode::Char('[')) => {
            let count = editor.effective_count();
            Motions::section_end_forward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        (']', KeyCode::Char('}')) => {
            let count = editor.effective_count();
            Motions::unmatched_brace_forward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        (']', KeyCode::Char(')')) => {
            let count = editor.effective_count();
            Motions::unmatched_paren_forward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        (']', KeyCode::Char('m')) => {
            let count = editor.effective_count();
            Motions::method_forward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        (']', KeyCode::Char('M')) => {
            let count = editor.effective_count();
            Motions::method_end_forward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        (']', KeyCode::Char('d')) => {
            editor.goto_next_diagnostic();
            editor.clear_count();
        }

        // =====================================================================
        // 'W' - Window commands (Ctrl-W prefix)
        // =====================================================================
        ('W', KeyCode::Char('w')) => {
            editor.focus_next_window();
        }
        ('W', KeyCode::Char('p')) => {
            editor.focus_prev_window();
        }
        ('W', KeyCode::Char('s')) => {
            editor.split_window_horizontal();
        }
        ('W', KeyCode::Char('v')) => {
            editor.split_window_vertical();
        }
        ('W', KeyCode::Char('o')) => {
            // <C-w>o - close all other windows (like :only)
            editor.close_other_windows();
        }
        ('W', KeyCode::Char('c')) => {
            // <C-w>c - close current window (silently fails if last window)
            let _ = editor.close_current_window();
        }
        ('W', KeyCode::Char('q')) => {
            // <C-w>q - close current window, quit if last window
            editor.close_or_quit_window();
        }
        ('W', KeyCode::Char('h')) => {
            editor.focus_window_left();
        }
        ('W', KeyCode::Char('j')) => {
            editor.focus_window_down();
        }
        ('W', KeyCode::Char('k')) => {
            editor.focus_window_up();
        }
        ('W', KeyCode::Char('l')) => {
            editor.focus_window_right();
        }

        _ => {
            // Unknown command sequence
            editor.clear_count();
        }
    }

    Ok(true)
}

/// Apply a pending operator to a visual selection created by gn/gN.
///
/// After `gn` or `gN` selects a search match and enters visual mode, this function
/// applies the pending operator (if any) to that selection. Supports:
/// - Delete operator (d)
/// - Yank operator (y)
/// - Change operator (c)
fn apply_operator_to_visual_selection(editor: &mut Editor, operator: Operator) -> Result<()> {
    match operator {
        Operator::Delete => {
            helpers::delete_visual_selection(editor)?;
            helpers::exit_visual_mode_to_normal(editor);
        }
        Operator::Yank => {
            helpers::yank_visual_selection(editor)?;
            helpers::exit_visual_mode_to_normal(editor);
        }
        Operator::Change => {
            // Capture cursor and search info before deletion
            let cursor_before_delete = (
                editor.buffer().cursor().line(),
                editor.buffer().cursor().col(),
            );
            let search_info = editor.current_search().map(|s| {
                (s.pattern().to_string(), s.is_forward())
            });

            helpers::delete_visual_selection(editor)?;

            // Pop the delete change to get deletion info for semantic change
            let delete_change = editor.pop_last_change();

            let cursor = editor.buffer().cursor();
            let cursor_after_delete = (cursor.line(), cursor.col());

            // Set up pending semantic change for cgn dot-repeat
            if let (Some(change), Some((pattern, forward))) = (delete_change, search_info) {
                let (old_text, old_range) = match &change {
                    crate::editor::Change::DeleteText { deleted_text, range, .. } => {
                        (deleted_text.clone(), range.clone())
                    }
                    _ => {
                        // Fallback: reconstruct from cursor positions
                        (String::new(), Range::default())
                    }
                };

                editor.set_pending_semantic_change(PendingSemanticChange {
                    object_type: None,
                    is_word_change: false,
                    is_search_match_change: true,
                    search_pattern: Some(pattern),
                    search_forward: Some(forward),
                    old_text,
                    old_range,
                    cursor_before: cursor_before_delete,
                });
            }

            editor.start_change_building(cursor_after_delete);
            helpers::save_and_clear_visual(editor);
            editor.set_mode(Mode::Insert);
        }
        _ => {} // Other operators not supported with gn/gN
    }
    Ok(())
}

/// Handle r{char} - replace character under cursor
fn handle_replace_char(editor: &mut Editor, ch: char) -> Result<()> {
    use crate::editor::{Change, Range};

    let count = editor.effective_count();
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let line_idx = cursor.line();
    let col = cursor.col();

    if let Some(line) = editor.buffer().line(line_idx) {
        let line_text = line.trim_end_matches('\n');
        let chars_count = line_text.chars().count();

        if col < chars_count {
            let replace_count = count.min(chars_count - col);
            let end_col = col + replace_count;

            // Delete the characters
            let deleted = editor
                .buffer_mut()
                .delete_range(line_idx, col, line_idx, end_col);

            // Insert the replacement character(s)
            let replacement = ch.to_string().repeat(replace_count);
            editor
                .buffer_mut()
                .insert_text_at(line_idx, col, &replacement);

            // Create composite change for undo/redo
            let start_pos = (line_idx, col);
            let end_pos = (line_idx, end_col);
            let range = Range::new(start_pos, end_pos);

            let delete_change = Change::delete(range, deleted, cursor_before);
            let insert_change = Change::insert((line_idx, col), replacement, cursor_before);
            let change = Change::composite(vec![delete_change, insert_change], cursor_before, cursor_before);

            editor.add_change(change);
        }
    }
    editor.clear_count();
    Ok(())
}
