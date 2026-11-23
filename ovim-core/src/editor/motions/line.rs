//! Line positioning motions: ^, g_, +, -, _

use super::Motions;
use crate::buffer::Buffer;
use crate::unicode::{CharCol, GraphemeCol};

impl Motions {
    /// Move to first non-blank character on line (^ motion).
    pub fn first_non_blank(buffer: &mut Buffer) {
        let line_idx = buffer.cursor().line();
        let index = buffer.line_index(line_idx);
        let col = buffer
            .rope()
            .line(line_idx)
            .chars()
            .take(index.len_chars())
            .position(|character| !character.is_whitespace())
            .map(|char_col| index.char_to_grapheme(CharCol(char_col)).0)
            .unwrap_or(0);
        buffer.cursor_mut().set_col(GraphemeCol(col));
    }

    /// Move to first non-blank character on line (_ motion, same as ^).
    pub fn first_non_blank_underscore(buffer: &mut Buffer) {
        Self::first_non_blank(buffer);
    }

    /// Move to first non-blank of next line (+ motion).
    pub fn plus_motion(buffer: &mut Buffer, count: usize) {
        let current_line = buffer.cursor().line();
        let target_line = (current_line + count).min(buffer.line_count().saturating_sub(1));
        buffer
            .cursor_mut()
            .set_position(target_line, GraphemeCol::ZERO);
        Self::first_non_blank(buffer);
    }

    /// Move to first non-blank of previous line (- motion).
    pub fn minus_motion(buffer: &mut Buffer, count: usize) {
        let current_line = buffer.cursor().line();
        let target_line = current_line.saturating_sub(count);
        buffer
            .cursor_mut()
            .set_position(target_line, GraphemeCol::ZERO);
        Self::first_non_blank(buffer);
    }

    /// Move to last non-blank character on line (g_ motion).
    pub fn last_non_blank(buffer: &mut Buffer) {
        let line_idx = buffer.cursor().line();
        let index = buffer.line_index(line_idx);
        let mut last_non_blank = None;
        for (char_col, character) in buffer
            .rope()
            .line(line_idx)
            .chars()
            .take(index.len_chars())
            .enumerate()
        {
            if !character.is_whitespace() {
                last_non_blank = Some(char_col);
            }
        }
        let col = last_non_blank
            .map(|char_col| index.char_to_grapheme(CharCol(char_col)).0)
            .unwrap_or(0);
        buffer.cursor_mut().set_col(GraphemeCol(col));
    }
}
