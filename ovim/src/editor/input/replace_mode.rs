//! Replace mode input handling
//!
//! Handles character replacement, backspace (with undo), and cursor movement.

use super::helpers;
use crate::editor::{Change, Editor, Range};
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

/// Handles input in Replace mode
pub fn handle_replace_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        KeyCode::Esc => {
            // Save last insert position
            let cursor_line = editor.buffer().cursor().line();
            let cursor_col = editor.buffer().cursor().col();
            editor.editing.last_insert_position = Some((cursor_line, cursor_col));

            // Create ReplaceMode change for dot-repeat
            if let Some(state) = editor.editing.replace_mode_state.take() {
                if !state.replacements.is_empty() {
                    let cursor_after = (cursor_line, cursor_col);
                    let replacement_len = state.replacements.chars().count();
                    let old_range = Range::new(
                        state.start_position,
                        (
                            state.start_position.0,
                            state.start_position.1 + replacement_len,
                        ),
                    );
                    let change = Change::replace_mode(
                        state.replacements,
                        state.start_position,
                        cursor_after,
                        state.old_text,
                        old_range,
                    );
                    editor.add_change(change);
                }
            }

            editor.update_last_inserted_register();
            editor.mark_buffer_modified();

            // Move cursor left one position (unless at column 0)
            if cursor_col > 0 {
                editor.buffer_mut().cursor_mut().move_left(1);
            }

            editor.set_mode(Mode::Normal);
        }
        KeyCode::Char(c) => {
            // Replace character under cursor with the typed character
            let line_idx = editor.buffer().cursor().line();
            let col = editor.buffer().cursor().col();

            if let Some(line) = editor.buffer().line(line_idx) {
                let line_text = line.trim_end_matches('\n');
                let chars: Vec<char> = line_text.chars().collect();

                if col < chars.len() {
                    // Track the original character for undo
                    let old_char = chars[col];

                    // Delete character under cursor
                    editor
                        .buffer_mut()
                        .delete_range(line_idx, col, line_idx, col + 1);

                    // Insert new character
                    let new_char = c.to_string();
                    editor.buffer_mut().insert_text_at(line_idx, col, &new_char);

                    // Track for dot-repeat
                    if let Some(ref mut state) = editor.editing.replace_mode_state {
                        state.replacements.push(c);
                        state.old_text.push(old_char);
                    }

                    // Move cursor forward
                    editor.buffer_mut().cursor_mut().move_right(1);
                } else {
                    // At end of line, just insert (like append)
                    helpers::insert_char(editor, c)?;
                    // Also track for dot-repeat
                    if let Some(ref mut state) = editor.editing.replace_mode_state {
                        state.replacements.push(c);
                    }
                }
            }
        }
        KeyCode::Enter => {
            // In replace mode, Enter inserts a newline (breaking the line)
            helpers::insert_newline(editor)?;
        }
        KeyCode::Backspace => {
            // Backspace in replace mode should restore original characters
            // and move cursor left, but only within the current replace session
            let cursor_col = editor.buffer().cursor().col();
            let cursor_line = editor.buffer().cursor().line();

            if let Some(ref mut state) = editor.editing.replace_mode_state {
                let (start_line, start_col) = state.start_position;

                // Check if we're past the start position and have replacements to undo
                if cursor_line == start_line
                    && cursor_col > start_col
                    && !state.replacements.is_empty()
                {
                    // Pop the last replacement
                    state.replacements.pop();

                    // If there's an old character to restore, restore it
                    if let Some(old_char) = state.old_text.pop() {
                        let restore_col = cursor_col - 1;
                        // Delete the current character at restore position
                        editor.buffer_mut().delete_range(
                            cursor_line,
                            restore_col,
                            cursor_line,
                            restore_col + 1,
                        );
                        // Insert the original character
                        editor.buffer_mut().insert_text_at(
                            cursor_line,
                            restore_col,
                            &old_char.to_string(),
                        );
                    } else {
                        // No old_text means this was an insertion at end of line, delete it
                        let delete_col = cursor_col - 1;
                        editor.buffer_mut().delete_range(
                            cursor_line,
                            delete_col,
                            cursor_line,
                            delete_col + 1,
                        );
                    }

                    // Move cursor left
                    editor.buffer_mut().cursor_mut().move_left(1);
                } else if cursor_col > 0 {
                    // We're before the start position or no replacements, just move left
                    editor.buffer_mut().cursor_mut().move_left(1);
                }
            } else if cursor_col > 0 {
                // No replace mode state, just move left
                editor.buffer_mut().cursor_mut().move_left(1);
            }
        }
        KeyCode::Left => {
            let cursor = editor.buffer_mut().cursor_mut();
            if cursor.col() > 0 {
                cursor.move_left(1);
            }
        }
        KeyCode::Right => {
            helpers::move_right(editor);
        }
        KeyCode::Up => {
            helpers::move_up(editor);
        }
        KeyCode::Down => {
            helpers::move_down(editor);
        }
        _ => {}
    }
    Ok(())
}
