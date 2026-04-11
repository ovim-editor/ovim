use crate::buffer::Buffer;
use crate::unicode::grapheme_count;

/// Character classification for word motions.
/// CJK ideographs are treated as individual words (each char = one word),
/// matching Vim's behavior.
#[derive(PartialEq, Eq, Clone, Copy)]
enum CharClass {
    Word,        // ASCII alphanumeric + underscore
    Cjk,         // CJK ideographs, Hiragana, Katakana, Hangul, Bopomofo
    Punctuation, // everything else that's not whitespace
    Whitespace,
}

fn char_class(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Whitespace
    } else if is_cjk_ideograph(c) {
        CharClass::Cjk
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punctuation
    }
}

fn is_cjk_ideograph(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF       // CJK Unified Ideographs
        | 0x3400..=0x4DBF     // CJK Extension A
        | 0x20000..=0x2A6DF   // CJK Extension B
        | 0x2A700..=0x2B73F   // CJK Extension C
        | 0x2B740..=0x2B81F   // CJK Extension D
        | 0x2B820..=0x2CEAF   // CJK Extension E
        | 0x2CEB0..=0x2EBEF   // CJK Extension F
        | 0x30000..=0x3134F   // CJK Extension G
        | 0x3100..=0x312F     // Bopomofo
        | 0x31A0..=0x31BF     // Bopomofo Extended
        | 0x3040..=0x309F     // Hiragana
        | 0x30A0..=0x30FF     // Katakana
        | 0x31F0..=0x31FF     // Katakana Phonetic Extensions
        | 0xAC00..=0xD7AF     // Hangul Syllables
        | 0x1100..=0x11FF     // Hangul Jamo
    )
}

/// Utilities for cursor motions
pub struct Motions;

impl Motions {
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
            // At end of line (or empty line), advance to next word start.
            if let Some((next_line, next_col)) =
                Self::find_next_word_start(rope, line_idx + 1, big_word)
            {
                buffer.cursor_mut().set_position(next_line, next_col);
            }
            // else: at end of buffer, don't move
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
            // Skip current word/CJK/punctuation
            let class = char_class(current_char);
            match class {
                CharClass::Cjk => {
                    // Each CJK char is its own word — skip exactly one
                    new_col += 1;
                }
                CharClass::Word => {
                    while new_col < chars.len() && char_class(chars[new_col]) == CharClass::Word {
                        new_col += 1;
                    }
                }
                CharClass::Punctuation => {
                    while new_col < chars.len()
                        && char_class(chars[new_col]) == CharClass::Punctuation
                    {
                        new_col += 1;
                    }
                }
                CharClass::Whitespace => {}
            }
            // Skip whitespace
            while new_col < chars.len() && Self::is_whitespace(chars[new_col]) {
                new_col += 1;
            }
        }

        if new_col >= chars.len() {
            // Ran past end of line — advance to next word start across lines.
            if let Some((next_line, next_col)) =
                Self::find_next_word_start(rope, line_idx + 1, big_word)
            {
                buffer.cursor_mut().set_position(next_line, next_col);
            }
            // else: end of buffer, don't move
        } else {
            buffer
                .cursor_mut()
                .set_col(new_col.min(chars.len().saturating_sub(1).max(0)));
        }
    }

    /// Scan forward from `start_line` looking for the next word start.
    ///
    /// Vim rules for cross-line `w`/`W`:
    /// - Empty line (zero visible chars) → word boundary, return `(line, 0)`
    /// - Whitespace-only line → skip it entirely
    /// - Line with non-whitespace → return `(line, first_non_ws_col)`
    fn find_next_word_start(
        rope: &ropey::Rope,
        start_line: usize,
        _big_word: bool,
    ) -> Option<(usize, usize)> {
        // Use Vim-compatible line count: exclude the phantom empty line that
        // ropey appends after a trailing '\n'. Motions should not land there.
        let total_lines = {
            let raw = rope.len_lines();
            if raw > 1 && rope.len_chars() > 0 && rope.char(rope.len_chars() - 1) == '\n' {
                raw - 1
            } else {
                raw
            }
        };
        for line_idx in start_line..total_lines {
            let line = rope.line(line_idx).to_string();
            let content = line.trim_end_matches('\n');

            if content.is_empty() {
                // Truly empty line — word boundary, stop here.
                return Some((line_idx, 0));
            }

            // Check if line has any non-whitespace
            if let Some(pos) = content.chars().position(|c| !c.is_whitespace()) {
                return Some((line_idx, pos));
            }

            // Whitespace-only line — skip.
        }
        None
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
        let mut line_idx = cursor.line();
        let mut col = cursor.col();

        if line_idx >= rope.len_lines() {
            return;
        }

        if col == 0 {
            // At start of line, move to end of previous line and continue
            if line_idx > 0 {
                line_idx -= 1;
                let prev_line = rope.line(line_idx).to_string();
                let prev_line_trimmed = prev_line.trim_end_matches('\n');
                col = grapheme_count(prev_line_trimmed);
                // If previous line is empty, just land at col 0
                if col == 0 {
                    buffer.cursor_mut().set_position(line_idx, 0);
                    return;
                }
                // col is now one past the last char; fall through to word-backward logic
            } else {
                return;
            }
        }

        let line = rope.line(line_idx).to_string();
        let line = line.trim_end_matches('\n');
        let chars: Vec<char> = line.chars().collect();
        let mut new_col = col;

        // Skip backward over whitespace first
        if new_col > 0 && new_col <= chars.len() {
            // When new_col == chars.len(), we're past the end; check chars[new_col - 1]
            while new_col > 0 && Self::is_whitespace(chars[new_col - 1]) {
                new_col -= 1;
            }
        }

        if new_col == 0 {
            buffer.cursor_mut().set_position(line_idx, 0);
            return;
        }

        if big_word {
            // Move back to start of WORD
            while new_col > 0 && !Self::is_whitespace(chars[new_col - 1]) {
                new_col -= 1;
            }
        } else {
            let target_char = chars[new_col - 1];
            let class = char_class(target_char);
            match class {
                CharClass::Cjk => {
                    // Each CJK char is its own word — back exactly one
                    new_col -= 1;
                }
                CharClass::Word => {
                    while new_col > 0 && char_class(chars[new_col - 1]) == CharClass::Word {
                        new_col -= 1;
                    }
                }
                CharClass::Punctuation => {
                    while new_col > 0 && char_class(chars[new_col - 1]) == CharClass::Punctuation {
                        new_col -= 1;
                    }
                }
                CharClass::Whitespace => {}
            }
        }

        buffer.cursor_mut().set_position(line_idx, new_col);
    }

    /// Moves cursor forward to the end of the current/next word
    /// e - moves to end of word
    pub fn word_end_forward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_end_forward_once(buffer, false, false);
        }
    }

    /// Variant of `word_end_forward` that prefers the current word on the
    /// first step, even if the cursor is already at that word's end.
    ///
    /// This matches Vim's `cw`/`ce` behavior on single-character words.
    pub fn word_end_forward_prefer_current(buffer: &mut Buffer, count: usize) {
        for i in 0..count {
            Self::word_end_forward_once(buffer, false, i == 0);
        }
    }

    /// Moves cursor forward to the end of the current/next WORD
    /// E - moves to end of WORD
    pub fn word_end_forward_big(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_end_forward_once(buffer, true, false);
        }
    }

    pub fn word_end_forward_big_prefer_current(buffer: &mut Buffer, count: usize) {
        for i in 0..count {
            Self::word_end_forward_once(buffer, true, i == 0);
        }
    }

    fn word_end_forward_once(buffer: &mut Buffer, big_word: bool, prefer_current: bool) {
        let (line_idx, col, total_lines, line_string) = {
            let cursor = buffer.cursor();
            let total_lines = buffer.line_count();

            (
                cursor.line(),
                cursor.col(),
                total_lines,
                buffer.rope().line(cursor.line()).to_string(),
            )
        };

        if line_idx >= total_lines {
            return;
        }

        let line = line_string.trim_end_matches('\n');
        let chars: Vec<char> = line.chars().collect();

        if chars.is_empty() {
            // Skip consecutive blank lines to find next non-empty line
            let mut next_line = line_idx + 1;
            while next_line < total_lines {
                let next = buffer.rope().line(next_line).to_string();
                let next_trimmed = next.trim_end_matches('\n');
                if !next_trimmed.is_empty() {
                    let next_chars: Vec<char> = next_trimmed.chars().collect();
                    // Start at first non-ws, then move to end of that word
                    let Some(start_col) = next_chars.iter().position(|c| !c.is_whitespace()) else {
                        next_line += 1;
                        continue;
                    };
                    buffer.cursor_mut().set_position(next_line, start_col);
                    Self::word_end_forward_once(buffer, big_word, prefer_current);
                    return;
                }
                next_line += 1;
            }
            buffer
                .cursor_mut()
                .set_position(total_lines.saturating_sub(1), 0);
            return;
        }

        if col >= chars.len() {
            if line_idx + 1 < total_lines {
                buffer.cursor_mut().set_position(line_idx + 1, 0);
                Self::word_end_forward_once(buffer, big_word, prefer_current);
            }
            return;
        }

        let mut idx = col;

        let is_ws = chars.get(idx).is_some_and(|c| c.is_whitespace());
        if is_ws {
            while idx < chars.len() && chars[idx].is_whitespace() {
                idx += 1;
            }
            if idx >= chars.len() {
                if line_idx + 1 < total_lines {
                    buffer.cursor_mut().set_position(line_idx + 1, 0);
                    Self::word_end_forward_once(buffer, big_word, prefer_current);
                }
                return;
            }
        } else {
            let end_of_current = if big_word {
                let mut end = idx;
                while end + 1 < chars.len() && !chars[end + 1].is_whitespace() {
                    end += 1;
                }
                end
            } else {
                let class = char_class(chars[idx]);
                match class {
                    CharClass::Cjk => idx,
                    CharClass::Word | CharClass::Punctuation => {
                        let mut end = idx;
                        while end + 1 < chars.len() && char_class(chars[end + 1]) == class {
                            end += 1;
                        }
                        end
                    }
                    CharClass::Whitespace => idx,
                }
            };

            if prefer_current || idx < end_of_current {
                buffer.cursor_mut().set_col(end_of_current);
                return;
            }

            // Subsequent repeats: already at end of current word — advance into next word.
            idx = idx.saturating_add(1);
            while idx < chars.len() && chars[idx].is_whitespace() {
                idx += 1;
            }
            if idx >= chars.len() {
                if line_idx + 1 < total_lines {
                    buffer.cursor_mut().set_position(line_idx + 1, 0);
                    Self::word_end_forward_once(buffer, big_word, prefer_current);
                }
                return;
            }
        }

        let end_of_next = if big_word {
            let mut end = idx;
            while end + 1 < chars.len() && !chars[end + 1].is_whitespace() {
                end += 1;
            }
            end
        } else {
            let class = char_class(chars[idx]);
            match class {
                CharClass::Cjk => idx,
                CharClass::Word | CharClass::Punctuation => {
                    let mut end = idx;
                    while end + 1 < chars.len() && char_class(chars[end + 1]) == class {
                        end += 1;
                    }
                    end
                }
                CharClass::Whitespace => idx,
            }
        };

        buffer.cursor_mut().set_col(end_of_next);
    }

    /// Moves cursor backward to the end of the previous word
    /// ge - moves to end of previous word
    pub fn word_end_backward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_end_backward_once(buffer, false);
        }
    }

    /// Moves cursor backward to the end of the previous WORD
    /// gE - moves to end of previous WORD
    pub fn word_end_backward_big(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_end_backward_once(buffer, true);
        }
    }

    fn word_end_backward_once(buffer: &mut Buffer, big_word: bool) {
        // ge/gE: move backward to the end of the previous word/WORD.
        // Algorithm:
        //   1. Move back one position (crossing lines)
        //   2. Skip whitespace backward (crossing lines)
        //   3. Now we're on a non-ws char. Check: did we cross a word class boundary
        //      relative to the original cursor position? If not, we're still inside
        //      the same word, so skip to start of this word class, then skip whitespace
        //      again. The char we land on is the end of the previous word.
        //   4. If we DID cross a boundary (or whitespace), we're already at the end of
        //      the previous word. Done.

        let rope = buffer.rope();
        let orig_line = buffer.cursor().line();
        let orig_col = buffer.cursor().col();

        if orig_line >= rope.len_lines() {
            return;
        }

        // Helper to get line chars
        let get_chars = |l: usize| -> Vec<char> {
            rope.line(l)
                .to_string()
                .trim_end_matches('\n')
                .chars()
                .collect()
        };

        let orig_chars = get_chars(orig_line);
        let orig_class = if orig_col < orig_chars.len() {
            Some(if big_word {
                // For WORD: any non-ws is the same "class"
                if Self::is_whitespace(orig_chars[orig_col]) {
                    CharClass::Whitespace
                } else {
                    CharClass::Word // treat all non-ws as Word for big_word
                }
            } else {
                char_class(orig_chars[orig_col])
            })
        } else {
            None
        };

        let mut line_idx = orig_line;
        let mut col = orig_col;

        // Step 1: Move back one position
        if col == 0 {
            if line_idx == 0 {
                return;
            }
            line_idx -= 1;
            let chars = get_chars(line_idx);
            col = if chars.is_empty() { 0 } else { chars.len() - 1 };
        } else {
            col -= 1;
        }

        // Step 2: Skip whitespace backward (crossing lines), landing on a non-ws char
        let (ws_line, ws_col) = Self::skip_whitespace_backward(line_idx, col, &get_chars);
        line_idx = ws_line;
        col = ws_col;

        let chars = get_chars(line_idx);
        if chars.is_empty() {
            buffer.cursor_mut().set_position(line_idx, 0);
            return;
        }

        // Step 3: Check if we crossed a word boundary
        let current_class = if big_word {
            CharClass::Word // all non-ws treated as same for WORD
        } else {
            char_class(chars[col])
        };

        // If we're on a different line, or crossed whitespace, or different word class,
        // then we already crossed a word boundary — this IS the end of the previous word.
        let crossed_boundary = line_idx != orig_line
            || orig_class != Some(current_class)
            || col < orig_col.saturating_sub(1); // whitespace was skipped

        if crossed_boundary {
            buffer.cursor_mut().set_position(line_idx, col);
            return;
        }

        // Still in same word — skip to start of this word class, then find end of previous word
        if big_word {
            while col > 0 && !Self::is_whitespace(chars[col - 1]) {
                col -= 1;
            }
        } else {
            while col > 0 && char_class(chars[col - 1]) == current_class {
                col -= 1;
            }
        }

        // Move back one more — if we can't, there's no previous word; don't move
        if col == 0 {
            if line_idx == 0 {
                return; // No previous word — leave cursor unchanged
            }
            line_idx -= 1;
            let prev_chars = get_chars(line_idx);
            col = if prev_chars.is_empty() {
                0
            } else {
                prev_chars.len() - 1
            };
        } else {
            col -= 1;
        }

        // Skip whitespace backward again
        let (final_line, final_col) = Self::skip_whitespace_backward(line_idx, col, &get_chars);
        buffer.cursor_mut().set_position(final_line, final_col);
    }

    /// Skip whitespace backward (crossing lines), returning the position of
    /// the first non-whitespace character found.
    fn skip_whitespace_backward(
        mut line_idx: usize,
        mut col: usize,
        get_chars: &dyn Fn(usize) -> Vec<char>,
    ) -> (usize, usize) {
        loop {
            let chars = get_chars(line_idx);
            if chars.is_empty() {
                if line_idx == 0 {
                    return (0, 0);
                }
                line_idx -= 1;
                let prev = get_chars(line_idx);
                col = if prev.is_empty() { 0 } else { prev.len() - 1 };
                continue;
            }

            // Clamp col
            if col >= chars.len() {
                col = chars.len() - 1;
            }

            // Skip whitespace on this line
            while col > 0 && Self::is_whitespace(chars[col]) {
                col -= 1;
            }

            if !Self::is_whitespace(chars[col]) {
                return (line_idx, col);
            }

            // col == 0 and it's whitespace
            if line_idx == 0 {
                return (0, 0);
            }
            line_idx -= 1;
            let prev = get_chars(line_idx);
            col = if prev.is_empty() { 0 } else { prev.len() - 1 };
        }
    }

    /// Finds next occurrence of character on current line (f motion)
    /// Returns true if character was found
    pub fn find_char_forward(buffer: &mut Buffer, ch: char, count: usize) -> bool {
        let rope = buffer.rope();
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        if line_idx >= rope.len_lines() {
            return false;
        }

        let line = rope.line(line_idx).to_string();
        let line = line.trim_end_matches('\n');
        let chars: Vec<char> = line.chars().collect();

        let mut found_count = 0;
        for (i, &c) in chars.iter().enumerate().skip(col + 1) {
            if c == ch {
                found_count += 1;
                if found_count == count {
                    buffer.cursor_mut().set_col(i);
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
        let col = cursor.col();

        if line_idx >= rope.len_lines() {
            return false;
        }

        let line = rope.line(line_idx).to_string();
        let line = line.trim_end_matches('\n');
        let chars: Vec<char> = line.chars().collect();

        if col == 0 {
            return false;
        }

        let mut found_count = 0;
        for i in (0..col).rev() {
            if chars[i] == ch {
                found_count += 1;
                if found_count == count {
                    buffer.cursor_mut().set_col(i);
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
        let col = cursor.col();

        if line_idx >= rope.len_lines() {
            return false;
        }

        let line = rope.line(line_idx).to_string();
        let line = line.trim_end_matches('\n');
        let chars: Vec<char> = line.chars().collect();

        let mut found_count = 0;
        for (i, &c) in chars.iter().enumerate().skip(col + 1) {
            if c == ch {
                found_count += 1;
                if found_count == count {
                    // Position cursor one before the character
                    // Only succeed if there's actual movement (i - 1 > col)
                    if i > 0 && i - 1 > col {
                        buffer.cursor_mut().set_col(i - 1);
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
        let col = cursor.col();

        if line_idx >= rope.len_lines() {
            return false;
        }

        let line = rope.line(line_idx).to_string();
        let line = line.trim_end_matches('\n');
        let chars: Vec<char> = line.chars().collect();

        if col == 0 {
            return false;
        }

        let mut found_count = 0;
        for i in (0..col).rev() {
            if chars[i] == ch {
                found_count += 1;
                if found_count == count {
                    // Position cursor one after the character
                    if i + 1 < chars.len() {
                        buffer.cursor_mut().set_col(i + 1);
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Jump to matching bracket/paren/brace (% motion)
    /// Returns true if a match was found
    pub fn jump_to_matching_bracket(buffer: &mut Buffer) -> bool {
        let rope = buffer.rope();
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        if line_idx >= rope.len_lines() {
            return false;
        }

        // Get all text from buffer to search across lines
        let text = rope.to_string();
        let chars: Vec<char> = text.chars().collect();

        // Convert line+col to absolute position
        let mut abs_pos = 0;
        for i in 0..line_idx {
            if i < rope.len_lines() {
                abs_pos += rope.line(i).len_chars();
            }
        }
        abs_pos += col;

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
            let line_text = rope.line(line_idx).to_string();
            let line_chars: Vec<char> = line_text.trim_end_matches('\n').chars().collect();
            let mut found = None;
            for (search_col, &ch) in line_chars.iter().enumerate().skip(col + 1) {
                if is_bracket(ch) {
                    found = Some((abs_pos + (search_col - col), ch));
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
            // Convert absolute position back to line+col
            let (new_line, new_col) = Self::abs_pos_to_line_col(rope, pos);
            buffer.cursor_mut().set_position(new_line, new_col);
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

    /// Convert absolute character position to (line, col)
    pub fn abs_pos_to_line_col(rope: &ropey::Rope, abs_pos: usize) -> (usize, usize) {
        let line = rope.char_to_line(abs_pos.min(rope.len_chars().saturating_sub(1)));
        let line_start = rope.line_to_char(line);
        let col = abs_pos.saturating_sub(line_start);
        (line, col)
    }

    /// Move to first non-blank character on line (^ motion)
    pub fn first_non_blank(buffer: &mut Buffer) {
        let cursor = buffer.cursor();
        let line_idx = cursor.line();

        if let Some(line) = buffer.line(line_idx) {
            let line_text = line.trim_end_matches('\n');
            let chars: Vec<char> = line_text.chars().collect();

            // Find first non-whitespace character
            let first_non_blank = chars.iter().position(|&c| !c.is_whitespace()).unwrap_or(0);

            buffer.cursor_mut().set_col(first_non_blank);
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

        buffer.cursor_mut().set_position(target_line, 0);
        Self::first_non_blank(buffer);
    }

    /// Move to first non-blank of previous line (- motion)
    pub fn minus_motion(buffer: &mut Buffer, count: usize) {
        let cursor = buffer.cursor();
        let current_line = cursor.line();
        let target_line = current_line.saturating_sub(count);

        buffer.cursor_mut().set_position(target_line, 0);
        Self::first_non_blank(buffer);
    }

    /// Move to last non-blank character on line (g_ motion)
    pub fn last_non_blank(buffer: &mut Buffer) {
        let cursor = buffer.cursor();
        let line_idx = cursor.line();

        if let Some(line) = buffer.line(line_idx) {
            let line_text = line.trim_end_matches('\n');
            let chars: Vec<char> = line_text.chars().collect();

            // Find last non-whitespace character
            let mut last_non_blank = 0;
            for (i, &c) in chars.iter().enumerate() {
                if !c.is_whitespace() {
                    last_non_blank = i;
                }
            }

            buffer.cursor_mut().set_col(last_non_blank);
        }
    }

    /// Move forward to start of next paragraph ({ and } motions)
    pub fn paragraph_forward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::paragraph_forward_once(buffer);
        }
    }

    fn paragraph_forward_once(buffer: &mut Buffer) {
        let cursor = buffer.cursor();
        let mut line_idx = cursor.line();
        let total_lines = buffer.line_count();

        // First skip any blank lines at/after cursor
        while line_idx < total_lines {
            if let Some(line) = buffer.line(line_idx) {
                if !line.trim().is_empty() {
                    break;
                }
            }
            line_idx += 1;
        }

        // Then skip non-blank lines to find the next blank line boundary
        while line_idx < total_lines {
            if let Some(line) = buffer.line(line_idx) {
                if line.trim().is_empty() {
                    break;
                }
            }
            line_idx += 1;
        }

        // Clamp to buffer bounds
        line_idx = line_idx.min(total_lines.saturating_sub(1));
        buffer.cursor_mut().set_position(line_idx, 0);
    }

    /// Move backward to start of previous paragraph
    pub fn paragraph_backward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::paragraph_backward_once(buffer);
        }
    }

    fn paragraph_backward_once(buffer: &mut Buffer) {
        let cursor = buffer.cursor();
        let mut line_idx = cursor.line();

        if line_idx == 0 {
            return;
        }

        line_idx = line_idx.saturating_sub(1);

        // Fix: Skip blank lines backward - check line 0 explicitly
        while line_idx > 0 {
            if let Some(line) = buffer.line(line_idx) {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    break;
                }
            }
            line_idx = line_idx.saturating_sub(1);
        }
        // Check line 0 after loop (loop condition skips it)
        if line_idx == 0 {
            if let Some(line) = buffer.line(0) {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    // Line 0 is non-blank, continue to next phase
                } else {
                    // Line 0 is blank, stop here
                    buffer.cursor_mut().set_position(0, 0);
                    return;
                }
            }
        }

        // Fix: Skip non-blank lines backward until we find a blank line
        while line_idx > 0 {
            if let Some(line) = buffer.line(line_idx) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    break; // Stop at the blank line
                }
            }
            line_idx = line_idx.saturating_sub(1);
        }
        // Check line 0 after loop - if we're here, check if it's blank
        if line_idx == 0 {
            if let Some(line) = buffer.line(0) {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    // Line 0 is non-blank, we've gone as far back as we can
                    // The paragraph starts at line 0
                }
                // If line 0 is blank, line_idx is already 0
            }
        }

        buffer.cursor_mut().set_position(line_idx, 0);
    }

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
        let col = cursor.col();
        let total_lines = buffer.line_count();

        // Get text from current position onwards
        let mut current_line = line_idx;
        let mut current_col = col + 1;

        // Look for sentence-ending punctuation (.!?) followed by space/newline
        while current_line < total_lines {
            if let Some(line) = buffer.line(current_line) {
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
                                    buffer.cursor_mut().set_position(current_line + 1, 0);
                                } else {
                                    buffer.cursor_mut().set_position(
                                        current_line,
                                        chars.len().saturating_sub(1).max(0),
                                    );
                                }
                            } else {
                                buffer.cursor_mut().set_position(current_line, current_col);
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
        buffer.cursor_mut().set_position(last_line, 0);
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
        let mut col = cursor.col();

        if col == 0 && line_idx == 0 {
            return;
        }

        // Move back one position
        if col > 0 {
            col -= 1;
        } else if line_idx > 0 {
            line_idx -= 1;
            if let Some(line) = buffer.line(line_idx) {
                col = line
                    .trim_end_matches('\n')
                    .chars()
                    .count()
                    .saturating_sub(1);
            }
        }

        // Look for sentence-ending punctuation (.!?) followed by space/newline
        loop {
            if let Some(line) = buffer.line(line_idx) {
                let chars: Vec<char> = line.chars().collect();

                if chars.is_empty() {
                    // Empty line — skip to previous line
                    if line_idx == 0 {
                        buffer.cursor_mut().set_position(0, 0);
                        return;
                    }
                    line_idx -= 1;
                    if let Some(prev_line) = buffer.line(line_idx) {
                        col = prev_line
                            .trim_end_matches('\n')
                            .chars()
                            .count()
                            .saturating_sub(1);
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
                            buffer.cursor_mut().set_position(line_idx + 1, 0);
                        } else {
                            buffer
                                .cursor_mut()
                                .set_position(line_idx, col.min(chars.len().saturating_sub(1)));
                        }
                        return;
                    }
                    col = col.saturating_sub(1);
                }
            }

            if line_idx == 0 {
                buffer.cursor_mut().set_position(0, 0);
                return;
            }

            line_idx -= 1;
            if let Some(line) = buffer.line(line_idx) {
                col = line
                    .trim_end_matches('\n')
                    .chars()
                    .count()
                    .saturating_sub(1);
            }
        }
    }

    /// Moves cursor to the top of the visible screen (H command)
    /// viewport_start: first visible line
    /// viewport_height: number of visible lines
    /// offset: optional offset from top (0 = first line, 1 = second line, etc.)
    pub fn move_to_screen_top(buffer: &mut Buffer, viewport_start: usize, offset: usize) {
        let target_line = (viewport_start + offset).min(buffer.line_count().saturating_sub(1));

        // Move to first non-blank character on the line
        if let Some(line) = buffer.line(target_line) {
            let line_text = line.trim_end_matches('\n');
            let first_non_blank = line_text
                .chars()
                .position(|c| !c.is_whitespace())
                .unwrap_or(0);

            buffer
                .cursor_mut()
                .set_position(target_line, first_non_blank);
        }
    }

    /// Moves cursor to the middle of the visible screen (M command)
    /// viewport_start: first visible line
    /// viewport_height: number of visible lines
    pub fn move_to_screen_middle(
        buffer: &mut Buffer,
        viewport_start: usize,
        viewport_height: usize,
    ) {
        let middle_offset = viewport_height / 2;
        let target_line =
            (viewport_start + middle_offset).min(buffer.line_count().saturating_sub(1));

        // Move to first non-blank character on the line
        if let Some(line) = buffer.line(target_line) {
            let line_text = line.trim_end_matches('\n');
            let first_non_blank = line_text
                .chars()
                .position(|c| !c.is_whitespace())
                .unwrap_or(0);

            buffer
                .cursor_mut()
                .set_position(target_line, first_non_blank);
        }
    }

    /// Moves cursor to the bottom of the visible screen (L command)
    /// viewport_start: first visible line
    /// viewport_height: number of visible lines
    /// offset: optional offset from bottom (0 = last line, 1 = second to last, etc.)
    pub fn move_to_screen_bottom(
        buffer: &mut Buffer,
        viewport_start: usize,
        viewport_height: usize,
        offset: usize,
    ) {
        let last_visible = viewport_start + viewport_height.saturating_sub(1);
        let target_line = last_visible
            .saturating_sub(offset)
            .min(buffer.line_count().saturating_sub(1));

        // Move to first non-blank character on the line
        if let Some(line) = buffer.line(target_line) {
            let line_text = line.trim_end_matches('\n');
            let first_non_blank = line_text
                .chars()
                .position(|c| !c.is_whitespace())
                .unwrap_or(0);

            buffer
                .cursor_mut()
                .set_position(target_line, first_non_blank);
        }
    }

    /// Scrolls viewport down one line (Ctrl-E)
    /// Returns new viewport_start and whether cursor needs adjustment
    pub fn scroll_down_line(
        buffer: &Buffer,
        viewport_start: usize,
        viewport_height: usize,
        count: usize,
    ) -> (usize, bool) {
        let max_scroll = buffer.line_count().saturating_sub(viewport_height);
        let new_viewport = (viewport_start + count).min(max_scroll);

        // Check if cursor would be above viewport
        let cursor_line = buffer.cursor().line();
        let needs_cursor_adjustment = cursor_line < new_viewport;

        (new_viewport, needs_cursor_adjustment)
    }

    /// Scrolls viewport up one line (Ctrl-Y)
    /// Returns new viewport_start and whether cursor needs adjustment
    pub fn scroll_up_line(
        buffer: &Buffer,
        viewport_start: usize,
        viewport_height: usize,
        count: usize,
    ) -> (usize, bool) {
        let new_viewport = viewport_start.saturating_sub(count);

        // Check if cursor would be below viewport
        let cursor_line = buffer.cursor().line();
        let last_visible = new_viewport + viewport_height.saturating_sub(1);
        let needs_cursor_adjustment = cursor_line > last_visible;

        (new_viewport, needs_cursor_adjustment)
    }

    /// Scrolls down half a page (Ctrl-D)
    /// Returns new viewport_start and moves cursor
    pub fn scroll_half_page_down(
        buffer: &mut Buffer,
        viewport_start: usize,
        viewport_height: usize,
    ) -> usize {
        let half_page = viewport_height / 2;
        let max_scroll = buffer.line_count().saturating_sub(viewport_height);
        let new_viewport = (viewport_start + half_page).min(max_scroll);

        // Move cursor down by the same amount
        let cursor_line = buffer.cursor().line();
        let new_cursor_line = (cursor_line + half_page).min(buffer.line_count().saturating_sub(1));

        // Keep cursor in same column if possible
        let col = buffer.cursor().col();
        buffer.cursor_mut().set_position(new_cursor_line, col);

        // Adjust column to be within line bounds
        if let Some(line) = buffer.line(new_cursor_line) {
            let line_len = grapheme_count(line.trim_end_matches('\n'));
            if line_len > 0 {
                let clamped_col = col.min(line_len.saturating_sub(1));
                buffer.cursor_mut().set_col(clamped_col);
            } else {
                buffer.cursor_mut().set_col(0);
            }
        }

        new_viewport
    }

    /// Scrolls up half a page (Ctrl-U)
    /// Returns new viewport_start and moves cursor
    pub fn scroll_half_page_up(
        buffer: &mut Buffer,
        viewport_start: usize,
        viewport_height: usize,
    ) -> usize {
        let half_page = viewport_height / 2;
        let new_viewport = viewport_start.saturating_sub(half_page);

        // Move cursor up by the same amount
        let cursor_line = buffer.cursor().line();
        let new_cursor_line = cursor_line.saturating_sub(half_page);

        // Keep cursor in same column if possible
        let col = buffer.cursor().col();
        buffer.cursor_mut().set_position(new_cursor_line, col);

        // Adjust column to be within line bounds
        if let Some(line) = buffer.line(new_cursor_line) {
            let line_len = grapheme_count(line.trim_end_matches('\n'));
            if line_len > 0 {
                let clamped_col = col.min(line_len.saturating_sub(1));
                buffer.cursor_mut().set_col(clamped_col);
            } else {
                buffer.cursor_mut().set_col(0);
            }
        }

        new_viewport
    }

    /// Scrolls forward (down) one full page (Ctrl-F / Page Down)
    /// Returns new viewport_start and moves cursor
    pub fn scroll_page_down(
        buffer: &mut Buffer,
        viewport_start: usize,
        viewport_height: usize,
    ) -> usize {
        let max_scroll = buffer.line_count().saturating_sub(viewport_height);
        let new_viewport = (viewport_start + viewport_height.saturating_sub(2)).min(max_scroll);

        // Move cursor down by the same amount (keep relative position in viewport)
        let cursor_line = buffer.cursor().line();
        let cursor_offset = cursor_line.saturating_sub(viewport_start);
        let new_cursor_line =
            (new_viewport + cursor_offset).min(buffer.line_count().saturating_sub(1));

        // Keep cursor in same column if possible
        let col = buffer.cursor().col();
        buffer.cursor_mut().set_position(new_cursor_line, col);

        // Adjust column to be within line bounds
        if let Some(line) = buffer.line(new_cursor_line) {
            let line_len = grapheme_count(line.trim_end_matches('\n'));
            if line_len > 0 {
                let clamped_col = col.min(line_len.saturating_sub(1));
                buffer.cursor_mut().set_col(clamped_col);
            } else {
                buffer.cursor_mut().set_col(0);
            }
        }

        new_viewport
    }

    /// Scrolls backward (up) one full page (Ctrl-B / Page Up)
    /// Returns new viewport_start and moves cursor
    pub fn scroll_page_up(
        buffer: &mut Buffer,
        viewport_start: usize,
        viewport_height: usize,
    ) -> usize {
        let scroll_amount = viewport_height.saturating_sub(2);
        let new_viewport = viewport_start.saturating_sub(scroll_amount);

        // Move cursor up by the same amount (keep relative position in viewport)
        let cursor_line = buffer.cursor().line();
        let cursor_offset = cursor_line.saturating_sub(viewport_start);
        let new_cursor_line = new_viewport + cursor_offset;

        // Keep cursor in same column if possible
        let col = buffer.cursor().col();
        buffer.cursor_mut().set_position(new_cursor_line, col);

        // Adjust column to be within line bounds
        if let Some(line) = buffer.line(new_cursor_line) {
            let line_len = grapheme_count(line.trim_end_matches('\n'));
            if line_len > 0 {
                let clamped_col = col.min(line_len.saturating_sub(1));
                buffer.cursor_mut().set_col(clamped_col);
            } else {
                buffer.cursor_mut().set_col(0);
            }
        }

        new_viewport
    }

    /// Section navigation: jump to next section start (`{` at column 0)
    /// `]]` motion in Vim
    pub fn section_forward(buffer: &mut Buffer, count: usize) {
        let total_lines = buffer.line_count();
        let mut current_line = buffer.cursor().line();

        for _ in 0..count {
            current_line += 1;
            while current_line < total_lines {
                if let Some(line) = buffer.line(current_line) {
                    if line.starts_with('{') {
                        break;
                    }
                }
                current_line += 1;
            }
            if current_line >= total_lines {
                current_line = total_lines.saturating_sub(1);
                break;
            }
        }

        buffer.cursor_mut().set_position(current_line, 0);
    }

    /// Section navigation: jump to previous section start (`{` at column 0)
    /// `[[` motion in Vim
    pub fn section_backward(buffer: &mut Buffer, count: usize) {
        let mut current_line = buffer.cursor().line();

        for _ in 0..count {
            if current_line == 0 {
                break;
            }
            current_line -= 1;
            while current_line > 0 {
                if let Some(line) = buffer.line(current_line) {
                    if line.starts_with('{') {
                        break;
                    }
                }
                current_line -= 1;
            }
            // Check if line 0 is a match
            if current_line == 0 {
                if let Some(line) = buffer.line(0) {
                    if !line.starts_with('{') {
                        // No match found, stay at line 0
                    }
                }
            }
        }

        buffer.cursor_mut().set_position(current_line, 0);
    }

    /// Section navigation: jump to next section end (`}` at column 0)
    /// `][` motion in Vim
    pub fn section_end_forward(buffer: &mut Buffer, count: usize) {
        let total_lines = buffer.line_count();
        let mut current_line = buffer.cursor().line();

        for _ in 0..count {
            current_line += 1;
            while current_line < total_lines {
                if let Some(line) = buffer.line(current_line) {
                    if line.starts_with('}') {
                        break;
                    }
                }
                current_line += 1;
            }
            if current_line >= total_lines {
                current_line = total_lines.saturating_sub(1);
                break;
            }
        }

        buffer.cursor_mut().set_position(current_line, 0);
    }

    /// Section navigation: jump to previous section end (`}` at column 0)
    /// `[]` motion in Vim
    pub fn section_end_backward(buffer: &mut Buffer, count: usize) {
        let mut current_line = buffer.cursor().line();

        for _ in 0..count {
            if current_line == 0 {
                break;
            }
            current_line -= 1;
            while current_line > 0 {
                if let Some(line) = buffer.line(current_line) {
                    if line.starts_with('}') {
                        break;
                    }
                }
                current_line -= 1;
            }
        }

        buffer.cursor_mut().set_position(current_line, 0);
    }

    /// Jump to enclosing `{` brace
    /// `[{` motion in Vim
    pub fn jump_to_enclosing_open_brace(buffer: &mut Buffer) -> bool {
        let rope = buffer.rope();
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        // Convert to absolute position
        let text = rope.to_string();
        let chars: Vec<char> = text.chars().collect();

        let mut abs_pos = 0;
        for i in 0..line_idx {
            if let Some(line) = buffer.line(i) {
                abs_pos += line.chars().count();
            }
        }
        abs_pos += col;

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
                        let (new_line, new_col) = Self::abs_pos_to_line_col(rope, search_pos);
                        buffer.cursor_mut().set_position(new_line, new_col);
                        return true;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }

        // Check position 0
        if chars[0] == '{' && depth == 0 {
            buffer.cursor_mut().set_position(0, 0);
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

        // Convert to absolute position
        let text = rope.to_string();
        let chars: Vec<char> = text.chars().collect();

        let mut abs_pos = 0;
        for i in 0..line_idx {
            if let Some(line) = buffer.line(i) {
                abs_pos += line.chars().count();
            }
        }
        abs_pos += col;

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
                        let (new_line, new_col) = Self::abs_pos_to_line_col(rope, search_pos);
                        buffer.cursor_mut().set_position(new_line, new_col);
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

        // Convert to absolute position
        let text = rope.to_string();
        let chars: Vec<char> = text.chars().collect();

        let mut abs_pos = 0;
        for i in 0..line_idx {
            if let Some(line) = buffer.line(i) {
                abs_pos += line.chars().count();
            }
        }
        abs_pos += col;

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
                            let (new_line, new_col) = Self::abs_pos_to_line_col(rope, search_pos);
                            buffer.cursor_mut().set_position(new_line, new_col);
                            return true;
                        }
                        depth -= 1;
                    }
                    _ => {}
                }
            }

            // Check position 0
            if chars[0] == open_char && depth == 0 {
                buffer.cursor_mut().set_position(0, 0);
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
                            let (new_line, new_col) = Self::abs_pos_to_line_col(rope, search_pos);
                            buffer.cursor_mut().set_position(new_line, new_col);
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

    /// Method navigation: jump to next method/function start
    /// `]m` motion in Vim
    /// Looks for patterns like: fn name(, def name(, function name(, etc.
    pub fn method_forward(buffer: &mut Buffer, count: usize) {
        let total_lines = buffer.line_count();
        let mut current_line = buffer.cursor().line();

        for _ in 0..count {
            current_line += 1;
            while current_line < total_lines {
                if Self::is_method_start(buffer, current_line) {
                    break;
                }
                current_line += 1;
            }
            if current_line >= total_lines {
                current_line = total_lines.saturating_sub(1);
                break;
            }
        }

        // Position cursor at first non-whitespace (strip trailing newline so it
        // isn't counted as whitespace — buffer.line() includes the '\n')
        if let Some(line) = buffer.line(current_line) {
            let col = line
                .trim_end_matches('\n')
                .chars()
                .take_while(|c| c.is_whitespace())
                .count();
            buffer.cursor_mut().set_position(current_line, col);
        } else {
            buffer.cursor_mut().set_position(current_line, 0);
        }
    }

    /// Method navigation: jump to previous method/function start
    /// `[m` motion in Vim
    pub fn method_backward(buffer: &mut Buffer, count: usize) {
        let mut current_line = buffer.cursor().line();

        for _ in 0..count {
            if current_line == 0 {
                break;
            }
            current_line -= 1;
            while current_line > 0 {
                if Self::is_method_start(buffer, current_line) {
                    break;
                }
                current_line -= 1;
            }
            // Check line 0
            if current_line == 0 && !Self::is_method_start(buffer, 0) {
                // No match found, stay at line 0
            }
        }

        // Position cursor at first non-whitespace (strip trailing newline so it
        // isn't counted as whitespace — buffer.line() includes the '\n')
        if let Some(line) = buffer.line(current_line) {
            let col = line
                .trim_end_matches('\n')
                .chars()
                .take_while(|c| c.is_whitespace())
                .count();
            buffer.cursor_mut().set_position(current_line, col);
        } else {
            buffer.cursor_mut().set_position(current_line, 0);
        }
    }

    /// Method navigation: jump to next method/function end
    /// `]M` motion in Vim
    pub fn method_end_forward(buffer: &mut Buffer, count: usize) {
        let total_lines = buffer.line_count();
        let mut current_line = buffer.cursor().line();

        for _ in 0..count {
            current_line += 1;
            while current_line < total_lines {
                if Self::is_method_end(buffer, current_line) {
                    break;
                }
                current_line += 1;
            }
            if current_line >= total_lines {
                current_line = total_lines.saturating_sub(1);
                break;
            }
        }

        // Position at the closing brace.
        // Use char_to_grapheme_col because set_position expects a grapheme index,
        // and chars().position() gives a char index (not byte offset like str::find).
        if let Some(line) = buffer.line(current_line) {
            let char_col = line.chars().position(|c| c == '}').unwrap_or(0);
            let grapheme_col = crate::unicode::char_to_grapheme_col(&line, char_col);
            buffer.cursor_mut().set_position(current_line, grapheme_col);
        } else {
            buffer.cursor_mut().set_position(current_line, 0);
        }
    }

    /// Method navigation: jump to previous method/function end
    /// `[M` motion in Vim
    pub fn method_end_backward(buffer: &mut Buffer, count: usize) {
        let mut current_line = buffer.cursor().line();

        for _ in 0..count {
            if current_line == 0 {
                break;
            }
            current_line -= 1;
            while current_line > 0 {
                if Self::is_method_end(buffer, current_line) {
                    break;
                }
                current_line -= 1;
            }
        }

        // Position at the closing brace (same conversion as method_end_forward)
        if let Some(line) = buffer.line(current_line) {
            let char_col = line.chars().position(|c| c == '}').unwrap_or(0);
            let grapheme_col = crate::unicode::char_to_grapheme_col(&line, char_col);
            buffer.cursor_mut().set_position(current_line, grapheme_col);
        } else {
            buffer.cursor_mut().set_position(current_line, 0);
        }
    }

    /// Check if a line is the start of a method/function
    fn is_method_start(buffer: &Buffer, line_idx: usize) -> bool {
        if let Some(line) = buffer.line(line_idx) {
            let trimmed = line.trim();
            // Common function definition patterns
            // Rust: fn name(, pub fn name(, async fn name(
            // Python: def name(
            // JavaScript/TypeScript: function name(, async function name(
            // C/C++/Java: type name(, void name(, int name(, etc.

            // Check for Rust-style fn
            if trimmed.contains("fn ") && trimmed.contains('(') {
                return true;
            }
            // Check for Python-style def
            if trimmed.starts_with("def ") && trimmed.contains('(') {
                return true;
            }
            // Check for JavaScript/TypeScript function
            if trimmed.contains("function ") && trimmed.contains('(') {
                return true;
            }
            // Check for C/C++/Java/Go style - identifier followed by ( at start of line
            // Look for pattern: word word( or word( at reasonable indent level
            let indent = line.len() - line.trim_start().len();
            if indent <= 8
                && trimmed.contains('(')
                && !trimmed.starts_with("if ")
                && !trimmed.starts_with("for ")
                && !trimmed.starts_with("while ")
                && !trimmed.starts_with("switch ")
                && !trimmed.starts_with("match ")
                && !trimmed.starts_with("return ")
                && !trimmed.starts_with("//")
                && !trimmed.starts_with("/*")
                && !trimmed.starts_with("*")
            {
                // Check if line ends with { or has { after )
                if trimmed.ends_with('{') || (trimmed.contains(") {") || trimmed.contains("){")) {
                    return true;
                }
                // Check if next line starts with {
                if let Some(next_line) = buffer.line(line_idx + 1) {
                    if next_line.trim().starts_with('{') {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Check if a line is the end of a method/function
    /// A method end is a closing } at low indentation
    fn is_method_end(buffer: &Buffer, line_idx: usize) -> bool {
        if let Some(line) = buffer.line(line_idx) {
            let trimmed = line.trim();
            let indent = line.len() - line.trim_start().len();
            // A method end is typically a } at indentation <= 4 (or 8 for nested classes)
            // and the line is just the closing brace (possibly with semicolon for C++)
            if indent <= 4 && (trimmed == "}" || trimmed == "};" || trimmed == "},") {
                return true;
            }
        }
        false
    }
}
