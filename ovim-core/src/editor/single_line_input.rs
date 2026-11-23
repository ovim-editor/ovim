/// UTF-8-safe state for an editable single-line text field.
///
/// The cursor is stored as a byte offset so it can be used directly with Rust
/// strings, but every mutation keeps it on a character boundary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SingleLineInput {
    text: String,
    cursor: usize,
}

impl SingleLineInput {
    /// Create an input with the cursor at the end, discarding line breaks.
    pub fn new(text: impl Into<String>) -> Self {
        let mut text = text.into();
        text.retain(|character| !matches!(character, '\n' | '\r'));
        let cursor = text.len();
        Self { text, cursor }
    }

    /// Return the input text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Return the cursor as a UTF-8 byte offset.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Return whether the input contains no text.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Remove all text and reset the cursor to the beginning.
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// Insert a printable character at the cursor. Newline characters are
    /// ignored because this type deliberately models a single-line field.
    pub fn insert(&mut self, character: char) -> bool {
        if matches!(character, '\n' | '\r') {
            return false;
        }
        self.text.insert(self.cursor, character);
        self.cursor += character.len_utf8();
        true
    }

    /// Insert text at the cursor, ignoring line breaks.
    pub fn insert_str(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        if text.contains(['\n', '\r']) {
            let filtered: String = text
                .chars()
                .filter(|character| !matches!(character, '\n' | '\r'))
                .collect();
            if filtered.is_empty() {
                return false;
            }
            self.text.insert_str(self.cursor, &filtered);
            self.cursor += filtered.len();
        } else {
            self.text.insert_str(self.cursor, text);
            self.cursor += text.len();
        }
        true
    }

    /// Remove the character before the cursor.
    pub fn backspace(&mut self) -> bool {
        let Some(previous) = self.previous_boundary() else {
            return false;
        };
        self.text.remove(previous);
        self.cursor = previous;
        true
    }

    /// Remove the character at the cursor.
    pub fn delete(&mut self) -> bool {
        if self.cursor >= self.text.len() {
            return false;
        }
        self.text.remove(self.cursor);
        true
    }

    /// Remove everything before the cursor (Vim cmdline `CTRL-U`).
    pub fn delete_to_start(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.text.drain(..self.cursor);
        self.cursor = 0;
        true
    }

    /// Remove the word before the cursor (Vim cmdline `CTRL-W`): trailing
    /// whitespace first, then one run of keyword or of other non-blank
    /// characters.
    pub fn delete_word_backward(&mut self) -> bool {
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let before = &self.text[..self.cursor];
        let mut start = self.cursor;
        let mut chars = before.char_indices().rev().peekable();
        while let Some(&(index, c)) = chars.peek() {
            if !c.is_whitespace() {
                break;
            }
            start = index;
            chars.next();
        }
        if let Some(&(_, first)) = chars.peek() {
            let class = is_word(first);
            while let Some(&(index, c)) = chars.peek() {
                if c.is_whitespace() || is_word(c) != class {
                    break;
                }
                start = index;
                chars.next();
            }
        }
        if start == self.cursor {
            return false;
        }
        self.text.drain(start..self.cursor);
        self.cursor = start;
        true
    }

    /// Move the cursor one character left.
    pub fn move_left(&mut self) -> bool {
        let Some(previous) = self.previous_boundary() else {
            return false;
        };
        self.cursor = previous;
        true
    }

    /// Move the cursor one character right.
    pub fn move_right(&mut self) -> bool {
        if self.cursor >= self.text.len() {
            return false;
        }
        self.cursor = self.text[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(index, _)| self.cursor + index)
            .unwrap_or(self.text.len());
        true
    }

    /// Move the cursor to the beginning.
    pub fn move_home(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor = 0;
        true
    }

    /// Move the cursor to the end.
    pub fn move_end(&mut self) -> bool {
        if self.cursor == self.text.len() {
            return false;
        }
        self.cursor = self.text.len();
        true
    }

    fn previous_boundary(&self) -> Option<usize> {
        if self.cursor == 0 {
            return None;
        }
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(index, _)| index)
    }
}

#[cfg(test)]
mod tests {
    use super::SingleLineInput;

    #[test]
    fn edits_unicode_without_leaving_character_boundaries() {
        let mut input = SingleLineInput::new("a🙂z");
        assert_eq!(input.cursor(), 6);

        assert!(input.move_left());
        assert_eq!(input.cursor(), 5);
        assert!(input.backspace());
        assert_eq!(input.text(), "az");
        assert_eq!(input.cursor(), 1);
        assert!(input.insert('é'));
        assert_eq!(input.text(), "aéz");
        assert_eq!(input.cursor(), 3);
        assert!(input.delete());
        assert_eq!(input.text(), "aé");
    }

    #[test]
    fn boundary_moves_and_deletes_are_noops() {
        let mut input = SingleLineInput::default();
        assert!(!input.move_left());
        assert!(!input.move_right());
        assert!(!input.backspace());
        assert!(!input.delete());
        assert!(!input.move_home());
        assert!(!input.move_end());
    }

    #[test]
    fn delete_to_start_keeps_the_text_after_the_cursor() {
        let mut input = SingleLineInput::new("hello world");
        for _ in 0..5 {
            input.move_left();
        }
        assert!(input.delete_to_start());
        assert_eq!(input.text(), "world");
        assert_eq!(input.cursor(), 0);
        assert!(!input.delete_to_start());
    }

    /// Matches Vim's cmdline CTRL-W (checked against `nvim --headless`).
    #[test]
    fn delete_word_backward_follows_vim_word_classes() {
        let cases = [
            ("foo bar", "foo "),
            ("foo bar  ", "foo "),
            ("foo.bar", "foo."),
            ("foo.", "foo"),
            ("foo..", "foo"),
            ("héllo wörld", "héllo "),
            ("   ", ""),
            ("word", ""),
        ];
        for (before, after) in cases {
            let mut input = SingleLineInput::new(before);
            assert!(input.delete_word_backward(), "{before:?}");
            assert_eq!(input.text(), after, "{before:?}");
            assert_eq!(input.cursor(), after.len());
        }
        let mut empty = SingleLineInput::default();
        assert!(!empty.delete_word_backward());
    }

    #[test]
    fn home_end_and_newline_policy_are_explicit() {
        let mut input = SingleLineInput::new("text");
        assert!(!input.insert('\n'));
        assert_eq!(input.text(), "text");
        assert!(input.move_home());
        assert_eq!(input.cursor(), 0);
        assert!(input.move_end());
        assert_eq!(input.cursor(), input.text().len());
    }

    #[test]
    fn inserting_text_filters_line_breaks_and_clear_resets_the_cursor() {
        let mut input = SingleLineInput::new("a\n");
        assert_eq!(input.text(), "a");
        assert!(input.insert_str("b\nc\r"));
        assert_eq!(input.text(), "abc");
        assert_eq!(input.cursor(), 3);

        input.clear();
        assert!(input.is_empty());
        assert_eq!(input.cursor(), 0);
    }
}
