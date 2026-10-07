//! Number operations (Ctrl-A, Ctrl-X, g Ctrl-A, g Ctrl-X)
//!
//! Handles increment/decrement of numbers under/after cursor.
//! Supports decimal, hexadecimal (0x), binary (0b), and octal (0o) formats.

use crate::editor::{CursorPos, Editor};
use crate::mode::Mode;
use crate::number_ops::{find_number_at_or_after, format_number, parse_number};
use crate::repeat_action::RepeatAction;
use crate::unicode::{grapheme_count, grapheme_to_char_col, CharCol, GraphemeCol};
use anyhow::Result;

/// Increments the number under/after the cursor
pub fn increment_number(editor: &mut Editor, count: usize) -> Result<()> {
    modify_number(editor, count as i64)
}

/// Decrements the number under/after the cursor
pub fn decrement_number(editor: &mut Editor, count: usize) -> Result<()> {
    modify_number(editor, -(count as i64))
}

/// Adds `delta` to the first number inside the Visual selection on each line
/// (`<C-a>` / `<C-x>`). With `sequential` (`g<C-a>` / `g<C-x>`) the n-th line
/// that has a number gets `n * delta` instead. Leaves Visual mode with the
/// cursor on the start of the selection.
pub fn modify_numbers_in_selection(
    editor: &mut Editor,
    delta: i64,
    sequential: bool,
) -> Result<()> {
    let mode = editor.mode();
    let Some(((start_line, start_col), (end_line, end_col))) = editor.visual_selection() else {
        return Ok(());
    };
    let cursor_before = editor.cursor_position();
    let cursor_after = CursorPos::new(start_line, GraphemeCol(start_col));

    let ((), edits) = editor.buffer_mut().record(|buf| {
        let mut changed_lines: i64 = 0;
        for line_idx in start_line..=end_line {
            let Some(line_text) = buf.line_text(line_idx).map(|text| text.into_owned()) else {
                continue;
            };
            // The columns of this line the selection covers, as chars.
            let char_col = |grapheme: usize| {
                grapheme_to_char_col(
                    &line_text,
                    GraphemeCol(grapheme.min(grapheme_count(&line_text))),
                )
            };
            let (from, to) = match mode {
                Mode::VisualLine => (CharCol::ZERO, usize::MAX),
                Mode::VisualBlock => (char_col(start_col), char_col(end_col.saturating_add(1)).0),
                _ => (
                    if line_idx == start_line {
                        char_col(start_col)
                    } else {
                        CharCol::ZERO
                    },
                    if line_idx == end_line {
                        char_col(end_col.saturating_add(1)).0
                    } else {
                        usize::MAX
                    },
                ),
            };

            let Some((number_start, number_end, number_str)) =
                find_number_at_or_after(&line_text, from)
            else {
                continue;
            };
            if number_start.0 >= to {
                continue;
            }
            changed_lines += 1;
            let step = if sequential {
                delta * changed_lines
            } else {
                delta
            };
            let (value, base, prefix_len) = parse_number(&number_str);

            let new_value = value.wrapping_add(step);
            let mut new_number_str = format_number(new_value, base, prefix_len);

            let has_plus_sign = number_str.starts_with('+');
            if has_plus_sign && new_value >= 0 && !new_number_str.starts_with('+') {
                new_number_str = format!("+{}", new_number_str);
            }

            buf.delete_range(line_idx, number_start, line_idx, number_end);
            buf.insert_text_at(line_idx, number_start, &new_number_str);
        }
        buf.cursor_mut()
            .set_position(cursor_after.line, cursor_after.col);
    });

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
