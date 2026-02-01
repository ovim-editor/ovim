use crate::display::{char_display_width, control_char_caret};
use std::ops::Range;
use unicode_width::UnicodeWidthChar;

/// Expands tabs and control characters for rendering.
/// Returns:
/// - The expanded string (tabs → spaces, control chars → caret notation)
/// - Byte mapping from original byte offsets to expanded byte offsets
/// - Control char ranges in the expanded string (for special styling)
pub fn expand_tabs_with_mapping(
    text: &str,
    tab_width: usize,
) -> (String, Vec<(usize, usize)>, Vec<Range<usize>>) {
    let mut result = String::with_capacity(text.len() * 2);
    let mut display_col = 0;
    let mut byte_mapping = Vec::new(); // original_byte_idx -> expanded_byte_idx
    let mut control_ranges = Vec::new();

    let mut expanded_byte_pos = 0;

    for (orig_byte_idx, ch) in text.char_indices() {
        // Record mapping from original position to expanded position
        byte_mapping.push((orig_byte_idx, expanded_byte_pos));

        if ch == '\t' {
            // Calculate spaces needed to reach next tab stop
            let spaces_to_add = tab_width - (display_col % tab_width);
            result.push_str(&" ".repeat(spaces_to_add));
            expanded_byte_pos += spaces_to_add;
            display_col += spaces_to_add;
        } else if let Some(caret) = control_char_caret(ch) {
            let start = expanded_byte_pos;
            result.push(caret[0]); // '^'
            result.push(caret[1]); // notation char
            expanded_byte_pos += 2;
            display_col += 2;
            control_ranges.push(start..expanded_byte_pos);
        } else {
            result.push(ch);
            expanded_byte_pos += ch.len_utf8();
            // Use display width (emojis = 2, most chars = 1, zero-width = 0)
            display_col += ch.width().unwrap_or(1);
        }
    }

    // Add final mapping for end position
    byte_mapping.push((text.len(), expanded_byte_pos));

    (result, byte_mapping, control_ranges)
}

/// Expands tabs and control characters (simple version without mapping)
pub fn expand_tabs(text: &str, tab_width: usize) -> String {
    expand_tabs_with_mapping(text, tab_width).0
}

/// Converts a character column index to a display column, accounting for tabs and wide characters
pub fn char_col_to_display_col(text: &str, char_col: usize, tab_width: usize) -> usize {
    crate::display::char_col_to_display_col(text, char_col, tab_width)
}

/// Truncates text to fit within a display width, accounting for wide characters
pub fn truncate_to_width(text: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut display_width = 0;

    for ch in text.chars() {
        let ch_width = char_display_width(ch);
        if display_width + ch_width > max_width {
            break;
        }
        result.push(ch);
        display_width += ch_width;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_tabs_basic() {
        let (text, _, control_ranges) = expand_tabs_with_mapping("\thello", 4);
        assert_eq!(text, "    hello");
        assert!(control_ranges.is_empty());
    }

    #[test]
    fn test_expand_control_chars() {
        // ESC (0x1B) → ^[
        let (text, _, control_ranges) = expand_tabs_with_mapping("\x1b[31m", 4);
        assert_eq!(text, "^[[31m");
        assert_eq!(control_ranges.len(), 1);
        assert_eq!(control_ranges[0], 0..2); // ^[ occupies bytes 0..2
    }

    #[test]
    fn test_expand_nul() {
        let (text, _, control_ranges) = expand_tabs_with_mapping("a\x00b", 4);
        assert_eq!(text, "a^@b");
        assert_eq!(control_ranges.len(), 1);
        assert_eq!(control_ranges[0], 1..3); // ^@ at byte offset 1..3
    }

    #[test]
    fn test_expand_del() {
        let (text, _, control_ranges) = expand_tabs_with_mapping("x\x7fy", 4);
        assert_eq!(text, "x^?y");
        assert_eq!(control_ranges.len(), 1);
        assert_eq!(control_ranges[0], 1..3);
    }

    #[test]
    fn test_expand_multiple_control_chars() {
        // "\x1b[31mred\x1b[0m" → "^[[31mred^[[0m"
        let (text, _, control_ranges) = expand_tabs_with_mapping("\x1b[31mred\x1b[0m", 4);
        assert_eq!(text, "^[[31mred^[[0m");
        assert_eq!(control_ranges.len(), 2);
        assert_eq!(control_ranges[0], 0..2);   // first ^[
        assert_eq!(control_ranges[1], 9..11);  // second ^[
    }

    #[test]
    fn test_expand_no_control_chars() {
        let (text, _, control_ranges) = expand_tabs_with_mapping("hello world", 4);
        assert_eq!(text, "hello world");
        assert!(control_ranges.is_empty());
    }

    #[test]
    fn test_byte_mapping_with_control_chars() {
        // "a\x01b" → "a^Ab"
        let (text, mapping, _) = expand_tabs_with_mapping("a\x01b", 4);
        assert_eq!(text, "a^Ab");
        // mapping: orig 0 → exp 0 ('a'), orig 1 → exp 1 ('\x01' → ^A), orig 2 → exp 3 ('b'), end
        assert_eq!(mapping[0], (0, 0));
        assert_eq!(mapping[1], (1, 1));
        assert_eq!(mapping[2], (2, 3));
        assert_eq!(mapping[3], (3, 4)); // end sentinel
    }
}
