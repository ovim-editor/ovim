//! UTF-16 position conversion utilities for LSP protocol compliance
//!
//! LSP requires character positions in UTF-16 code units, not Unicode scalar values.
//! Characters outside the BMP (emoji, CJK extensions, etc.) occupy 2 UTF-16 code units
//! but only 1 Rust char. These utilities convert between the two representations.

/// Convert a char column index to UTF-16 code units for a line of text.
///
/// Takes a `&str` representing one line (without newline) and a column
/// expressed in Unicode scalar values (chars). Returns the same column
/// expressed in UTF-16 code units, which is what LSP `Position.character`
/// expects.
pub fn char_col_to_utf16(line_text: &str, col: usize) -> u32 {
    line_text
        .chars()
        .take(col)
        .map(|c| c.len_utf16() as u32)
        .sum()
}

/// Convert UTF-16 code units (from LSP) back to a char column index.
///
/// Takes a `&str` representing one line (without newline) and a column
/// expressed in UTF-16 code units. Returns the equivalent column in
/// Unicode scalar values (chars).
pub fn utf16_to_char_col(line_text: &str, utf16_col: u32) -> usize {
    let mut utf16_offset = 0u32;
    let mut char_col = 0usize;
    for ch in line_text.chars() {
        if utf16_offset >= utf16_col {
            break;
        }
        utf16_offset += ch.len_utf16() as u32;
        char_col += 1;
    }
    char_col
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ascii_roundtrip() {
        let line = "hello world";
        assert_eq!(char_col_to_utf16(line, 0), 0);
        assert_eq!(char_col_to_utf16(line, 5), 5);
        assert_eq!(char_col_to_utf16(line, 11), 11);
        assert_eq!(utf16_to_char_col(line, 5), 5);
    }

    #[test]
    fn test_emoji_supplementary() {
        // 🦀 is U+1F980, outside BMP → 2 UTF-16 code units
        let line = "a🦀b";
        // char cols: a=0, 🦀=1, b=2
        // utf16:     a=0, 🦀=1..2, b=3
        assert_eq!(char_col_to_utf16(line, 0), 0); // before 'a'
        assert_eq!(char_col_to_utf16(line, 1), 1); // after 'a', before 🦀
        assert_eq!(char_col_to_utf16(line, 2), 3); // after 🦀, before 'b'
        assert_eq!(char_col_to_utf16(line, 3), 4); // after 'b'

        assert_eq!(utf16_to_char_col(line, 0), 0);
        assert_eq!(utf16_to_char_col(line, 1), 1);
        assert_eq!(utf16_to_char_col(line, 3), 2);
        assert_eq!(utf16_to_char_col(line, 4), 3);
    }

    #[test]
    fn test_multiple_emoji() {
        let line = "🎉🦀🌍";
        // Each emoji is 2 UTF-16 code units
        assert_eq!(char_col_to_utf16(line, 0), 0);
        assert_eq!(char_col_to_utf16(line, 1), 2);
        assert_eq!(char_col_to_utf16(line, 2), 4);
        assert_eq!(char_col_to_utf16(line, 3), 6);
    }

    #[test]
    fn test_bmp_chars() {
        // BMP characters (e.g. Chinese) are 1 UTF-16 code unit each
        let line = "你好世界";
        assert_eq!(char_col_to_utf16(line, 2), 2);
        assert_eq!(utf16_to_char_col(line, 2), 2);
    }

    #[test]
    fn test_empty_line() {
        assert_eq!(char_col_to_utf16("", 0), 0);
        assert_eq!(utf16_to_char_col("", 0), 0);
    }
}
