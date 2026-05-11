//! Bracket matching and enclosing-char motions: %, [{, ]}, [(, ])

use super::Motions;
use crate::buffer::Buffer;
use crate::unicode::GraphemeCol;

impl Motions {
    /// Jump to matching bracket/paren/brace (% motion)
    /// Returns true if a match was found
    pub fn jump_to_matching_bracket(buffer: &mut Buffer) -> bool {
        let rope = buffer.rope();
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let grapheme_col = cursor.col();

        if line_idx >= rope.len_lines() {
            return false;
        }

        // Get all text from buffer to search across lines
        let text = rope.to_string();
        let chars: Vec<char> = text.chars().collect();

        // Absolute char position of the cursor in the full buffer text.
        let current_line_str = crate::display::line_content(rope, line_idx);
        let char_col = crate::unicode::grapheme_to_char_col(&current_line_str, grapheme_col).0;
        let abs_pos = rope.line_to_char(line_idx) + char_col;

        if abs_pos >= chars.len() {
            return false;
        }

        fn is_bracket(c: char) -> bool {
            matches!(c, '(' | ')' | '[' | ']' | '{' | '}')
        }

        // Determine bracket position: on cursor, or search forward on current line
        let (bracket_pos, current_char) = if is_bracket(chars[abs_pos]) {
            (abs_pos, chars[abs_pos])
        } else {
            // Search forward on current line for nearest bracket
            let line_chars: Vec<char> = current_line_str.chars().collect();
            let mut found = None;
            for (search_col, &ch) in line_chars.iter().enumerate().skip(char_col + 1) {
                if is_bracket(ch) {
                    found = Some((abs_pos + (search_col - char_col), ch));
                    break;
                }
            }
            match found {
                Some(f) => f,
                None => return false,
            }
        };

        // Determine if we're on a bracket and its type
        let (is_opening, matching_char) = match current_char {
            '(' => (true, ')'),
            ')' => (false, '('),
            '[' => (true, ']'),
            ']' => (false, '['),
            '{' => (true, '}'),
            '}' => (false, '{'),
            _ => return false,
        };

        // Search for matching bracket
        let match_pos = if is_opening {
            Self::find_matching_bracket_forward(&chars, bracket_pos, current_char, matching_char)
        } else {
            Self::find_matching_bracket_backward(&chars, bracket_pos, matching_char, current_char)
        };

        if let Some(pos) = match_pos {
            // Convert absolute position back to line+col (char-based)
            let (new_line, new_char_col) = Self::abs_pos_to_line_col(rope, pos);
            let target_line_str = crate::display::line_content(rope, new_line);
            let new_grapheme_col =
                crate::unicode::char_to_grapheme_col(&target_line_str, new_char_col);
            buffer.cursor_mut().set_position(new_line, new_grapheme_col);
            true
        } else {
            false
        }
    }

    /// Find matching closing bracket searching forward
    pub fn find_matching_bracket_forward(
        chars: &[char],
        start_pos: usize,
        open: char,
        close: char,
    ) -> Option<usize> {
        let mut depth = 1;
        for (i, &ch) in chars.iter().enumerate().skip(start_pos + 1) {
            if ch == open {
                depth += 1;
            } else if ch == close {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Find matching opening bracket searching backward
    pub fn find_matching_bracket_backward(
        chars: &[char],
        start_pos: usize,
        open: char,
        close: char,
    ) -> Option<usize> {
        let mut depth = 1;
        for i in (0..start_pos).rev() {
            let ch = chars[i];
            if ch == close {
                depth += 1;
            } else if ch == open {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Jump to enclosing `{` brace
    /// `[{` motion in Vim
    pub fn jump_to_enclosing_open_brace(buffer: &mut Buffer) -> bool {
        let rope = buffer.rope();
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        let text = rope.to_string();
        let chars: Vec<char> = text.chars().collect();

        // Absolute char position of the cursor in the full buffer text.
        let abs_pos = rope.line_to_char(line_idx)
            + crate::unicode::grapheme_to_char_col(
                &crate::display::line_content(rope, line_idx),
                col,
            )
            .0;

        if abs_pos >= chars.len() {
            return false;
        }

        // Search backward for unmatched `{`
        let mut depth = 0;
        let mut search_pos = abs_pos;

        while search_pos > 0 {
            search_pos -= 1;
            match chars[search_pos] {
                '}' => depth += 1,
                '{' => {
                    if depth == 0 {
                        // Found unmatched opening brace
                        let (new_line, new_char_col) = Self::abs_pos_to_line_col(rope, search_pos);
                        let target_line_str = crate::display::line_content(rope, new_line);
                        let new_grapheme_col =
                            crate::unicode::char_to_grapheme_col(&target_line_str, new_char_col);
                        buffer.cursor_mut().set_position(new_line, new_grapheme_col);
                        return true;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }

        // Check position 0
        if chars[0] == '{' && depth == 0 {
            buffer.cursor_mut().set_position(0, GraphemeCol::ZERO);
            return true;
        }

        false
    }

    /// Jump to enclosing `}` brace
    /// `]}` motion in Vim
    pub fn jump_to_enclosing_close_brace(buffer: &mut Buffer) -> bool {
        let rope = buffer.rope();
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        let text = rope.to_string();
        let chars: Vec<char> = text.chars().collect();

        // Absolute char position of the cursor in the full buffer text.
        let abs_pos = rope.line_to_char(line_idx)
            + crate::unicode::grapheme_to_char_col(
                &crate::display::line_content(rope, line_idx),
                col,
            )
            .0;

        if abs_pos >= chars.len() {
            return false;
        }

        // Search forward for unmatched `}` — skip cursor character
        let mut depth = 0;
        let mut search_pos = abs_pos + 1;

        while search_pos < chars.len() {
            match chars[search_pos] {
                '{' => depth += 1,
                '}' => {
                    if depth == 0 {
                        // Found unmatched closing brace
                        let (new_line, new_char_col) = Self::abs_pos_to_line_col(rope, search_pos);
                        let target_line_str = crate::display::line_content(rope, new_line);
                        let new_grapheme_col =
                            crate::unicode::char_to_grapheme_col(&target_line_str, new_char_col);
                        buffer.cursor_mut().set_position(new_line, new_grapheme_col);
                        return true;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            search_pos += 1;
        }

        false
    }

    /// Unmatched brace backward: `[{` motion in Vim
    /// Jumps to the previous unmatched `{` (opening brace that has no matching closer before cursor)
    pub fn unmatched_brace_backward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            if !Self::jump_to_enclosing_open_brace(buffer) {
                break;
            }
        }
    }

    /// Unmatched brace forward: `]}` motion in Vim
    /// Jumps to the next unmatched `}` (closing brace that has no matching opener after cursor)
    pub fn unmatched_brace_forward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            if !Self::jump_to_enclosing_close_brace(buffer) {
                break;
            }
        }
    }

    /// Unmatched parenthesis backward: `[(` motion in Vim
    /// Jumps to the previous unmatched `(` (opening paren that has no matching closer before cursor)
    pub fn unmatched_paren_backward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            if !Self::jump_to_enclosing_char(buffer, '(', ')', true) {
                break;
            }
        }
    }

    /// Unmatched parenthesis forward: `])` motion in Vim
    /// Jumps to the next unmatched `)` (closing paren that has no matching opener after cursor)
    pub fn unmatched_paren_forward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            if !Self::jump_to_enclosing_char(buffer, '(', ')', false) {
                break;
            }
        }
    }

    /// Generic jump to enclosing character
    /// If `backward` is true, searches for unmatched opener; otherwise, searches for unmatched closer
    fn jump_to_enclosing_char(
        buffer: &mut Buffer,
        open_char: char,
        close_char: char,
        backward: bool,
    ) -> bool {
        let rope = buffer.rope();
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        let text = rope.to_string();
        let chars: Vec<char> = text.chars().collect();

        // Absolute char position of the cursor in the full buffer text.
        let abs_pos = rope.line_to_char(line_idx)
            + crate::unicode::grapheme_to_char_col(
                &crate::display::line_content(rope, line_idx),
                col,
            )
            .0;

        if abs_pos >= chars.len() {
            return false;
        }

        if backward {
            // Search backward for unmatched opener
            let mut depth = 0;
            let mut search_pos = abs_pos;

            while search_pos > 0 {
                search_pos -= 1;
                match chars[search_pos] {
                    c if c == close_char => depth += 1,
                    c if c == open_char => {
                        if depth == 0 {
                            let (new_line, new_char_col) =
                                Self::abs_pos_to_line_col(rope, search_pos);
                            let target_line_str = crate::display::line_content(rope, new_line);
                            let new_grapheme_col = crate::unicode::char_to_grapheme_col(
                                &target_line_str,
                                new_char_col,
                            );
                            buffer.cursor_mut().set_position(new_line, new_grapheme_col);
                            return true;
                        }
                        depth -= 1;
                    }
                    _ => {}
                }
            }

            // Check position 0
            if chars[0] == open_char && depth == 0 {
                buffer.cursor_mut().set_position(0, GraphemeCol::ZERO);
                return true;
            }
        } else {
            // Search forward for unmatched closer — skip cursor character
            let mut depth = 0;
            let mut search_pos = abs_pos + 1;

            while search_pos < chars.len() {
                match chars[search_pos] {
                    c if c == open_char => depth += 1,
                    c if c == close_char => {
                        if depth == 0 {
                            let (new_line, new_char_col) =
                                Self::abs_pos_to_line_col(rope, search_pos);
                            let target_line_str = crate::display::line_content(rope, new_line);
                            let new_grapheme_col = crate::unicode::char_to_grapheme_col(
                                &target_line_str,
                                new_char_col,
                            );
                            buffer.cursor_mut().set_position(new_line, new_grapheme_col);
                            return true;
                        }
                        depth -= 1;
                    }
                    _ => {}
                }
                search_pos += 1;
            }
        }

        false
    }
}
