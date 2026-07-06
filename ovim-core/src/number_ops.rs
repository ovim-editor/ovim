//! Number parsing/formatting helpers for the Ctrl-A / Ctrl-X increment commands.
//!
//! These operate on plain `&str` line content and don't touch the buffer,
//! cursor, or undo machinery. They live in their own module so that
//! `change.rs` can focus entirely on the undo/repeat record shape.

use crate::unicode::CharCol;

/// Finds a number at or after the given column position.
/// Returns (start_col, end_col, number_string) as `CharCol` indices.
/// Handles cursor on hex digits (a-f) inside a 0x prefix number.
pub fn find_number_at_or_after(line: &str, col: CharCol) -> Option<(CharCol, CharCol, String)> {
    let chars: Vec<char> = line.chars().collect();

    if chars.is_empty() {
        return None;
    }

    // First, check if we're currently inside a number by searching backward.
    // Internal arithmetic uses raw usize; we wrap at the return boundary.
    let col = col.0;
    let cursor_col = col.min(chars.len().saturating_sub(1));

    // Check if we're on a digit or hex digit that's part of a hex number
    let on_digit = cursor_col < chars.len() && chars[cursor_col].is_ascii_digit();
    let on_hex_digit = cursor_col < chars.len()
        && chars[cursor_col].is_ascii_hexdigit()
        && !chars[cursor_col].is_ascii_digit();

    // If we're on a hex digit (a-f/A-F), check if we're inside a hex number
    let in_hex_number = if on_hex_digit {
        let mut check = cursor_col;
        let mut found_hex = false;
        while check > 0 {
            let prev = chars[check - 1];
            if prev.is_ascii_hexdigit() || prev.is_ascii_digit() {
                check -= 1;
            } else if check >= 2 && (prev == 'x' || prev == 'X') && chars[check - 2] == '0' {
                found_hex = true;
                break;
            } else {
                break;
            }
        }
        found_hex
    } else {
        false
    };

    // If we're on a digit (or hex digit within a hex number), search backward to find the start
    if on_digit || in_hex_number {
        let mut start_col = cursor_col;

        while start_col > 0 {
            let prev_ch = chars[start_col - 1];
            if prev_ch.is_ascii_digit() {
                start_col -= 1;
            } else if in_hex_number && prev_ch.is_ascii_hexdigit() {
                // Only allow hex digits if we're in a hex number context
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
            } else if start_col >= 2
                && matches!(prev_ch, 'x' | 'X' | 'b' | 'B' | 'o' | 'O')
                && chars[start_col - 2] == '0'
            {
                start_col -= 2;
                break;
            } else {
                break;
            }
        }

        // Detect a based literal (0x/0b/0o) at the number start so the cursor
        // sitting on the leading '0' (or the prefix letter) still yields the
        // whole literal rather than just "0". Without this, the end-scan starts
        // at cursor+1 (the prefix letter), which fails the digit test and
        // truncates "0xff" to "0".
        let prefix = if start_col + 1 < chars.len() && chars[start_col] == '0' {
            match chars[start_col + 1] {
                'x' | 'X' => Some((true, false, false)),
                'b' | 'B' => Some((false, true, false)),
                'o' | 'O' => Some((false, false, true)),
                _ => None,
            }
        } else {
            None
        };

        let (mut end_col, is_hex, is_binary, is_octal) = match prefix {
            // Start scanning just past the two-char prefix, validating per base.
            Some((h, b, o)) => (start_col + 2, h, b, o),
            None => (cursor_col + 1, false, false, false),
        };
        while end_col < chars.len() {
            let ch = chars[end_col];
            let valid = if is_hex {
                ch.is_ascii_hexdigit()
            } else if is_binary {
                ch == '0' || ch == '1'
            } else if is_octal {
                ch.is_ascii_digit()
            } else {
                ch.is_ascii_digit()
            };
            if valid {
                end_col += 1;
            } else {
                break;
            }
        }

        // A prefix with no following digits (e.g. a bare "0x") isn't a real
        // based literal — fall back to treating the leading '0' as decimal.
        if prefix.is_some() && end_col == start_col + 2 {
            let number_str: String = chars[start_col..=start_col].iter().collect();
            return Some((CharCol(start_col), CharCol(start_col + 1), number_str));
        }

        let number_str: String = chars[start_col..end_col].iter().collect();
        return Some((CharCol(start_col), CharCol(end_col), number_str));
    }

    // Not on a digit — search forward only (matches Vim behavior)
    let mut search_col = col;

    while search_col < chars.len() {
        let ch = chars[search_col];
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
        if next == 'x' || next == 'X' || next == 'b' || next == 'B' || next == 'o' || next == 'O' {
            end_col += 2;

            let is_hex = next == 'x' || next == 'X';
            let is_binary = next == 'b' || next == 'B';

            while end_col < chars.len() {
                let ch = chars[end_col];
                let valid_digit = (is_hex && ch.is_ascii_hexdigit())
                    || (is_binary && (ch == '0' || ch == '1'))
                    || (!is_hex && !is_binary && ch.is_ascii_digit());
                if valid_digit {
                    end_col += 1;
                } else {
                    break;
                }
            }

            if end_col > start_col + 2 {
                let number_str: String = chars[start_col..end_col].iter().collect();
                return Some((CharCol(start_col), CharCol(end_col), number_str));
            }
        }
    }

    // Regular decimal number (may have sign)
    end_col = start_col;

    if end_col < chars.len() && (chars[end_col] == '-' || chars[end_col] == '+') {
        end_col += 1;
    }

    while end_col < chars.len() && chars[end_col].is_ascii_digit() {
        end_col += 1;
    }

    if end_col > start_col {
        let number_str: String = chars[start_col..end_col].iter().collect();
        Some((CharCol(start_col), CharCol(end_col), number_str))
    } else {
        None
    }
}

/// Parses a number string, detecting the base from prefix.
/// Returns (value, base, prefix_length).
///
/// Malformed digits fall back to `0`. Callers are expected to pass strings
/// already vetted by `find_number_at_or_after`, so in practice the fallback
/// only protects against i64 overflow (e.g. a 20-digit decimal).
pub fn parse_number(s: &str) -> (i64, u32, usize) {
    if s.len() >= 3 {
        let prefix = &s[0..2];
        let digits = &s[2..];

        match prefix {
            "0x" | "0X" => {
                let value = i64::from_str_radix(digits, 16).unwrap_or(0);
                return (value, 16, 2);
            }
            "0b" | "0B" => {
                let value = i64::from_str_radix(digits, 2).unwrap_or(0);
                return (value, 2, 2);
            }
            "0o" | "0O" => {
                let value = i64::from_str_radix(digits, 8).unwrap_or(0);
                return (value, 8, 2);
            }
            _ => {}
        }
    }

    // Regular decimal
    let value = s.parse::<i64>().unwrap_or(0);
    (value, 10, 0)
}

/// Formats a number with the given base.
/// Handles negative hex/bin/oct via sign + unsigned abs.
pub fn format_number(value: i64, base: u32, prefix_len: usize) -> String {
    match base {
        16 => {
            let abs = value.unsigned_abs();
            let sign = if value < 0 { "-" } else { "" };
            if prefix_len > 0 {
                format!("{sign}0x{abs:x}")
            } else {
                format!("{sign}{abs:x}")
            }
        }
        2 => {
            let abs = value.unsigned_abs();
            let sign = if value < 0 { "-" } else { "" };
            if prefix_len > 0 {
                format!("{sign}0b{abs:b}")
            } else {
                format!("{sign}{abs:b}")
            }
        }
        8 => {
            let abs = value.unsigned_abs();
            let sign = if value < 0 { "-" } else { "" };
            if prefix_len > 0 {
                format!("{sign}0o{abs:o}")
            } else {
                format!("{sign}{abs:o}")
            }
        }
        _ => format!("{}", value),
    }
}
