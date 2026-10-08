//! Direct editing commands in normal mode.
//!
//! These are commands that directly edit text without requiring an operator+motion.
//! Includes: x, X, D, C, s, S, p, P, Y, J, ~, u, Ctrl-R, .

use crate::editor::input::helpers;
use crate::editor::{Editor, RegisterType};
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
        // <Del> is x -- but while a count is being typed it removes the last digit
        KeyCode::Delete => {
            match editor.count() {
                Some(count) if count >= 10 => editor.set_count(count / 10),
                Some(_) => editor.clear_count(),
                None => delete_char_forward(editor)?,
            }
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
            if editor.closed_fold_at_cursor().is_some() {
                // A linewise paste lands below a closed fold, not inside it.
                // (Reading the register consumes the pending `"x` prefix.)
                let pending = editor.pending_register();
                let linewise = editor.get_from_register_with_type().1 == RegisterType::Line;
                if let Some(register) = pending {
                    editor.set_pending_register(register);
                }
                if linewise {
                    editor.cursor_to_closed_fold_end();
                }
            }
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
            editor.undo_count(editor.effective_count());
            editor.clear_count();
            Ok(true)
        }
        // Ctrl-R - redo
        KeyCode::Char('r') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.redo_count(editor.effective_count());
            editor.clear_count();
            Ok(true)
        }
        // . - repeat last change
        KeyCode::Char('.') => {
            editor.repeat_last_change_with_count(editor.count());
            editor.clear_count();
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// x - delete character(s) under cursor
fn delete_char_forward(editor: &mut Editor) -> Result<()> {
    let count = editor.effective_count();
    if editor.closed_fold_at_cursor().is_some() {
        // `x` is `dl`: on a closed fold it deletes the whole fold (Vim).
        let lines = editor.linewise_count_over_folds(1);
        let deleted = editor.record_operation(
            |buf| buf.delete_lines(lines),
            Some(RepeatAction::DeleteLines { count: lines }),
        );
        if !deleted.is_empty() {
            editor.delete_to_register_with_type(deleted, RegisterType::Line);
        }
        editor.clear_count();
        return Ok(());
    }
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
    if editor.closed_fold_at_cursor().is_some() {
        let lines = editor.linewise_count_over_folds(1);
        let deleted = editor.record_operation(
            |buf| buf.delete_lines(lines),
            Some(RepeatAction::DeleteLines { count: lines }),
        );
        if !deleted.is_empty() {
            editor.delete_to_register_with_type(deleted, RegisterType::Line);
        }
        editor.clear_count();
        return Ok(());
    }
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
pub(super) fn change_to_end_of_line(editor: &mut Editor) -> Result<()> {
    if editor.closed_fold_at_cursor().is_some() {
        return substitute_line(editor);
    }
    super::operators::change_with(editor, RepeatAction::DeleteToEndOfLine, |buf| {
        let line = buf.cursor().line();
        let col = buf.cursor().col();
        let deleted = buf.delete_to_end_of_line();
        buf.cursor_mut().set_position(line, col);
        deleted
    })
}

/// s - substitute character(s) under cursor
fn substitute_chars(editor: &mut Editor) -> Result<()> {
    if editor.closed_fold_at_cursor().is_some() {
        return substitute_line(editor);
    }
    let count = editor.effective_count();
    super::operators::change_with(editor, RepeatAction::DeleteCharForward { count }, |buf| {
        buf.change_chars_forward(count)
    })
}

/// S - substitute entire line
fn substitute_line(editor: &mut Editor) -> Result<()> {
    if helpers::linewise_count_fails(editor, editor.effective_count()) {
        editor.clear_count();
        return Ok(());
    }
    let count = editor.linewise_count_over_folds(editor.effective_count());
    let start = editor.buffer().cursor().line();
    let end = (start + count).min(editor.buffer().line_count());
    super::operators::change_lines(editor, start, end, RepeatAction::DeleteLines { count })
}

/// Y - yank line
fn yank_line(editor: &mut Editor) -> Result<()> {
    if helpers::linewise_count_fails(editor, editor.effective_count()) {
        editor.clear_count();
        return Ok(());
    }
    let count = editor.linewise_count_over_folds(editor.effective_count());
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
