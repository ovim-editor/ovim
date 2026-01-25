//! Direct editing commands in normal mode.
//!
//! These are commands that directly edit text without requiring an operator+motion.
//! Includes: x, X, D, C, s, S, p, P, Y, J, ~, u, Ctrl-R, .

use crate::editor::input::helpers;
use crate::editor::{Change, Editor, Range, RegisterType};
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::case;

/// Try to handle an editing command.
///
/// Returns `Ok(true)` if the key was handled, `Ok(false)` otherwise.
pub fn try_handle(editor: &mut Editor, key_event: KeyEvent) -> Result<bool> {
    match key_event.code {
        // x - delete character under cursor (but not Ctrl+X which is decrement)
        KeyCode::Char('x') if !key_event.modifiers.contains(KeyModifiers::CONTROL) => {
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
            helpers::paste_after(editor)?;
            editor.clear_count();
            Ok(true)
        }
        // P - paste before cursor
        KeyCode::Char('P') => {
            helpers::paste_before(editor)?;
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
        KeyCode::Char('u') if !key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            editor.undo();
            editor.clear_count();
            Ok(true)
        }
        // Ctrl-R - redo
        KeyCode::Char('r') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
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
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let line_idx = cursor.line();
    let col = cursor.col();

    if let Some(line) = editor.buffer().line(line_idx) {
        let line_text = line.trim_end_matches('\n');
        let chars_count = line_text.chars().count();

        if col < chars_count {
            let end_col = (col + count).min(chars_count);
            let start_pos = (line_idx, col);
            let end_pos = (line_idx, end_col);

            let deleted = editor
                .buffer_mut()
                .delete_range(line_idx, col, line_idx, end_col);
            let range = Range::new(start_pos, end_pos);
            let change = Change::delete(range, deleted.clone(), cursor_before);

            editor.delete_to_register(deleted);
            editor.add_change(change);

            helpers::clamp_cursor_to_buffer(editor);
        }
    }
    editor.clear_count();
    Ok(())
}

/// X - delete character(s) before cursor
fn delete_char_backward(editor: &mut Editor) -> Result<()> {
    let count = editor.effective_count();
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let line_idx = cursor.line();
    let col = cursor.col();

    if col > 0 {
        if let Some(line) = editor.buffer().line(line_idx) {
            let _line_text = line.trim_end_matches('\n');

            let start_col = col.saturating_sub(count);
            let start_pos = (line_idx, start_col);
            let end_pos = (line_idx, col);

            let deleted = editor
                .buffer_mut()
                .delete_range(line_idx, start_col, line_idx, col);
            let range = Range::new(start_pos, end_pos);
            let change = Change::delete(range, deleted.clone(), cursor_before);

            editor.delete_to_register(deleted);
            editor.add_change(change);

            editor.buffer_mut().cursor_mut().set_col(start_col);
            helpers::clamp_cursor_to_buffer(editor);
        }
    }
    editor.clear_count();
    Ok(())
}

/// D - delete to end of line
fn delete_to_end_of_line(editor: &mut Editor) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let line_idx = cursor.line();
    let col = cursor.col();

    if let Some(line) = editor.buffer().line(line_idx) {
        let line_text = line.trim_end_matches('\n');
        let line_len = line_text.chars().count();

        if col < line_len {
            let start_pos = (line_idx, col);
            let end_pos = (line_idx, line_len);

            let deleted = editor
                .buffer_mut()
                .delete_range(line_idx, col, line_idx, line_len);
            let range = Range::new(start_pos, end_pos);
            let change = Change::delete(range, deleted.clone(), cursor_before);

            editor.delete_to_register(deleted);
            editor.add_change(change);

            helpers::clamp_cursor_to_buffer(editor);
        }
    }
    editor.clear_count();
    Ok(())
}

/// C - change to end of line
fn change_to_end_of_line(editor: &mut Editor) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let line_idx = cursor.line();
    let col = cursor.col();

    if let Some(line) = editor.buffer().line(line_idx) {
        let line_text = line.trim_end_matches('\n');
        let line_len = line_text.chars().count();

        if col < line_len {
            let start_pos = (line_idx, col);
            let end_pos = (line_idx, line_len);

            let deleted = editor
                .buffer_mut()
                .delete_range(line_idx, col, line_idx, line_len);
            let range = Range::new(start_pos, end_pos);
            let change = Change::delete(range, deleted.clone(), cursor_before);

            editor.delete_to_register(deleted);
            editor.add_change(change);

            editor.clear_count();
            let insert_cursor = (
                editor.buffer().cursor().line(),
                editor.buffer().cursor().col(),
            );
            editor.start_change_building(insert_cursor);
            editor.set_mode(Mode::Insert);
            return Ok(());
        }
    }
    editor.clear_count();
    let cursor_before = (
        editor.buffer().cursor().line(),
        editor.buffer().cursor().col(),
    );
    editor.start_change_building(cursor_before);
    editor.set_mode(Mode::Insert);
    Ok(())
}

/// s - substitute character(s) under cursor
fn substitute_chars(editor: &mut Editor) -> Result<()> {
    let count = editor.effective_count();
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let line_idx = cursor.line();
    let col = cursor.col();

    if let Some(line) = editor.buffer().line(line_idx) {
        let line_text = line.trim_end_matches('\n');
        let chars_count = line_text.chars().count();

        if col < chars_count {
            let end_col = (col + count).min(chars_count);
            let start_pos = (line_idx, col);
            let end_pos = (line_idx, end_col);

            let deleted = editor
                .buffer_mut()
                .delete_range(line_idx, col, line_idx, end_col);
            let range = Range::new(start_pos, end_pos);
            let change = Change::delete(range, deleted.clone(), cursor_before);

            editor.delete_to_register(deleted);
            editor.add_change(change);

            helpers::clamp_cursor_to_buffer(editor);
        }
    }
    editor.clear_count();
    let cursor_before = (
        editor.buffer().cursor().line(),
        editor.buffer().cursor().col(),
    );
    editor.start_change_building(cursor_before);
    editor.set_mode(Mode::Insert);
    Ok(())
}

/// S - substitute entire line
fn substitute_line(editor: &mut Editor) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let start_line = cursor.line();
    let count = editor.effective_count();
    let end_line = (start_line + count).min(editor.buffer().line_count());

    let start_pos = (start_line, 0);
    let end_pos = (end_line, 0);

    let deleted = editor.buffer_mut().delete_range(start_line, 0, end_line, 0);
    let range = Range::new(start_pos, end_pos);
    let change = Change::delete(range, deleted.clone(), cursor_before);

    editor.delete_to_register(deleted);
    editor.add_change(change);
    editor.clear_count();

    let cursor_before = (
        editor.buffer().cursor().line(),
        editor.buffer().cursor().col(),
    );
    editor.start_change_building(cursor_before);
    editor.set_mode(Mode::Insert);
    helpers::insert_line_above(editor)?;
    Ok(())
}

/// Y - yank line
fn yank_line(editor: &mut Editor) -> Result<()> {
    let count = editor.effective_count();
    let yanked = helpers::yank_line(editor.buffer(), count)?;
    editor.yank_to_register_with_type(yanked, RegisterType::Line);
    editor.clear_count();
    Ok(())
}

/// ~ - toggle case of character(s) under cursor
fn toggle_case(editor: &mut Editor) -> Result<()> {
    let count = editor.effective_count();
    for _ in 0..count {
        case::toggle_case_at_cursor(editor)?;
    }
    editor.clear_count();
    Ok(())
}
