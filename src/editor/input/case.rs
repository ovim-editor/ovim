//! Case operations (toggle, upper, lower)
//!
//! Handles case transformations for characters and text ranges.

use crate::buffer::Buffer;
use crate::editor::{Change, Editor, Range};
use anyhow::Result;

/// Type of case change operation
pub enum CaseChange {
    Lowercase,
    Uppercase,
    Toggle,
}

/// Toggle case of character at cursor position (~)
pub fn toggle_case_at_cursor(editor: &mut Editor) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let line_idx = cursor.line();
    let col = cursor.col();

    if let Some(line) = editor.buffer().line(line_idx) {
        let line_text = line.trim_end_matches('\n');
        let chars: Vec<char> = line_text.chars().collect();

        if col < chars.len() {
            let ch = chars[col];
            let toggled = if ch.is_lowercase() {
                ch.to_uppercase().to_string()
            } else {
                ch.to_lowercase().to_string()
            };

            // Delete the character
            let start_pos = (line_idx, col);
            let end_pos = (line_idx, col + 1);
            let deleted = editor
                .buffer_mut()
                .delete_range(line_idx, col, line_idx, col + 1);
            let range = Range::new(start_pos, end_pos);
            let delete_change = Change::delete(range, deleted, cursor_before);

            // Insert the toggled character
            let insert_change = Change::insert((line_idx, col), toggled.clone(), cursor_before);
            insert_change.apply(editor.buffer_mut());

            editor.add_change(delete_change);
            editor.add_change(insert_change);

            // Move cursor right (Vim behavior)
            let new_col = col + toggled.chars().count();
            if new_col < chars.len() {
                editor.buffer_mut().cursor_mut().set_col(new_col);
            }
        }
    }

    Ok(())
}

/// Changes case of entire line(s)
pub fn change_case_line(editor: &mut Editor, count: usize, case_change: CaseChange) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let start_line = cursor.line();
    let end_line = (start_line + count).min(editor.buffer().line_count());

    for line_idx in start_line..end_line {
        if let Some(line) = editor.buffer().line(line_idx) {
            let line_text = line.trim_end_matches('\n');
            let transformed = apply_case_change(line_text, &case_change);

            if transformed != line_text {
                let line_len = line_text.chars().count();
                let deleted = editor
                    .buffer_mut()
                    .delete_range(line_idx, 0, line_idx, line_len);
                let delete_range = Range::new((line_idx, 0), (line_idx, line_len));
                let delete_change = Change::delete(delete_range, deleted, cursor_before);

                let insert_change = Change::insert((line_idx, 0), transformed, cursor_before);
                insert_change.apply(editor.buffer_mut());

                editor.add_change(delete_change);
                editor.add_change(insert_change);
            }
        }
    }

    Ok(())
}

/// Changes case using a motion
pub fn change_case_motion<F>(
    editor: &mut Editor,
    count: usize,
    case_change: CaseChange,
    motion: F,
) -> Result<()>
where
    F: FnOnce(&mut Buffer, usize),
{
    let start_cursor = *editor.buffer().cursor();
    let cursor_before = (start_cursor.line(), start_cursor.col());
    let start_line = start_cursor.line();
    let start_col = start_cursor.col();

    // Apply the motion to find the end position
    motion(editor.buffer_mut(), count);

    let end_cursor = editor.buffer().cursor();
    let end_line = end_cursor.line();
    let end_col = end_cursor.col();

    // Get the text in the range
    let start_char = editor.buffer().rope().line_to_char(start_line) + start_col;
    let end_char = editor.buffer().rope().line_to_char(end_line) + end_col;
    let text = editor
        .buffer()
        .rope()
        .slice(start_char..end_char)
        .to_string();

    // Transform the case
    let transformed = apply_case_change(&text, &case_change);

    if transformed != text {
        // Delete the old text
        let deleted = editor
            .buffer_mut()
            .delete_range(start_line, start_col, end_line, end_col);
        let delete_range = Range::new((start_line, start_col), (end_line, end_col));
        let delete_change = Change::delete(delete_range, deleted, cursor_before);

        // Insert the transformed text
        let insert_change = Change::insert((start_line, start_col), transformed, cursor_before);
        insert_change.apply(editor.buffer_mut());

        editor.add_change(delete_change);
        editor.add_change(insert_change);
    }

    // Reset cursor to start position
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(start_line, start_col);

    Ok(())
}

/// Changes case from cursor to end of line
pub fn change_case_to_end_of_line(editor: &mut Editor, case_change: CaseChange) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let line_idx = cursor.line();
    let col = cursor.col();

    if let Some(line) = editor.buffer().line(line_idx) {
        let line_text = line.trim_end_matches('\n');
        let line_len = line_text.chars().count();

        if col < line_len {
            let text_to_end: String = line_text.chars().skip(col).collect();
            let transformed = apply_case_change(&text_to_end, &case_change);

            if transformed != text_to_end {
                let deleted = editor
                    .buffer_mut()
                    .delete_range(line_idx, col, line_idx, line_len);
                let delete_range = Range::new((line_idx, col), (line_idx, line_len));
                let delete_change = Change::delete(delete_range, deleted, cursor_before);

                let insert_change = Change::insert((line_idx, col), transformed, cursor_before);
                insert_change.apply(editor.buffer_mut());

                editor.add_change(delete_change);
                editor.add_change(insert_change);
            }
        }
    }

    Ok(())
}

/// Applies case change transformation to a string
pub fn apply_case_change(text: &str, case_change: &CaseChange) -> String {
    match case_change {
        CaseChange::Lowercase => text.to_lowercase(),
        CaseChange::Uppercase => text.to_uppercase(),
        CaseChange::Toggle => text
            .chars()
            .map(|ch| {
                if ch.is_lowercase() {
                    ch.to_uppercase().to_string()
                } else {
                    ch.to_lowercase().to_string()
                }
            })
            .collect(),
    }
}
