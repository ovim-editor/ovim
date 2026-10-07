//! Character find motions: f, F, t, T

use super::Motions;
use crate::buffer::Buffer;
use crate::unicode::CharCol;

fn find_forward_char(
    buffer: &Buffer,
    line_idx: usize,
    after_char: usize,
    target: char,
    count: usize,
) -> Option<usize> {
    let index = buffer.line_index(line_idx);
    let start = index.char_to_grapheme(CharCol(after_char));
    let mut found = 0;
    for grapheme in index.graphemes_from(start) {
        for (offset, character) in grapheme.text.chars().enumerate() {
            let char_col = grapheme.char_start + offset;
            if char_col > after_char && character == target {
                found += 1;
                if found == count {
                    return Some(char_col);
                }
            }
        }
    }
    None
}

fn find_backward_char(
    buffer: &Buffer,
    line_idx: usize,
    before_char: usize,
    target: char,
    count: usize,
) -> Option<usize> {
    let line = buffer.rope().line(line_idx);
    let mut found = 0;
    for (offset, character) in line.chars_at(before_char).reversed().enumerate() {
        if character == target {
            found += 1;
            if found == count {
                return Some(before_char - offset - 1);
            }
        }
    }
    None
}

impl Motions {
    /// Finds the next occurrence of a character on the current line (f).
    pub fn find_char_forward(buffer: &mut Buffer, ch: char, count: usize) -> bool {
        let line_idx = buffer.cursor().line();
        if line_idx >= buffer.line_count() {
            return false;
        }
        let cursor_char = buffer
            .line_index(line_idx)
            .grapheme_to_char(buffer.cursor().col())
            .0;
        let Some(found_char) = find_forward_char(buffer, line_idx, cursor_char, ch, count) else {
            return false;
        };
        let grapheme = buffer
            .line_index(line_idx)
            .char_to_grapheme(CharCol(found_char));
        buffer.cursor_mut().set_col(grapheme);
        true
    }

    /// Finds the previous occurrence of a character on the current line (F).
    pub fn find_char_backward(buffer: &mut Buffer, ch: char, count: usize) -> bool {
        let line_idx = buffer.cursor().line();
        if line_idx >= buffer.line_count() {
            return false;
        }
        let cursor_char = buffer
            .line_index(line_idx)
            .grapheme_to_char(buffer.cursor().col())
            .0;
        let Some(found_char) = find_backward_char(buffer, line_idx, cursor_char, ch, count) else {
            return false;
        };
        let grapheme = buffer
            .line_index(line_idx)
            .char_to_grapheme(CharCol(found_char));
        buffer.cursor_mut().set_col(grapheme);
        true
    }

    /// Finds the next occurrence and positions the cursor before it (t).
    pub fn till_char_forward(buffer: &mut Buffer, ch: char, count: usize) -> bool {
        let line_idx = buffer.cursor().line();
        if line_idx >= buffer.line_count() {
            return false;
        }
        let cursor_char = buffer
            .line_index(line_idx)
            .grapheme_to_char(buffer.cursor().col())
            .0;
        let Some(found_char) = find_forward_char(buffer, line_idx, cursor_char, ch, count) else {
            return false;
        };
        // A target right next to the cursor still succeeds (staying put), so
        // `ct"` on the character before a quote works.
        let grapheme = buffer
            .line_index(line_idx)
            .char_to_grapheme(CharCol(found_char - 1));
        buffer.cursor_mut().set_col(grapheme);
        true
    }

    /// Finds the previous occurrence and positions the cursor after it (T).
    pub fn till_char_backward(buffer: &mut Buffer, ch: char, count: usize) -> bool {
        let line_idx = buffer.cursor().line();
        if line_idx >= buffer.line_count() {
            return false;
        }
        let cursor_char = buffer
            .line_index(line_idx)
            .grapheme_to_char(buffer.cursor().col())
            .0;
        let Some(found_char) = find_backward_char(buffer, line_idx, cursor_char, ch, count) else {
            return false;
        };
        // A target right next to the cursor still succeeds (staying put), so
        // `ct"` on the character before a quote works.
        let grapheme = buffer
            .line_index(line_idx)
            .char_to_grapheme(CharCol(found_char + 1));
        buffer.cursor_mut().set_col(grapheme);
        true
    }
}
