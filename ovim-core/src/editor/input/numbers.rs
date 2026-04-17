//! Number operations (Ctrl-A, Ctrl-X, g Ctrl-A, g Ctrl-X)
//!
//! Handles increment/decrement of numbers under/after cursor.
//! Supports decimal, hexadecimal (0x), binary (0b), and octal (0o) formats.

use crate::editor::{CursorPos, Editor};
use crate::number_ops::{find_number_at_or_after, format_number, parse_number};
use crate::repeat_action::RepeatAction;
use crate::unicode::CharCol;
use anyhow::Result;

/// Increments the number under/after the cursor
pub fn increment_number(editor: &mut Editor, count: usize) -> Result<()> {
    modify_number(editor, count as i64)
}

/// Decrements the number under/after the cursor
pub fn decrement_number(editor: &mut Editor, count: usize) -> Result<()> {
    modify_number(editor, -(count as i64))
}

/// Sequential modify numbers in visual selection (g Ctrl-A / g Ctrl-X)
/// delta: 1 for increment, -1 for decrement
pub fn sequential_modify_numbers(editor: &mut Editor, delta: i64) -> Result<()> {
    let selection = editor.visual_selection();
    if selection.is_none() {
        return Ok(());
    }

    let ((start_line, _), (end_line, _)) = selection.unwrap();
    let cursor_col = editor.buffer().cursor().col();
    let cursor_before = CursorPos::new(start_line, cursor_col);

    let ((), edits) = editor.buffer_mut().record(|buf| {
        for line_idx in start_line..=end_line {
            let line_offset = (line_idx - start_line) as i64;
            let total_delta = delta * line_offset;

            let Some(line) = buf.line(line_idx) else {
                continue;
            };
            let line_text = line.trim_end_matches('\n');

            let Some((start_col, end_col, number_str)) =
                find_number_at_or_after(line_text, CharCol::ZERO)
            else {
                continue;
            };
            let (value, base, prefix_len) = parse_number(&number_str);

            let new_value = value.wrapping_add(total_delta);
            let mut new_number_str = format_number(new_value, base, prefix_len);

            let has_plus_sign = number_str.starts_with('+');
            if has_plus_sign && new_value >= 0 && !new_number_str.starts_with('+') {
                new_number_str = format!("+{}", new_number_str);
            }

            buf.delete_range(line_idx, start_col, line_idx, end_col);
            buf.insert_text_at(line_idx, start_col, &new_number_str);
        }
        buf.cursor_mut().set_position(start_line, cursor_col);
    });

    let cursor_after = CursorPos::new(start_line, cursor_col);
    if !edits.is_empty() {
        editor.push_recorded_undo(edits, cursor_before, cursor_after);
        // No set_repeat_action — visual mode repeat is separate
    }

    Ok(())
}

/// Modifies (increments or decrements) the number under/after the cursor
pub fn modify_number(editor: &mut Editor, delta: i64) -> Result<()> {
    editor.record_operation(
        |buf| buf.modify_number_at_cursor(delta),
        Some(RepeatAction::NumberOperation { delta }),
    );
    Ok(())
}
