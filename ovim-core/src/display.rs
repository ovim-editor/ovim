use unicode_width::UnicodeWidthChar;

/// Returns true for control characters that should be displayed as caret notation.
/// Covers 0x00–0x1F (excluding \t and \n) and 0x7F (DEL).
pub fn is_control_char(ch: char) -> bool {
    let c = ch as u32;
    (c <= 0x1F && ch != '\t' && ch != '\n') || c == 0x7F
}

/// Returns the display width of a character, accounting for control chars (2 columns),
/// wide characters (2 columns), and normal characters (1 column).
pub fn char_display_width(ch: char) -> usize {
    if is_control_char(ch) {
        2
    } else {
        ch.width().unwrap_or(1)
    }
}

/// Returns the caret notation for a control character, or None if not a control char.
/// ESC → ['^','['], NUL → ['^','@'], DEL → ['^','?'], etc.
pub fn control_char_caret(ch: char) -> Option<[char; 2]> {
    let c = ch as u32;
    if c <= 0x1F && ch != '\t' && ch != '\n' {
        // 0x00 → '@', 0x01 → 'A', ..., 0x1A → 'Z', 0x1B → '[', etc.
        Some(['^', (c as u8 + b'@') as char])
    } else if c == 0x7F {
        Some(['^', '?'])
    } else {
        None
    }
}

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
            display_col += char_display_width(ch);
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
            current_display += char_display_width(ch);
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
            width += char_display_width(ch);
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

    #[test]
    fn test_is_control_char() {
        assert!(is_control_char('\x00')); // NUL
        assert!(is_control_char('\x01')); // SOH
        assert!(is_control_char('\x1b')); // ESC
        assert!(is_control_char('\x7f')); // DEL
        assert!(!is_control_char('\t')); // tab excluded
        assert!(!is_control_char('\n')); // newline excluded
        assert!(!is_control_char('a'));
        assert!(!is_control_char(' ')); // space is 0x20, not control
    }

    #[test]
    fn test_control_char_caret() {
        assert_eq!(control_char_caret('\x00'), Some(['^', '@'])); // NUL
        assert_eq!(control_char_caret('\x01'), Some(['^', 'A'])); // SOH
        assert_eq!(control_char_caret('\x1b'), Some(['^', '['])); // ESC
        assert_eq!(control_char_caret('\x7f'), Some(['^', '?'])); // DEL
        assert_eq!(control_char_caret('\t'), None);
        assert_eq!(control_char_caret('a'), None);
    }

    #[test]
    fn test_char_display_width() {
        assert_eq!(char_display_width('\x00'), 2); // control char = 2
        assert_eq!(char_display_width('\x1b'), 2); // ESC = 2
        assert_eq!(char_display_width('a'), 1);
        assert_eq!(char_display_width('世'), 2); // wide char = 2
    }

    #[test]
    fn test_display_width_with_control_chars() {
        // "a\x1b[b" = 'a'(1) + ESC(2) + '['(1) + 'b'(1) = 5
        assert_eq!(display_width("a\x1b[b", 4), 5);
    }

    #[test]
    fn test_control_char_roundtrip() {
        let text = "a\x01b";
        // 'a'=1, '\x01'=2, 'b'=1 → total width=4
        assert_eq!(display_width(text, 4), 4);
        // char_col 0 → display 0
        assert_eq!(char_col_to_display_col(text, 0, 4), 0);
        // char_col 1 → display 1 (after 'a')
        assert_eq!(char_col_to_display_col(text, 1, 4), 1);
        // char_col 2 → display 3 (after control char width 2)
        assert_eq!(char_col_to_display_col(text, 2, 4), 3);
        // roundtrip
        assert_eq!(display_col_to_char_col(text, 0, 4), 0);
        assert_eq!(display_col_to_char_col(text, 1, 4), 1);
        assert_eq!(display_col_to_char_col(text, 2, 4), 1); // mid-control → char 1
        assert_eq!(display_col_to_char_col(text, 3, 4), 2);
    }
}
