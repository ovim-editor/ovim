use unicode_width::UnicodeWidthChar;

/// Converts a character column index to a display column, accounting for tabs and wide characters.
pub fn char_col_to_display_col(text: &str, char_col: usize, tab_width: usize) -> usize {
    let mut display_col = 0;

    for (current_char_idx, ch) in text.chars().enumerate() {
        if current_char_idx >= char_col {
            break;
        }

        if ch == '\t' {
            let spaces_to_add = tab_width - (display_col % tab_width);
            display_col += spaces_to_add;
        } else {
            display_col += ch.width().unwrap_or(1);
        }
    }

    display_col
}

/// Converts a display column to a character column index, accounting for tabs and wide characters.
/// If the display column falls in the middle of a wide character, returns the char index of that character.
pub fn display_col_to_char_col(text: &str, display_col: usize, tab_width: usize) -> usize {
    let mut current_display = 0;

    for (char_idx, ch) in text.chars().enumerate() {
        if current_display >= display_col {
            return char_idx;
        }

        if ch == '\t' {
            let spaces = tab_width - (current_display % tab_width);
            current_display += spaces;
        } else {
            current_display += ch.width().unwrap_or(1);
        }

        if current_display > display_col {
            return char_idx;
        }
    }

    // display_col is beyond end of text — return char count
    text.chars().count()
}

/// Calculates the display width of a string, accounting for tabs and wide characters.
pub fn display_width(text: &str, tab_width: usize) -> usize {
    let mut width = 0;
    for ch in text.chars() {
        if ch == '\t' {
            width += tab_width - (width % tab_width);
        } else {
            width += ch.width().unwrap_or(1);
        }
    }
    width
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ascii_roundtrip() {
        let text = "hello world";
        for i in 0..text.len() {
            let display = char_col_to_display_col(text, i, 4);
            let back = display_col_to_char_col(text, display, 4);
            assert_eq!(back, i, "roundtrip failed for char_col {i}");
        }
    }

    #[test]
    fn test_tab_handling() {
        let text = "\thello";
        // char_col 0 = display_col 0 (before the tab)
        assert_eq!(char_col_to_display_col(text, 0, 4), 0);
        // char_col 1 = display_col 4 (after the tab)
        assert_eq!(char_col_to_display_col(text, 1, 4), 4);
        // inverse: display_col 4 = char_col 1
        assert_eq!(display_col_to_char_col(text, 4, 4), 1);
        // display_col in the middle of a tab (e.g., 2) = char_col 0 (the tab itself)
        assert_eq!(display_col_to_char_col(text, 2, 4), 0);
    }

    #[test]
    fn test_wide_char() {
        let text = "a世b";
        // 'a' = width 1, '世' = width 2, 'b' = width 1
        assert_eq!(char_col_to_display_col(text, 0, 4), 0);
        assert_eq!(char_col_to_display_col(text, 1, 4), 1); // after 'a'
        assert_eq!(char_col_to_display_col(text, 2, 4), 3); // after '世'
                                                            // inverse
        assert_eq!(display_col_to_char_col(text, 0, 4), 0);
        assert_eq!(display_col_to_char_col(text, 1, 4), 1); // start of '世'
        assert_eq!(display_col_to_char_col(text, 2, 4), 1); // middle of '世' -> returns char 1
        assert_eq!(display_col_to_char_col(text, 3, 4), 2); // 'b'
    }

    #[test]
    fn test_display_width_with_wide_chars() {
        assert_eq!(display_width("a世b", 4), 4); // 1 + 2 + 1
        assert_eq!(display_width("hello", 4), 5);
        assert_eq!(display_width("\thello", 4), 9); // 4 + 5
        assert_eq!(display_width("", 4), 0);
    }

    #[test]
    fn test_display_col_beyond_text() {
        let text = "abc";
        assert_eq!(display_col_to_char_col(text, 10, 4), 3);
    }
}
