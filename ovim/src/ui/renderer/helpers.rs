use unicode_width::UnicodeWidthChar;

/// Expands tabs to spaces based on display width (accounts for wide chars like emojis)
/// Returns both the expanded string and a mapping from original byte offsets to expanded byte offsets
pub fn expand_tabs_with_mapping(text: &str, tab_width: usize) -> (String, Vec<(usize, usize)>) {
    let mut result = String::with_capacity(text.len() * 2);
    let mut display_col = 0;
    let mut byte_mapping = Vec::new(); // original_byte_idx -> expanded_byte_idx

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
        } else {
            result.push(ch);
            expanded_byte_pos += ch.len_utf8();
            // Use display width (emojis = 2, most chars = 1, zero-width = 0)
            display_col += ch.width().unwrap_or(1);
        }
    }

    // Add final mapping for end position
    byte_mapping.push((text.len(), expanded_byte_pos));

    (result, byte_mapping)
}

/// Expands tabs to spaces (simple version without mapping)
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
        let ch_width = ch.width().unwrap_or(1);
        if display_width + ch_width > max_width {
            break;
        }
        result.push(ch);
        display_width += ch_width;
    }

    result
}
