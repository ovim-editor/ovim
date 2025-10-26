//! Insert mode input handling

use super::helpers;
use crate::buffer::Buffer;
use crate::editor::{
    Change, Editor, FindDirection, FindType, Motions, Operator, Operators, Range, RegisterType,
    Search, TextObjects,
};
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Handles input in Insert mode
pub(super) fn handle_insert_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        match key_event.code {
            KeyCode::Esc => {
                // If completion menu is visible, hide it first without exiting insert mode
                if editor.completion_menu().is_visible() {
                    editor.hide_completion_menu();
                } else {
                    // Save last insert position BEFORE moving cursor (this is where we can continue inserting)
                    let cursor = editor.buffer().cursor();
                    editor.last_insert_position = Some((cursor.line(), cursor.col()));

                    editor.finalize_change_building();
                    // Update the . register with the last inserted text
                    editor.update_last_inserted_register();
                    editor.mark_buffer_modified(); // Mark for LSP didChange notification

                    // If we were in visual block insert/append mode, replay the changes on all other lines
                    let should_move_to_end_line =
                        if let Some((start_line, end_line, col, is_append, move_to_end)) =
                            editor.visual_block_insert_state()
                        {
                            // Get the text that was inserted and the first line's change
                            if let Some(last_change) = editor.last_change() {
                                let inserted_text = last_change.get_inserted_text();
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
                                            editor.buffer_mut().insert_text_at(
                                                line_idx,
                                                line_len,
                                                &inserted_text,
                                            );
                                            // Track this insertion as a change
                                            let change = Change::insert(
                                                (line_idx, line_len),
                                                inserted_text.clone(),
                                                cursor_before,
                                            );
                                            all_changes.push(change);
                                        }
                                    } else {
                                        // Insert mode: insert at column
                                        if let Some(line) = editor.buffer().line(line_idx) {
                                            let line_text = line.trim_end_matches('\n');
                                            let insert_col = col.min(line_text.chars().count());
                                            editor.buffer_mut().insert_text_at(
                                                line_idx,
                                                insert_col,
                                                &inserted_text,
                                            );
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

                                // If multiple lines were affected, wrap in composite for proper undo
                                if all_changes.len() > 1 {
                                    // Remove the last change (first line's change) from undo stack
                                    editor.pop_last_change();

                                    // Create composite for all insert changes
                                    let insert_composite = Change::composite(
                                        all_changes,
                                        cursor_before,
                                        cursor_before,
                                    );

                                    // Check if there's a delete composite on the stack (from visual block 'c')
                                    // If so, combine delete + insert into a super-composite
                                    if let Some(prev_change) = editor.pop_last_change() {
                                        // Check if previous change looks like a visual block delete
                                        // (it would be a composite or delete change)
                                        // Combine them into a super-composite
                                        let super_composite = Change::composite(
                                            vec![prev_change, insert_composite],
                                            cursor_before,
                                            cursor_before,
                                        );
                                        editor.add_change(super_composite);
                                    } else {
                                        // No previous change, just add the insert composite
                                        editor.add_change(insert_composite);
                                    }
                                }
                            }

                            // Clear the visual block insert state
                            editor.set_visual_block_insert_state(None);
                            Some((start_line, end_line, col, is_append, move_to_end))
                        } else {
                            None
                        };

                    editor.set_mode(Mode::Normal);

                    // Move cursor left when exiting insert mode (unless at column 0)

                    // If we were in visual block mode, move cursor to appropriate line
                    if let Some((start_line, end_line, _col, is_append, move_to_end)) =
                        should_move_to_end_line
                    {
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
                                    .set_position(target_line, final_col);
                            }
                        } else {
                            // For insert mode, use the same column as on the first line
                            let cursor = editor.buffer().cursor();
                            let current_col = cursor.col();
                            let inserted_col = if current_col > 0 { current_col - 1 } else { 0 };
                            editor
                                .buffer_mut()
                                .cursor_mut()
                                .set_position(target_line, inserted_col);
                        }
                    } else {
                        let cursor = editor.buffer_mut().cursor_mut();
                        if cursor.col() > 0 {
                            cursor.move_left(1);
                        }
                    }
                }
            }
            // Ctrl-W - Delete word backward
            KeyCode::Char('w') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                Self::delete_word_backward_insert(editor)?;
            }
            // Ctrl-U - Delete to start of line
            KeyCode::Char('u') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                Self::delete_to_line_start_insert(editor)?;
            }
            // Ctrl-T - Indent current line in insert mode
            KeyCode::Char('t') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                Self::indent_line_insert(editor)?;
            }
            // Ctrl-D - Dedent current line in insert mode
            KeyCode::Char('d') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                Self::dedent_line_insert(editor)?;
            }
            // Ctrl-Space - Request code completion
            KeyCode::Char(' ') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.request_completion();
            }
            // Ctrl-O - Request code completion (vim omni-completion)
            KeyCode::Char('o') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.request_completion();
            }
            // Ctrl-N - Next completion item
            KeyCode::Char('n') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                if editor.completion_menu().is_visible() {
                    editor.completion_next();
                }
            }
            // Ctrl-P - Previous completion item
            KeyCode::Char('p') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                if editor.completion_menu().is_visible() {
                    editor.completion_previous();
                }
            }
            // Tab - Accept completion if menu is visible, otherwise insert tab
            KeyCode::Tab if editor.completion_menu().is_visible() => {
                editor.accept_completion();
            }
            KeyCode::Char(c) => {
                Self::insert_char(editor, c)?;
            }
            KeyCode::Enter => {
                // If completion menu is visible, accept the selected completion
                if editor.completion_menu().is_visible() {
                    editor.accept_completion();
                } else {
                    Self::insert_newline(editor)?;
                }
            }
            KeyCode::Backspace => {
                Self::delete_char_before_cursor(editor)?;
            }
            KeyCode::Left => {
                let cursor = editor.buffer_mut().cursor_mut();
                if cursor.col() > 0 {
                    cursor.move_left(1);
                }
            }
            KeyCode::Right => {
                Self::move_right(editor);
            }
            KeyCode::Up => {
                Self::move_up(editor);
            }
            KeyCode::Down => {
                Self::move_down(editor);
            }
            _ => {}
        }
        Ok(())
    }

    /// Handles input in Visual mode
}
