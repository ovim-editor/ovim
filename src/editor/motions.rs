use crate::buffer::Buffer;

/// Utilities for cursor motions
pub struct Motions;

impl Motions {
    /// Checks if a character is a word character (alphanumeric or underscore)
    fn is_word_char(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }

    /// Checks if a character is whitespace
    fn is_whitespace(c: char) -> bool {
        c.is_whitespace()
    }

    /// Moves cursor forward to the start of the next word
    /// w - moves to start of next word (word = alphanumeric + underscore)
    pub fn word_forward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_forward_once(buffer, false);
        }
    }

    /// Moves cursor forward to the start of the next WORD
    /// W - moves to start of next WORD (WORD = any non-whitespace)
    pub fn word_forward_big(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_forward_once(buffer, true);
        }
    }

    fn word_forward_once(buffer: &mut Buffer, big_word: bool) {
        let rope = buffer.rope();
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        if line_idx >= rope.len_lines() {
            return;
        }

        let line = rope.line(line_idx).to_string();
        let line = line.trim_end_matches('\n');
        let chars: Vec<char> = line.chars().collect();

        if col >= chars.len() {
            // At end of line, move to next line
            if line_idx + 1 < rope.len_lines() {
                buffer.cursor_mut().set_position(line_idx + 1, 0);
                // Skip leading whitespace
                Self::skip_whitespace_forward(buffer);
            }
            return;
        }

        let current_char = chars[col];
        let mut new_col = col;

        if big_word {
            // Skip current WORD (non-whitespace)
            if !Self::is_whitespace(current_char) {
                while new_col < chars.len() && !Self::is_whitespace(chars[new_col]) {
                    new_col += 1;
                }
            }
            // Skip whitespace
            while new_col < chars.len() && Self::is_whitespace(chars[new_col]) {
                new_col += 1;
            }
        } else {
            // Skip current word
            if Self::is_word_char(current_char) {
                // In a word
                while new_col < chars.len() && Self::is_word_char(chars[new_col]) {
                    new_col += 1;
                }
            } else if !Self::is_whitespace(current_char) {
                // In punctuation
                while new_col < chars.len()
                    && !Self::is_word_char(chars[new_col])
                    && !Self::is_whitespace(chars[new_col]) {
                    new_col += 1;
                }
            }
            // Skip whitespace
            while new_col < chars.len() && Self::is_whitespace(chars[new_col]) {
                new_col += 1;
            }
        }

        if new_col >= chars.len() && line_idx + 1 < rope.len_lines() {
            // Reached end of line, move to next line
            buffer.cursor_mut().set_position(line_idx + 1, 0);
            Self::skip_whitespace_forward(buffer);
        } else {
            buffer.cursor_mut().set_col(new_col.min(chars.len().saturating_sub(1).max(0)));
        }
    }

    /// Moves cursor backward to the start of the previous word
    /// b - moves to start of previous word
    pub fn word_backward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_backward_once(buffer, false);
        }
    }

    /// Moves cursor backward to the start of the previous WORD
    /// B - moves to start of previous WORD
    pub fn word_backward_big(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_backward_once(buffer, true);
        }
    }

    fn word_backward_once(buffer: &mut Buffer, big_word: bool) {
        let rope = buffer.rope();
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        if line_idx >= rope.len_lines() {
            return;
        }

        let line = rope.line(line_idx).to_string();
        let line = line.trim_end_matches('\n');
        let chars: Vec<char> = line.chars().collect();

        if col == 0 {
            // At start of line, move to previous line
            if line_idx > 0 {
                let prev_line = rope.line(line_idx - 1).to_string();
                let prev_line = prev_line.trim_end_matches('\n');
                let prev_len = prev_line.chars().count();
                buffer.cursor_mut().set_position(
                    line_idx - 1,
                    prev_len.saturating_sub(1).max(0)
                );
            }
            return;
        }

        let mut new_col = col;

        // Skip backward over whitespace first
        if new_col > 0 && new_col < chars.len() && Self::is_whitespace(chars[new_col - 1]) {
            while new_col > 0 && Self::is_whitespace(chars[new_col - 1]) {
                new_col -= 1;
            }
        }

        if new_col == 0 {
            buffer.cursor_mut().set_col(0);
            return;
        }

        if big_word {
            // Move back to start of WORD
            while new_col > 0 && !Self::is_whitespace(chars[new_col - 1]) {
                new_col -= 1;
            }
        } else {
            let target_char = chars[new_col - 1];
            if Self::is_word_char(target_char) {
                // Move back through word
                while new_col > 0 && Self::is_word_char(chars[new_col - 1]) {
                    new_col -= 1;
                }
            } else {
                // Move back through punctuation
                while new_col > 0
                    && !Self::is_word_char(chars[new_col - 1])
                    && !Self::is_whitespace(chars[new_col - 1]) {
                    new_col -= 1;
                }
            }
        }

        buffer.cursor_mut().set_col(new_col);
    }

    /// Moves cursor forward to the end of the current/next word
    /// e - moves to end of word
    pub fn word_end_forward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_end_forward_once(buffer, false);
        }
    }

    /// Moves cursor forward to the end of the current/next WORD
    /// E - moves to end of WORD
    pub fn word_end_forward_big(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_end_forward_once(buffer, true);
        }
    }

    fn word_end_forward_once(buffer: &mut Buffer, big_word: bool) {
        let rope = buffer.rope();
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        if line_idx >= rope.len_lines() {
            return;
        }

        let line = rope.line(line_idx).to_string();
        let line = line.trim_end_matches('\n');
        let chars: Vec<char> = line.chars().collect();

        if chars.is_empty() {
            if line_idx + 1 < rope.len_lines() {
                buffer.cursor_mut().set_position(line_idx + 1, 0);
            }
            return;
        }

        let mut new_col = col;

        // Move forward at least one character
        if new_col < chars.len() {
            new_col += 1;
        }

        // Skip whitespace
        while new_col < chars.len() && Self::is_whitespace(chars[new_col]) {
            new_col += 1;
        }

        if new_col >= chars.len() {
            if line_idx + 1 < rope.len_lines() {
                buffer.cursor_mut().set_position(line_idx + 1, 0);
                Self::word_end_forward_once(buffer, big_word);
            }
            return;
        }

        if big_word {
            // Move to end of WORD
            while new_col < chars.len() && !Self::is_whitespace(chars[new_col]) {
                new_col += 1;
            }
        } else {
            let start_char = chars[new_col];
            if Self::is_word_char(start_char) {
                // Move through word
                while new_col < chars.len() && Self::is_word_char(chars[new_col]) {
                    new_col += 1;
                }
            } else {
                // Move through punctuation
                while new_col < chars.len()
                    && !Self::is_word_char(chars[new_col])
                    && !Self::is_whitespace(chars[new_col]) {
                    new_col += 1;
                }
            }
        }

        buffer.cursor_mut().set_col((new_col - 1).min(chars.len().saturating_sub(1)));
    }

    fn skip_whitespace_forward(buffer: &mut Buffer) {
        let rope = buffer.rope();
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        if line_idx >= rope.len_lines() {
            return;
        }

        let line = rope.line(line_idx).to_string();
        let line = line.trim_end_matches('\n');
        let chars: Vec<char> = line.chars().collect();

        let mut new_col = col;
        while new_col < chars.len() && Self::is_whitespace(chars[new_col]) {
            new_col += 1;
        }

        buffer.cursor_mut().set_col(new_col.min(chars.len().saturating_sub(1).max(0)));
    }
}
