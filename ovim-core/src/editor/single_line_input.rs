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
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self { text, cursor }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
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

    pub fn backspace(&mut self) -> bool {
        let Some(previous) = self.previous_boundary() else {
            return false;
        };
        self.text.remove(previous);
        self.cursor = previous;
        true
    }

    pub fn delete(&mut self) -> bool {
        if self.cursor >= self.text.len() {
            return false;
        }
        self.text.remove(self.cursor);
        true
    }

    pub fn move_left(&mut self) -> bool {
        let Some(previous) = self.previous_boundary() else {
            return false;
        };
        self.cursor = previous;
        true
    }

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

    pub fn move_home(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor = 0;
        true
    }

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
    fn home_end_and_newline_policy_are_explicit() {
        let mut input = SingleLineInput::new("text");
        assert!(!input.insert('\n'));
        assert_eq!(input.text(), "text");
        assert!(input.move_home());
        assert_eq!(input.cursor(), 0);
        assert!(input.move_end());
        assert_eq!(input.cursor(), input.text().len());
    }
}
