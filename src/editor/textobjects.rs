use crate::buffer::Buffer;
use anyhow::Result;

/// Represents a text object selection range
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextObjectRange {
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

/// Handles text object operations
pub struct TextObjects;

impl TextObjects {
    /// Gets the range for "inner word" (iw)
    pub fn inner_word(buffer: &Buffer) -> Option<TextObjectRange> {
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        if line_idx >= buffer.line_count() {
            return None;
        }

        let line = buffer.rope().line(line_idx).to_string();
        let line_text = line.trim_end_matches('\n');
        let chars: Vec<char> = line_text.chars().collect();

        if col >= chars.len() {
            return None;
        }

        // Find word boundaries
        let is_word_char = |c: char| c.is_alphanumeric() || c == '_';

        // If cursor is on whitespace, return None (iw doesn't select whitespace)
        if !is_word_char(chars[col]) {
            return None;
        }

        // Find start of word
        let mut start_col = col;
        while start_col > 0 && is_word_char(chars[start_col - 1]) {
            start_col -= 1;
        }

        // Find end of word
        let mut end_col = col;
        while end_col < chars.len() && is_word_char(chars[end_col]) {
            end_col += 1;
        }

        Some(TextObjectRange {
            start_line: line_idx,
            start_col,
            end_line: line_idx,
            end_col: end_col.saturating_sub(1),
        })
    }

    /// Gets the range for "around word" (aw)
    pub fn around_word(buffer: &Buffer) -> Option<TextObjectRange> {
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        if line_idx >= buffer.line_count() {
            return None;
        }

        let line = buffer.rope().line(line_idx).to_string();
        let line_text = line.trim_end_matches('\n');
        let chars: Vec<char> = line_text.chars().collect();

        if col >= chars.len() {
            return None;
        }

        let is_word_char = |c: char| c.is_alphanumeric() || c == '_';

        // If cursor is on whitespace, select the whitespace
        if !is_word_char(chars[col]) {
            let mut start_col = col;
            while start_col > 0 && !is_word_char(chars[start_col - 1]) {
                start_col -= 1;
            }

            let mut end_col = col;
            while end_col < chars.len() && !is_word_char(chars[end_col]) {
                end_col += 1;
            }

            return Some(TextObjectRange {
                start_line: line_idx,
                start_col,
                end_line: line_idx,
                end_col: end_col.saturating_sub(1),
            });
        }

        // Find start of word
        let mut start_col = col;
        while start_col > 0 && is_word_char(chars[start_col - 1]) {
            start_col -= 1;
        }

        // Find end of word
        let mut end_col = col;
        while end_col < chars.len() && is_word_char(chars[end_col]) {
            end_col += 1;
        }

        // Include trailing whitespace
        while end_col < chars.len() && chars[end_col].is_whitespace() {
            end_col += 1;
        }

        // If no trailing whitespace, include leading whitespace
        if end_col == col + 1 || (end_col < chars.len() && is_word_char(chars[end_col - 1])) {
            while start_col > 0 && chars[start_col - 1].is_whitespace() {
                start_col -= 1;
            }
        }

        Some(TextObjectRange {
            start_line: line_idx,
            start_col,
            end_line: line_idx,
            end_col: end_col.saturating_sub(1),
        })
    }

    /// Gets the range for a quoted string (inner or around)
    /// quote_char: the quote character (', ", `)
    /// include_quotes: true for "around", false for "inner"
    pub fn quoted_string(
        buffer: &Buffer,
        quote_char: char,
        include_quotes: bool,
    ) -> Option<TextObjectRange> {
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        if line_idx >= buffer.line_count() {
            return None;
        }

        let line = buffer.rope().line(line_idx).to_string();
        let line_text = line.trim_end_matches('\n');
        let chars: Vec<char> = line_text.chars().collect();

        if chars.is_empty() {
            return None;
        }

        // Find the opening quote before or at cursor
        let mut start_col = None;
        for i in (0..=col.min(chars.len().saturating_sub(1))).rev() {
            if chars[i] == quote_char {
                start_col = Some(i);
                break;
            }
        }

        let start_col = start_col?;

        // Find the closing quote after the opening quote
        let mut end_col = None;
        for i in (start_col + 1)..chars.len() {
            if chars[i] == quote_char {
                end_col = Some(i);
                break;
            }
        }

        let end_col = end_col?;

        if include_quotes {
            // "around" - include the quotes
            Some(TextObjectRange {
                start_line: line_idx,
                start_col,
                end_line: line_idx,
                end_col,
            })
        } else {
            // "inner" - exclude the quotes
            if start_col + 1 >= end_col {
                return None; // Empty string
            }
            Some(TextObjectRange {
                start_line: line_idx,
                start_col: start_col + 1,
                end_line: line_idx,
                end_col: end_col - 1,
            })
        }
    }

    /// Gets the range for parentheses, brackets, or braces
    /// open_char: '(', '[', or '{'
    /// close_char: ')', ']', or '}'
    /// include_delimiters: true for "around", false for "inner"
    pub fn paired_delimiters(
        buffer: &Buffer,
        open_char: char,
        close_char: char,
        include_delimiters: bool,
    ) -> Option<TextObjectRange> {
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        if line_idx >= buffer.line_count() {
            return None;
        }

        let line = buffer.rope().line(line_idx).to_string();
        let line_text = line.trim_end_matches('\n');
        let chars: Vec<char> = line_text.chars().collect();

        if chars.is_empty() {
            return None;
        }

        // Find opening delimiter (search backward from cursor)
        let mut start_col = None;
        let mut depth = 0;
        for i in (0..=col.min(chars.len().saturating_sub(1))).rev() {
            if chars[i] == close_char {
                depth += 1;
            } else if chars[i] == open_char {
                if depth == 0 {
                    start_col = Some(i);
                    break;
                }
                depth -= 1;
            }
        }

        let start_col = start_col?;

        // Find closing delimiter (search forward from opening delimiter)
        let mut end_col = None;
        let mut depth = 0;
        for i in (start_col + 1)..chars.len() {
            if chars[i] == open_char {
                depth += 1;
            } else if chars[i] == close_char {
                if depth == 0 {
                    end_col = Some(i);
                    break;
                }
                depth -= 1;
            }
        }

        let end_col = end_col?;

        if include_delimiters {
            // "around" - include the delimiters
            Some(TextObjectRange {
                start_line: line_idx,
                start_col,
                end_line: line_idx,
                end_col,
            })
        } else {
            // "inner" - exclude the delimiters
            if start_col + 1 >= end_col {
                return None; // Empty content
            }
            Some(TextObjectRange {
                start_line: line_idx,
                start_col: start_col + 1,
                end_line: line_idx,
                end_col: end_col - 1,
            })
        }
    }

    /// Deletes text within a text object range
    pub fn delete_range(buffer: &mut Buffer, range: TextObjectRange) -> Result<String> {
        let start_char = buffer.rope().line_to_char(range.start_line) + range.start_col;
        let end_char = buffer.rope().line_to_char(range.end_line) + range.end_col + 1;

        let deleted = buffer.rope().slice(start_char..end_char).to_string();
        buffer.rope_mut().remove(start_char..end_char);

        // Position cursor at start of deleted range
        buffer.cursor_mut().set_position(range.start_line, range.start_col);

        Ok(deleted)
    }

    /// Yanks text within a text object range
    pub fn yank_range(buffer: &Buffer, range: TextObjectRange) -> Result<String> {
        let start_char = buffer.rope().line_to_char(range.start_line) + range.start_col;
        let end_char = buffer.rope().line_to_char(range.end_line) + range.end_col + 1;

        Ok(buffer.rope().slice(start_char..end_char).to_string())
    }
}
