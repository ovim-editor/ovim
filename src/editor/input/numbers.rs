//! Number operations (Ctrl-A, Ctrl-X, g Ctrl-A, g Ctrl-X)
//!
//! Handles increment/decrement of numbers under/after cursor.
//! Supports decimal, hexadecimal (0x), binary (0b), and octal (0o) formats.

use crate::editor::{Change, Editor, Range};
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
    // Get visual selection range
    let selection = editor.visual_selection();
    if selection.is_none() {
        return Ok(());
    }

    let ((start_line, _), (end_line, _)) = selection.unwrap();
    let cursor_before = (start_line, editor.buffer().cursor().col());

    // Track all changes for composite undo
    let mut changes = Vec::new();

    // For each line in selection, find and modify number
    for line_idx in start_line..=end_line {
        let line_offset = (line_idx - start_line) as i64;
        let total_delta = delta * line_offset;

        if let Some(line) = editor.buffer().line(line_idx) {
            let line_text = line.trim_end_matches('\n');

            // Find number on this line (start from beginning)
            if let Some((start_col, end_col, number_str)) = find_number_at_or_after(line_text, 0) {
                // Parse the number
                if let Ok((value, base, prefix_len)) = parse_number(&number_str) {
                    // Apply the sequential delta
                    let new_value = value.wrapping_add(total_delta);

                    // Format the new number
                    let mut new_number_str = format_number(new_value, base, prefix_len);

                    // Preserve explicit '+' sign if original had it
                    let has_plus_sign = number_str.starts_with('+');
                    if has_plus_sign && new_value >= 0 && !new_number_str.starts_with('+') {
                        new_number_str = format!("+{}", new_number_str);
                    }

                    // Store the old text and range for undo
                    let old_text = number_str.clone();
                    let old_range = Range::new((line_idx, start_col), (line_idx, end_col));

                    // Delete and insert
                    let _deleted = editor
                        .buffer_mut()
                        .delete_range(line_idx, start_col, line_idx, end_col);
                    editor
                        .buffer_mut()
                        .insert_text_at(line_idx, start_col, &new_number_str);

                    // Create a NumberOperation for this line
                    let line_cursor_after = (line_idx, start_col + new_number_str.len() - 1);
                    let number_op = Change::number_operation(
                        total_delta,
                        cursor_before,
                        line_cursor_after,
                        old_text,
                        old_range,
                    );
                    changes.push(number_op);
                }
            }
        }
    }

    // Position cursor back at start of selection
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(start_line, cursor_before.1);

    // Create a composite change for all the sequential modifications
    if !changes.is_empty() {
        let cursor_after = (start_line, cursor_before.1);
        let composite = Change::composite(changes, cursor_before, cursor_after);
        editor.add_change(composite);
    }

    Ok(())
}

/// Modifies (increments or decrements) the number under/after the cursor
pub fn modify_number(editor: &mut Editor, delta: i64) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let line_idx = cursor.line();
    let col = cursor.col();

    if let Some(line) = editor.buffer().line(line_idx) {
        let line_text = line.trim_end_matches('\n');

        // Find number at or after cursor position
        if let Some((start_col, end_col, number_str)) = find_number_at_or_after(line_text, col) {
            // Check if number has explicit '+' sign
            let has_plus_sign = number_str.starts_with('+');

            // Parse the number with base detection
            let (value, base, prefix_len) = parse_number(&number_str)?;

            // Apply the delta
            let new_value = value.wrapping_add(delta);

            // Format the new number with the same base
            let mut new_number_str = format_number(new_value, base, prefix_len);

            // Preserve explicit '+' sign for positive numbers
            if has_plus_sign && new_value >= 0 && !new_number_str.starts_with('+') {
                new_number_str = format!("+{}", new_number_str);
            }

            // Replace the number in the buffer
            // Store old text before deleting for undo
            let old_text = number_str.clone();
            let old_range = Range::new((line_idx, start_col), (line_idx, end_col));

            let _deleted = editor
                .buffer_mut()
                .delete_range(line_idx, start_col, line_idx, end_col);
            editor
                .buffer_mut()
                .insert_text_at(line_idx, start_col, &new_number_str);

            // Position cursor on the last digit of the modified number
            let new_end_col = start_col + new_number_str.len() - 1;
            editor.buffer_mut().cursor_mut().set_col(new_end_col);
            let cursor_after = (line_idx, new_end_col);

            // Create a NumberOperation change for proper dot-repeat behavior
            let number_op = Change::number_operation(
                delta,
                cursor_before,
                cursor_after,
                old_text,
                old_range,
            );
            editor.add_change(number_op);
        }
    }

    Ok(())
}

/// Finds a number at or after the given column position
/// Returns (start_col, end_col, number_string)
pub fn find_number_at_or_after(line: &str, col: usize) -> Option<(usize, usize, String)> {
    let chars: Vec<char> = line.chars().collect();

    if chars.is_empty() {
        return None;
    }

    // First, check if we're currently inside a number by searching backward
    let cursor_col = col.min(chars.len().saturating_sub(1));

    // If we're on a digit, search backward to find the start of the number
    if cursor_col < chars.len() && chars[cursor_col].is_ascii_digit() {
        let mut start_col = cursor_col;

        // Search backward to find the start of the number
        while start_col > 0 {
            let prev_ch = chars[start_col - 1];
            if prev_ch.is_ascii_digit() {
                start_col -= 1;
            } else if prev_ch == '-' || prev_ch == '+' {
                // Check if this sign is part of the number
                if start_col > 1
                    && !chars[start_col - 2].is_whitespace()
                    && chars[start_col - 2] != '('
                    && chars[start_col - 2] != '['
                {
                    // Not a sign, just adjacent character
                    break;
                }
                start_col -= 1;
                break;
            } else if start_col >= 2 && prev_ch == 'x' && chars[start_col - 2] == '0' {
                // Hex prefix
                start_col -= 2;
                break;
            } else if start_col >= 2
                && (prev_ch == 'b' || prev_ch == 'o')
                && chars[start_col - 2] == '0'
            {
                // Binary or octal prefix
                start_col -= 2;
                break;
            } else {
                break;
            }
        }

        // Now find the end of the number
        let mut end_col = cursor_col + 1;
        while end_col < chars.len() && chars[end_col].is_ascii_digit() {
            end_col += 1;
        }

        let number_str: String = chars[start_col..end_col].iter().collect();
        return Some((start_col, end_col, number_str));
    }

    // Not on a digit, so search backward first, then forward
    // This matches Vim behavior: search backward on current line, then forward

    // Try searching backward from cursor
    if cursor_col > 0 {
        let mut back_col = cursor_col;
        while back_col > 0 {
            back_col -= 1;
            if chars[back_col].is_ascii_digit() {
                // Found a digit backward, now find the start and end of this number
                let mut start_col = back_col;
                while start_col > 0 {
                    let prev_ch = chars[start_col - 1];
                    if prev_ch.is_ascii_digit() {
                        start_col -= 1;
                    } else if prev_ch == '-' || prev_ch == '+' {
                        if start_col > 1
                            && !chars[start_col - 2].is_whitespace()
                            && chars[start_col - 2] != '('
                            && chars[start_col - 2] != '['
                        {
                            break;
                        }
                        start_col -= 1;
                        break;
                    } else if start_col >= 2 && prev_ch == 'x' && chars[start_col - 2] == '0' {
                        start_col -= 2;
                        break;
                    } else if start_col >= 2
                        && (prev_ch == 'b' || prev_ch == 'o')
                        && chars[start_col - 2] == '0'
                    {
                        start_col -= 2;
                        break;
                    } else {
                        break;
                    }
                }

                let mut end_col = back_col + 1;
                while end_col < chars.len() && chars[end_col].is_ascii_digit() {
                    end_col += 1;
                }

                let number_str: String = chars[start_col..end_col].iter().collect();
                return Some((start_col, end_col, number_str));
            }
        }
    }

    // No number found backward, search forward from cursor position
    let mut search_col = col;

    // Skip non-digit/non-hex characters to find start of number
    while search_col < chars.len() {
        let ch = chars[search_col];
        // Check if this could be the start of a number (including sign)
        if ch.is_ascii_digit()
            || ch == '-'
            || ch == '+'
            || (search_col + 1 < chars.len()
                && ch == '0'
                && (chars[search_col + 1] == 'x'
                    || chars[search_col + 1] == 'X'
                    || chars[search_col + 1] == 'b'
                    || chars[search_col + 1] == 'B'
                    || chars[search_col + 1] == 'o'
                    || chars[search_col + 1] == 'O'))
        {
            break;
        }
        search_col += 1;
    }

    if search_col >= chars.len() {
        return None;
    }

    let mut start_col = search_col;

    // Check if we're on a sign, and if so, verify there's a digit after it
    if chars[start_col] == '-' || chars[start_col] == '+' {
        if start_col + 1 < chars.len() && chars[start_col + 1].is_ascii_digit() {
            // Keep the sign as part of the number
        } else {
            // Not a number, just a sign
            start_col += 1;
            if start_col >= chars.len() {
                return None;
            }
        }
    }
    let mut end_col = start_col;

    // Check for hex (0x), binary (0b), or octal (0o) prefix
    if chars[end_col] == '0' && end_col + 1 < chars.len() {
        let next = chars[end_col + 1];
        if next == 'x'
            || next == 'X'
            || next == 'b'
            || next == 'B'
            || next == 'o'
            || next == 'O'
        {
            end_col += 2;

            // Collect hex/binary/octal digits
            let is_hex = next == 'x' || next == 'X';
            let is_binary = next == 'b' || next == 'B';

            while end_col < chars.len() {
                let ch = chars[end_col];
                if is_hex && ch.is_ascii_hexdigit() {
                    end_col += 1;
                } else if is_binary && (ch == '0' || ch == '1') {
                    end_col += 1;
                } else if !is_hex && !is_binary && ch.is_ascii_digit() {
                    end_col += 1;
                } else {
                    break;
                }
            }

            if end_col > start_col + 2 {
                let number_str: String = chars[start_col..end_col].iter().collect();
                return Some((start_col, end_col, number_str));
            }
        }
    }

    // Regular decimal number (may have sign)
    end_col = start_col;

    // Skip optional sign
    if end_col < chars.len() && (chars[end_col] == '-' || chars[end_col] == '+') {
        end_col += 1;
    }

    // Collect digits
    while end_col < chars.len() && chars[end_col].is_ascii_digit() {
        end_col += 1;
    }

    if end_col > start_col {
        let number_str: String = chars[start_col..end_col].iter().collect();
        Some((start_col, end_col, number_str))
    } else {
        None
    }
}

/// Parses a number string, detecting the base from prefix
/// Returns (value, base, prefix_length)
pub fn parse_number(s: &str) -> Result<(i64, u32, usize)> {
    if s.len() >= 3 {
        let prefix = &s[0..2];
        let digits = &s[2..];

        match prefix {
            "0x" | "0X" => {
                let value = i64::from_str_radix(digits, 16).unwrap_or(0);
                return Ok((value, 16, 2));
            }
            "0b" | "0B" => {
                let value = i64::from_str_radix(digits, 2).unwrap_or(0);
                return Ok((value, 2, 2));
            }
            "0o" | "0O" => {
                let value = i64::from_str_radix(digits, 8).unwrap_or(0);
                return Ok((value, 8, 2));
            }
            _ => {}
        }
    }

    // Regular decimal
    let value = s.parse::<i64>().unwrap_or(0);
    Ok((value, 10, 0))
}

/// Formats a number with the given base
pub fn format_number(value: i64, base: u32, prefix_len: usize) -> String {
    match base {
        16 => {
            if prefix_len > 0 {
                format!("0x{:x}", value)
            } else {
                format!("{:x}", value)
            }
        }
        2 => {
            if prefix_len > 0 {
                format!("0b{:b}", value)
            } else {
                format!("{:b}", value)
            }
        }
        8 => {
            if prefix_len > 0 {
                format!("0o{:o}", value)
            } else {
                format!("{:o}", value)
            }
        }
        _ => format!("{}", value),
    }
}
