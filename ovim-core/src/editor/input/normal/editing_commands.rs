//! Direct editing commands in normal mode.
//!
//! These are commands that directly edit text without requiring an operator+motion.
//! Includes: x, X, D, C, s, S, p, P, Y, J, ~, u, Ctrl-R, .

use crate::editor::input::helpers;
use crate::editor::{Editor, PendingChangeRepeat, RegisterType};
use crate::mode::Mode;
use crate::repeat_action::RepeatAction;
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;

use super::super::case;

/// Try to handle an editing command.
///
/// Returns `Ok(true)` if the key was handled, `Ok(false)` otherwise.
pub fn try_handle(editor: &mut Editor, key_event: KeyEvent) -> Result<bool> {
    match key_event.code {
        // x - delete character under cursor (but not Ctrl+X which is decrement)
        KeyCode::Char('x') if !key_event.modifiers.contains(Modifiers::CONTROL) => {
            delete_char_forward(editor)?;
            Ok(true)
        }
        // X - delete character before cursor
        KeyCode::Char('X') => {
            delete_char_backward(editor)?;
            Ok(true)
        }
        // D - delete to end of line
        KeyCode::Char('D') => {
            delete_to_end_of_line(editor)?;
            Ok(true)
        }
        // C - change to end of line
        KeyCode::Char('C') => {
            change_to_end_of_line(editor)?;
            Ok(true)
        }
        // s - substitute character(s)
        KeyCode::Char('s') => {
            substitute_chars(editor)?;
            Ok(true)
        }
        // S - substitute entire line
        KeyCode::Char('S') => {
            substitute_line(editor)?;
            Ok(true)
        }
        // p - paste after cursor
        KeyCode::Char('p') => {
            let count = editor.effective_count();
            helpers::paste_after(editor, count)?;
            editor.clear_count();
            Ok(true)
        }
        // P - paste before cursor
        KeyCode::Char('P') => {
            let count = editor.effective_count();
            helpers::paste_before(editor, count)?;
            editor.clear_count();
            Ok(true)
        }
        // Y - yank line
        KeyCode::Char('Y') => {
            yank_line(editor)?;
            Ok(true)
        }
        // J - join lines
        KeyCode::Char('J') => {
            let count = editor.effective_count();
            helpers::join_lines(editor, count)?;
            editor.clear_count();
            Ok(true)
        }
        // ~ - toggle case
        KeyCode::Char('~') => {
            toggle_case(editor)?;
            Ok(true)
        }
        // u - undo (but not Ctrl+U which is scroll up)
        KeyCode::Char('u') if !key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.undo();
            editor.clear_count();
            Ok(true)
        }
        // Ctrl-R - redo
        KeyCode::Char('r') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.redo();
            editor.clear_count();
            Ok(true)
        }
        // . - repeat last change
        KeyCode::Char('.') => {
            editor.repeat_last_change();
            editor.clear_count();
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// x - delete character(s) under cursor
fn delete_char_forward(editor: &mut Editor) -> Result<()> {
    let count = editor.effective_count();
    let deleted = editor.record_operation(
        |buf| buf.delete_chars_forward(count),
        Some(RepeatAction::DeleteCharForward { count }),
    );
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
    }
    editor.clear_count();
    Ok(())
}

/// X - delete character(s) before cursor
fn delete_char_backward(editor: &mut Editor) -> Result<()> {
    let count = editor.effective_count();
    let deleted = editor.record_operation(
        |buf| buf.delete_chars_backward(count),
        Some(RepeatAction::DeleteCharBackward { count }),
    );
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
    }
    editor.clear_count();
    Ok(())
}

/// D - delete to end of line
fn delete_to_end_of_line(editor: &mut Editor) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_to_end_of_line(),
        Some(RepeatAction::DeleteToEndOfLine),
    );
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
    }
    editor.clear_count();
    Ok(())
}

/// C - change to end of line
fn change_to_end_of_line(editor: &mut Editor) -> Result<()> {
    let cursor_before = editor.cursor_position();
    let line_idx = cursor_before.0;
    let col = cursor_before.1;

    let (deleted, edits) = editor.buffer_mut().record(|buf| {
        let line_len = buf
            .line(line_idx)
            .map(|l| l.trim_end_matches('\n').chars().count())
            .unwrap_or(0);
        if col < line_len {
            let deleted = buf.delete_range(line_idx, col, line_idx, line_len);
            // Keep cursor at col (insert position) — don't clamp to normal mode
            buf.cursor_mut().set_position(line_idx, col);
            deleted
        } else {
            String::new()
        }
    });
    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        let token = editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after);
        editor.delete_to_register(deleted);
        editor.mark_buffer_modified();
        Some(token)
    } else {
        None
    };

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteToEndOfLine,
        linewise: false,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}

/// s - substitute character(s) under cursor
fn substitute_chars(editor: &mut Editor) -> Result<()> {
    let count = editor.effective_count();
    let cursor_before = editor.cursor_position();

    let (deleted, edits) = editor
        .buffer_mut()
        .record(|buf| buf.delete_chars_forward(count));
    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        let token = editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after);
        editor.delete_to_register(deleted);
        editor.mark_buffer_modified();
        Some(token)
    } else {
        None
    };

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteCharForward { count },
        linewise: false,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}

/// S - substitute entire line
fn substitute_line(editor: &mut Editor) -> Result<()> {
    let cursor_before = editor.cursor_position();
    let start_line = editor.buffer().cursor().line();
    let count = editor.effective_count();
    let end_line = (start_line + count).min(editor.buffer().line_count());

    // Get indentation from the current line before deleting
    let indent = if let Some(line) = editor.buffer().line(start_line) {
        let trimmed = line.trim_start_matches([' ', '\t']);
        line[..line.len() - trimmed.len()].to_string()
    } else {
        String::new()
    };

    // Record all edits atomically (delete + indent insert = single undo)
    let (deleted_text, edits) = editor.buffer_mut().record(|buf| {
        if count == 1 {
            // Single line: clear content but preserve the line itself
            let deleted = if let Some(line) = buf.line(start_line) {
                let content_len = line.trim_end_matches('\n').chars().count();
                if content_len > 0 {
                    buf.delete_range(start_line, 0, start_line, content_len)
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            // Insert indentation
            if !indent.is_empty() {
                buf.insert_text_at(start_line, 0, &indent);
            }

            deleted
        } else {
            // Multi-line: delete all lines, insert a blank line with indent
            let deleted = buf.delete_range(start_line, 0, end_line, 0);

            let new_line_text = format!("{}\n", indent);
            buf.insert_text_at(start_line, 0, &new_line_text);

            deleted
        }
    });

    // Store deleted text in register
    if !deleted_text.is_empty() {
        editor.delete_to_register_with_type(deleted_text, RegisterType::Line);
    }

    // Position cursor at end of indentation
    let indent_len = indent.chars().count();
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(start_line, indent_len);

    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        Some(editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after))
    } else {
        None
    };

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteLines { count },
        linewise: true,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}

/// Y - yank line
fn yank_line(editor: &mut Editor) -> Result<()> {
    let count = editor.effective_count();
    let start_line = editor.buffer().cursor().line();
    let end_line = (start_line + count).min(editor.buffer().line_count()) - 1;
    let yanked = helpers::yank_line(editor.buffer(), count)?;
    editor.yank_to_register_with_type(yanked, RegisterType::Line);
    editor.set_yank_flash_lines(start_line, end_line);
    editor.clear_count();
    Ok(())
}

/// ~ - toggle case of character(s) under cursor
fn toggle_case(editor: &mut Editor) -> Result<()> {
    let count = editor.effective_count();
    for _ in 0..count {
        let advanced = case::toggle_case_at_cursor(editor)?;
        if !advanced {
            break; // At end of line — stop, don't re-toggle same char
        }
    }
    // Set repeat action with the full count (overrides per-char set_repeat_action)
    editor.set_repeat_action(RepeatAction::ToggleCase { count });
    editor.clear_count();
    Ok(())
}
