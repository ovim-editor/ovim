//! Case operations (toggle, upper, lower)
//!
//! Handles case transformations for characters and text ranges.

use crate::buffer::Buffer;
use crate::editor::Editor;
use crate::repeat_action::RepeatAction;
use anyhow::Result;

/// Type of case change operation
pub enum CaseChange {
    Lowercase,
    Uppercase,
    Toggle,
}

/// Toggle case of character at cursor position (~)
/// Returns true if the cursor advanced (more chars available).
pub fn toggle_case_at_cursor(editor: &mut Editor) -> Result<bool> {
    let cursor_before = editor.cursor_position();

    let (advanced, edits) = editor.buffer_mut().record(|buf| buf.toggle_char_at_cursor());

    let cursor_after = editor.cursor_position();
    editor.push_recorded_undo(edits, cursor_before, cursor_after);
    editor.set_repeat_action(RepeatAction::ToggleCase { count: 1 });

    Ok(advanced)
}

/// Changes case of entire line(s)
pub fn change_case_line(editor: &mut Editor, count: usize, case_change: CaseChange) -> Result<()> {
    let cursor_before = editor.cursor_position();
    let start_line = cursor_before.0;
    let end_line = (start_line + count).min(editor.buffer().line_count());

    let ((), edits) = editor.buffer_mut().record(|buf| {
        for line_idx in start_line..end_line {
            if let Some(line) = buf.line(line_idx) {
                let line_text = line.trim_end_matches('\n');
                let transformed = apply_case_change(line_text, &case_change);

                if transformed != line_text {
                    let line_len = line_text.chars().count();
                    buf.delete_range(line_idx, 0, line_idx, line_len);
                    buf.insert_text_at(line_idx, 0, &transformed);
                }
            }
        }
    });

    let cursor_after = editor.cursor_position();
    if !edits.is_empty() {
        editor.push_recorded_undo(edits, cursor_before, cursor_after);
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
        let ((), edits) = editor.buffer_mut().record(|buf| {
            buf.delete_range(start_line, start_col, end_line, end_col);
            buf.insert_text_at(start_line, start_col, &transformed);
        });

        editor.push_recorded_undo(edits, cursor_before, cursor_before);
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
    let cursor_before = editor.cursor_position();
    let line_idx = cursor_before.0;
    let col = cursor_before.1;

    let Some(line) = editor.buffer().line(line_idx) else {
        return Ok(());
    };
    let line_text = line.trim_end_matches('\n');
    let line_len = line_text.chars().count();

    if col >= line_len {
        return Ok(());
    }

    let text_to_end: String = line_text.chars().skip(col).collect();
    let transformed = apply_case_change(&text_to_end, &case_change);

    if transformed != text_to_end {
        let ((), edits) = editor.buffer_mut().record(|buf| {
            buf.delete_range(line_idx, col, line_idx, line_len);
            buf.insert_text_at(line_idx, col, &transformed);
        });

        let cursor_after = editor.cursor_position();
        editor.push_recorded_undo(edits, cursor_before, cursor_after);
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
