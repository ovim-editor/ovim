//! Replace mode input handling
//!
//! Handles character replacement, backspace (with undo), and cursor movement.

use super::helpers;
use crate::editor::{Change, Editor, Range};
use crate::mode::Mode;
use crate::repeat_action::RepeatAction;
use crate::unicode::GraphemeCol;
use crate::{KeyCode, KeyEvent};
use anyhow::Result;

/// Handles input in Replace mode
pub fn handle_replace_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        KeyCode::Esc => {
            // Save last insert position
            let cursor_line = editor.buffer().cursor().line();
            let cursor_col = editor.buffer().cursor().col();
            editor.editing.last_insert_position = Some((cursor_line, cursor_col.0));

            // Finalize replace-mode undo and semantic repeat payload.
            if let Some(state) = editor.editing.replace_mode_state.take() {
                if !state.replacements.is_empty() {
                    editor.finalize_change_building();
                    editor
                        .registers
                        .set_last_inserted(state.replacements.clone());
                    editor.set_repeat_action(RepeatAction::ReplaceMode {
                        replacements: state.replacements,
                    });
                } else {
                    // Typed/backspaced back to original: discard accumulated no-op edits.
                    editor.buffer_mut().change_manager_mut().current_builder = None;
                }
            } else {
                editor.buffer_mut().change_manager_mut().current_builder = None;
            }

            editor.mark_buffer_modified();

            // Move cursor left one position (unless at column 0)
            if cursor_col > GraphemeCol::ZERO {
                editor.buffer_mut().cursor_mut().move_left(1);
            }

            editor.set_mode(Mode::Normal);
        }
        KeyCode::Char(c) => {
            // Replace character under cursor with the typed character
            let line_idx = editor.buffer().cursor().line();
            let col = editor.buffer().cursor().col().0;

            if let Some(line) = editor.buffer().line(line_idx) {
                let line_text = line.trim_end_matches('\n');
                let chars: Vec<char> = line_text.chars().collect();

                if col < chars.len() {
                    // Track the original character for undo
                    let old_char = chars[col];
                    let cursor_before = (line_idx, col);
                    let delete_change = Change::delete(
                        Range::new((line_idx, col), (line_idx, col + 1)),
                        old_char.to_string(),
                        cursor_before,
                    );
                    if !editor.apply_change_and_record(delete_change) {
                        return Ok(());
                    }

                    let new_char = c.to_string();
                    let insert_change = Change::insert((line_idx, col), new_char, cursor_before);
                    if !editor.apply_change_and_record(insert_change) {
                        return Ok(());
                    }

                    // Track for dot-repeat
                    if let Some(ref mut state) = editor.editing.replace_mode_state {
                        state.replacements.push(c);
                        state.old_text.push(old_char);
                    }
                } else {
                    // At end of line, just insert (like append)
                    let cursor_before = (line_idx, col);
                    let insert_change =
                        Change::insert((line_idx, col), c.to_string(), cursor_before);
                    if !editor.apply_change_and_record(insert_change) {
                        return Ok(());
                    }
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
            let cursor_col = editor.buffer().cursor().col().0;
            let cursor_line = editor.buffer().cursor().line();

            if let Some(ref mut state) = editor.editing.replace_mode_state {
                let (start_line, start_col) = state.start_position;

                // Check if we're past the start position and have replacements to undo
                if cursor_line == start_line
                    && cursor_col > start_col
                    && !state.replacements.is_empty()
                {
                    // Pop the last replacement
                    let replaced_char = state.replacements.pop().unwrap();

                    // If there's an old character to restore, restore it
                    if let Some(old_char) = state.old_text.pop() {
                        let restore_col = cursor_col - 1;
                        let cursor_before = (cursor_line, restore_col);
                        let delete_change = Change::delete(
                            Range::new((cursor_line, restore_col), (cursor_line, restore_col + 1)),
                            replaced_char.to_string(),
                            cursor_before,
                        );
                        if !editor.apply_change_and_record(delete_change) {
                            return Ok(());
                        }

                        let insert_change = Change::insert(
                            (cursor_line, restore_col),
                            old_char.to_string(),
                            cursor_before,
                        );
                        if !editor.apply_change_and_record(insert_change) {
                            return Ok(());
                        }

                        // Change::insert leaves cursor after inserted char; move back one.
                        editor.buffer_mut().cursor_mut().move_left(1);
                    } else {
                        // No old_text means this was an insertion at end of line, delete it
                        let delete_col = cursor_col - 1;
                        let cursor_before = (cursor_line, delete_col);
                        let delete_change = Change::delete(
                            Range::new((cursor_line, delete_col), (cursor_line, delete_col + 1)),
                            replaced_char.to_string(),
                            cursor_before,
                        );
                        if !editor.apply_change_and_record(delete_change) {
                            return Ok(());
                        }
                    }
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
            if cursor.col() > GraphemeCol::ZERO {
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
