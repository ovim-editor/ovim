//! Replace mode input handling
//!
//! Handles character replacement, backspace (with undo), and cursor movement.

use super::helpers;
use crate::editor::Editor;
use crate::mode::Mode;
use crate::repeat_action::RepeatAction;
use crate::unicode::{CharCol, GraphemeCol};
use crate::{KeyCode, KeyEvent};
use anyhow::Result;

/// Handles input in Replace mode
pub fn handle_replace_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        KeyCode::Esc => {
            // Save last insert position (grapheme-space)
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
                    // Close the stateful recording session alongside the builder,
                    // otherwise subsequent record() calls (undo, redo) trip the
                    // nested-session assertion.
                    editor.buffer_mut().change_manager_mut().current_builder = None;
                    let _ = editor.buffer_mut().end_recording();
                }
            } else {
                editor.buffer_mut().change_manager_mut().current_builder = None;
                let _ = editor.buffer_mut().end_recording();
            }

            editor.mark_buffer_modified();

            // Move cursor left one position (unless at column 0)
            if cursor_col > GraphemeCol::ZERO {
                editor.buffer_mut().cursor_mut().move_left(1);
            }

            editor.set_mode(Mode::Normal);
        }
        KeyCode::Char(c) => {
            // Replace character under cursor with the typed character.
            // NB: This block indexes `chars` by grapheme col — correct for ASCII,
            // wrong for multi-char graphemes. That's pre-existing (Class-2 debt);
            // here we only port tuple positions to the typed CursorPos/ApplyPos.
            let line_idx = editor.buffer().cursor().line();
            let grapheme_col = editor.buffer().cursor().col();
            let col = grapheme_col.0;

            if let Some(line) = editor.buffer().line_text(line_idx) {
                let line_text = line;
                let chars: Vec<char> = line_text.chars().collect();

                if col < chars.len() {
                    // Track the original character for undo
                    let old_char = chars[col];
                    if !editor.record_session_edit(|buf| {
                        buf.delete_range_positioning_cursor(
                            line_idx,
                            CharCol(col),
                            line_idx,
                            CharCol(col + 1),
                        )
                        .0
                    }) {
                        return Ok(());
                    }

                    let new_char = c.to_string();
                    if !editor.record_session_edit(|buf| {
                        buf.insert_text_at_positioning_cursor(line_idx, CharCol(col), &new_char)
                    }) {
                        return Ok(());
                    }

                    // Track for dot-repeat
                    if let Some(ref mut state) = editor.editing.replace_mode_state {
                        state.replacements.push(c);
                        state.old_text.push(old_char);
                    }
                } else {
                    // At end of line, just insert (like append)
                    let new_char = c.to_string();
                    if !editor.record_session_edit(|buf| {
                        buf.insert_text_at_positioning_cursor(line_idx, CharCol(col), &new_char)
                    }) {
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
            // and move cursor left, but only within the current replace session.
            // NB: This block, like Char above, treats grapheme col as char col.
            // Pre-existing Class-2 debt; here we only port to typed positions.
            let cursor_col = editor.buffer().cursor().col().0;
            let cursor_line = editor.buffer().cursor().line();

            if let Some(ref mut state) = editor.editing.replace_mode_state {
                let start_line = state.start_position.line;
                let start_col = state.start_position.col.0;

                // Check if we're past the start position and have replacements to undo
                if cursor_line == start_line
                    && cursor_col > start_col
                    && !state.replacements.is_empty()
                {
                    // Shrink the replacements stack to match old_text. The
                    // char value isn't needed — buffer.record() captures the
                    // deleted text.
                    state.replacements.pop();

                    // If there's an old character to restore, restore it
                    if let Some(old_char) = state.old_text.pop() {
                        let restore_col = cursor_col - 1;
                        if !editor.record_session_edit(|buf| {
                            buf.delete_range_positioning_cursor(
                                cursor_line,
                                CharCol(restore_col),
                                cursor_line,
                                CharCol(restore_col + 1),
                            )
                            .0
                        }) {
                            return Ok(());
                        }

                        let old_char_str = old_char.to_string();
                        if !editor.record_session_edit(|buf| {
                            buf.insert_text_at_positioning_cursor(
                                cursor_line,
                                CharCol(restore_col),
                                &old_char_str,
                            )
                        }) {
                            return Ok(());
                        }

                        // insert_text_at_positioning_cursor leaves cursor after the
                        // inserted char; move back one to sit on the restored char.
                        editor.buffer_mut().cursor_mut().move_left(1);
                    } else {
                        // No old_text means this was an insertion at end of line, delete it
                        let delete_col = cursor_col - 1;
                        if !editor.record_session_edit(|buf| {
                            buf.delete_range_positioning_cursor(
                                cursor_line,
                                CharCol(delete_col),
                                cursor_line,
                                CharCol(delete_col + 1),
                            )
                            .0
                        }) {
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
