//! Line positioning motions: ^, g_, +, -, _

use super::Motions;
use crate::buffer::Buffer;
use crate::unicode::GraphemeCol;

impl Motions {
    /// Move to first non-blank character on line (^ motion)
    pub fn first_non_blank(buffer: &mut Buffer) {
        let cursor = buffer.cursor();
        let line_idx = cursor.line();

        if let Some(line) = buffer.line(line_idx) {
            let line_text = line.trim_end_matches('\n');

            // Find first non-whitespace character (char index → grapheme)
            let char_col = line_text
                .chars()
                .position(|c| !c.is_whitespace())
                .unwrap_or(0);
            let grapheme_col =
                crate::unicode::char_to_grapheme_col(line_text, crate::unicode::CharCol(char_col));

            buffer.cursor_mut().set_col(grapheme_col);
        }
    }

    /// Move to first non-blank character on line (_ motion, same as ^)
    pub fn first_non_blank_underscore(buffer: &mut Buffer) {
        Self::first_non_blank(buffer);
    }

    /// Move to first non-blank of next line (+ motion)
    pub fn plus_motion(buffer: &mut Buffer, count: usize) {
        let cursor = buffer.cursor();
        let current_line = cursor.line();
        let target_line = (current_line + count).min(buffer.line_count().saturating_sub(1));

        buffer
            .cursor_mut()
            .set_position(target_line, GraphemeCol::ZERO);
        Self::first_non_blank(buffer);
    }

    /// Move to first non-blank of previous line (- motion)
    pub fn minus_motion(buffer: &mut Buffer, count: usize) {
        let cursor = buffer.cursor();
        let current_line = cursor.line();
        let target_line = current_line.saturating_sub(count);

        buffer
            .cursor_mut()
            .set_position(target_line, GraphemeCol::ZERO);
        Self::first_non_blank(buffer);
    }

    /// Move to last non-blank character on line (g_ motion)
    pub fn last_non_blank(buffer: &mut Buffer) {
        let cursor = buffer.cursor();
        let line_idx = cursor.line();

        if let Some(line) = buffer.line(line_idx) {
            let line_text = line.trim_end_matches('\n');

            // Find last non-whitespace character (char index → grapheme)
            let mut last_char_col = 0;
            for (i, c) in line_text.chars().enumerate() {
                if !c.is_whitespace() {
                    last_char_col = i;
                }
            }
            let grapheme_col = crate::unicode::char_to_grapheme_col(
                line_text,
                crate::unicode::CharCol(last_char_col),
            );

            buffer.cursor_mut().set_col(grapheme_col);
        }
    }
}
