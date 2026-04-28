//! Character find motions: f, F, t, T

use super::Motions;
use crate::buffer::Buffer;

impl Motions {
    /// Finds next occurrence of character on current line (f motion)
    /// Returns true if character was found
    pub fn find_char_forward(buffer: &mut Buffer, ch: char, count: usize) -> bool {
        let rope = buffer.rope();
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let grapheme_col = cursor.col();

        if line_idx >= rope.len_lines() {
            return false;
        }

        let line = rope.line(line_idx).to_string();
        let line = line;
        let chars: Vec<char> = line.chars().collect();
        let char_col = crate::unicode::grapheme_to_char_col(&line, grapheme_col).0;

        let mut found_count = 0;
        for (i, &c) in chars.iter().enumerate().skip(char_col + 1) {
            if c == ch {
                found_count += 1;
                if found_count == count {
                    buffer
                        .cursor_mut()
                        .set_col(crate::unicode::char_to_grapheme_col(
                            &line,
                            crate::unicode::CharCol(i),
                        ));
                    return true;
                }
            }
        }
        false
    }

    /// Finds previous occurrence of character on current line (F motion)
    /// Returns true if character was found
    pub fn find_char_backward(buffer: &mut Buffer, ch: char, count: usize) -> bool {
        let rope = buffer.rope();
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let grapheme_col = cursor.col();

        if line_idx >= rope.len_lines() {
            return false;
        }

        let line = rope.line(line_idx).to_string();
        let line = line;
        let chars: Vec<char> = line.chars().collect();
        let char_col = crate::unicode::grapheme_to_char_col(&line, grapheme_col).0;

        if char_col == 0 {
            return false;
        }

        let mut found_count = 0;
        for i in (0..char_col).rev() {
            if chars[i] == ch {
                found_count += 1;
                if found_count == count {
                    buffer
                        .cursor_mut()
                        .set_col(crate::unicode::char_to_grapheme_col(
                            &line,
                            crate::unicode::CharCol(i),
                        ));
                    return true;
                }
            }
        }
        false
    }

    /// Finds next occurrence and positions cursor before it (t motion)
    /// Returns true if character was found
    pub fn till_char_forward(buffer: &mut Buffer, ch: char, count: usize) -> bool {
        let rope = buffer.rope();
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let grapheme_col = cursor.col();

        if line_idx >= rope.len_lines() {
            return false;
        }

        let line = rope.line(line_idx).to_string();
        let line = line;
        let chars: Vec<char> = line.chars().collect();
        let char_col = crate::unicode::grapheme_to_char_col(&line, grapheme_col).0;

        let mut found_count = 0;
        for (i, &c) in chars.iter().enumerate().skip(char_col + 1) {
            if c == ch {
                found_count += 1;
                if found_count == count {
                    // Position cursor one before the character
                    // Only succeed if there's actual movement (i - 1 > char_col)
                    if i > 0 && i - 1 > char_col {
                        buffer
                            .cursor_mut()
                            .set_col(crate::unicode::char_to_grapheme_col(
                                &line,
                                crate::unicode::CharCol(i - 1),
                            ));
                        return true;
                    }
                    return false;
                }
            }
        }
        false
    }

    /// Finds previous occurrence and positions cursor after it (T motion)
    /// Returns true if character was found
    pub fn till_char_backward(buffer: &mut Buffer, ch: char, count: usize) -> bool {
        let rope = buffer.rope();
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let grapheme_col = cursor.col();

        if line_idx >= rope.len_lines() {
            return false;
        }

        let line = rope.line(line_idx).to_string();
        let line = line;
        let chars: Vec<char> = line.chars().collect();
        let char_col = crate::unicode::grapheme_to_char_col(&line, grapheme_col).0;

        if char_col == 0 {
            return false;
        }

        let mut found_count = 0;
        for i in (0..char_col).rev() {
            if chars[i] == ch {
                found_count += 1;
                if found_count == count {
                    // Position cursor one after the character
                    // Only succeed if there's actual movement (i + 1 < char_col)
                    // OV-00205: symmetric with the forward till fix (OV-00087)
                    if i + 1 < char_col {
                        buffer
                            .cursor_mut()
                            .set_col(crate::unicode::char_to_grapheme_col(
                                &line,
                                crate::unicode::CharCol(i + 1),
                            ));
                        return true;
                    }
                    return false;
                }
            }
        }
        false
    }
}
