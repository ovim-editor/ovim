//! Sentence motions: (, )

use super::Motions;
use crate::buffer::Buffer;
use crate::unicode::GraphemeCol;

impl Motions {
    /// Move forward to start of next sentence (( and ) motions)
    pub fn sentence_forward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::sentence_forward_once(buffer);
        }
    }

    fn sentence_forward_once(buffer: &mut Buffer) {
        // TODO (Bug 4): Sentence motion doesn't handle abbreviations like "Dr.", "e.g.", "i.e."
        // Vim's sentence motion has some heuristics for this (e.g., two spaces after period)
        // but implementing full abbreviation support would require a dictionary or more
        // sophisticated pattern matching. Low priority since basic sentence navigation works.

        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let grapheme_col = cursor.col();
        let total_lines = buffer.line_count();

        // Convert grapheme col to char col for char-based iteration
        let char_col = if let Some(line) = buffer.line_text(line_idx) {
            let line_text = line;
            crate::unicode::grapheme_to_char_col(&line_text, grapheme_col).0
        } else {
            0
        };

        // Get text from current position onwards
        let mut current_line = line_idx;
        let mut current_col = char_col + 1;

        // Look for sentence-ending punctuation (.!?) followed by space/newline
        while current_line < total_lines {
            if let Some(line) = buffer.line_text(current_line) {
                let chars: Vec<char> = line.chars().collect();

                while current_col < chars.len() {
                    let ch = chars[current_col];
                    if ch == '.' || ch == '!' || ch == '?' {
                        // Check if followed by space or at end of line
                        if current_col + 1 >= chars.len() || chars[current_col + 1].is_whitespace()
                        {
                            // Skip whitespace after punctuation
                            current_col += 1;
                            while current_col < chars.len() && chars[current_col].is_whitespace() {
                                current_col += 1;
                            }

                            if current_col >= chars.len() {
                                // Move to next line
                                if current_line + 1 < total_lines {
                                    buffer
                                        .cursor_mut()
                                        .set_position(current_line + 1, GraphemeCol::ZERO);
                                } else {
                                    let line_str: String = line.to_string();
                                    let end_grapheme = crate::unicode::char_to_grapheme_col(
                                        &line_str,
                                        crate::unicode::CharCol(
                                            chars.len().saturating_sub(1).max(0),
                                        ),
                                    );
                                    buffer.cursor_mut().set_position(current_line, end_grapheme);
                                }
                            } else {
                                let line_str: String = line.to_string();
                                let grapheme = crate::unicode::char_to_grapheme_col(
                                    &line_str,
                                    crate::unicode::CharCol(current_col),
                                );
                                buffer.cursor_mut().set_position(current_line, grapheme);
                            }
                            return;
                        }
                    }
                    current_col += 1;
                }
            }

            current_line += 1;
            current_col = 0;
        }

        // No sentence found, move to end of buffer
        let last_line = buffer.line_count().saturating_sub(1);
        buffer
            .cursor_mut()
            .set_position(last_line, GraphemeCol::ZERO);
    }

    /// Move backward to start of previous sentence
    pub fn sentence_backward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::sentence_backward_once(buffer);
        }
    }

    fn sentence_backward_once(buffer: &mut Buffer) {
        let cursor = buffer.cursor();
        let mut line_idx = cursor.line();
        let grapheme_col = cursor.col();

        // Convert grapheme to char col for char-based iteration
        let mut col = if let Some(line) = buffer.line_text(line_idx) {
            let line_text = line;
            crate::unicode::grapheme_to_char_col(&line_text, grapheme_col).0
        } else {
            0
        };

        if col == 0 && line_idx == 0 {
            return;
        }

        // Move back one position
        if col > 0 {
            col -= 1;
        } else if line_idx > 0 {
            line_idx -= 1;
            if let Some(line) = buffer.line_text(line_idx) {
                col = line.chars().count().saturating_sub(1);
            }
        }

        // Look for sentence-ending punctuation (.!?) followed by space/newline
        loop {
            if let Some(line) = buffer.line_text(line_idx) {
                let chars: Vec<char> = line.chars().collect();

                if chars.is_empty() {
                    // Empty line — skip to previous line
                    if line_idx == 0 {
                        buffer.cursor_mut().set_position(0, GraphemeCol::ZERO);
                        return;
                    }
                    line_idx -= 1;
                    if let Some(prev_line) = buffer.line_text(line_idx) {
                        col = prev_line.chars().count().saturating_sub(1);
                    }
                    continue;
                }

                // Clamp col to valid range (cursor may exceed line length
                // e.g. after $ then k to a shorter line)
                col = col.min(chars.len().saturating_sub(1));

                while col > 0 {
                    let ch = chars[col];
                    if ch == '.' || ch == '!' || ch == '?' {
                        // Found sentence end, move past it
                        col += 1;
                        // Skip whitespace
                        while col < chars.len() && chars[col].is_whitespace() {
                            col += 1;
                        }

                        if col >= chars.len() && line_idx + 1 < buffer.line_count() {
                            buffer
                                .cursor_mut()
                                .set_position(line_idx + 1, GraphemeCol::ZERO);
                        } else {
                            let clamped = col.min(chars.len().saturating_sub(1));
                            let line_str: String = line.to_string();
                            buffer.cursor_mut().set_position(
                                line_idx,
                                crate::unicode::char_to_grapheme_col(
                                    &line_str,
                                    crate::unicode::CharCol(clamped),
                                ),
                            );
                        }
                        return;
                    }
                    col = col.saturating_sub(1);
                }
            }

            if line_idx == 0 {
                buffer.cursor_mut().set_position(0, GraphemeCol::ZERO);
                return;
            }

            line_idx -= 1;
            if let Some(line) = buffer.line_text(line_idx) {
                col = line.chars().count().saturating_sub(1);
            }
        }
    }
}
