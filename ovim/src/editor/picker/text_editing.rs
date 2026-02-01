use super::result::PickerField;
use super::Picker;

impl Picker {
    /// Returns mutable references to the active field's text and cursor
    fn active_field_mut(&mut self) -> (&mut String, &mut usize) {
        match self.active_field {
            PickerField::Query => (&mut self.query, &mut self.query_cursor),
            PickerField::FileFilter => (&mut self.file_filter, &mut self.file_filter_cursor),
        }
    }

    fn char_pos_to_byte_pos_in(s: &str, char_pos: usize) -> usize {
        s.char_indices()
            .nth(char_pos)
            .map(|(byte_pos, _)| byte_pos)
            .unwrap_or(s.len())
    }

    /// Inserts a character at the cursor position in the active field
    pub fn insert_char(&mut self, ch: char) {
        let (text, cursor) = self.active_field_mut();
        let byte_pos = Self::char_pos_to_byte_pos_in(text, *cursor);
        text.insert(byte_pos, ch);
        *cursor += 1;
        self.mark_filter_pending();
    }

    /// Inserts a string at the cursor position in the active field
    pub fn insert_text(&mut self, s: &str) {
        let (text, cursor) = self.active_field_mut();
        let byte_pos = Self::char_pos_to_byte_pos_in(text, *cursor);
        text.insert_str(byte_pos, s);
        *cursor += s.chars().count();
        self.mark_filter_pending();
    }

    /// Appends a character to the query (legacy method, inserts at cursor)
    pub fn append_query(&mut self, ch: char) {
        self.insert_char(ch);
    }

    /// Removes the character before the cursor in the active field
    pub fn backspace_query(&mut self) {
        let (text, cursor) = self.active_field_mut();
        if *cursor > 0 {
            let byte_pos = Self::char_pos_to_byte_pos_in(text, *cursor - 1);
            text.remove(byte_pos);
            *cursor -= 1;
        } else {
            return;
        }
        self.mark_filter_pending();
    }

    /// Removes the character at the cursor in the active field (delete key)
    pub fn delete_char(&mut self) {
        let (text, cursor) = self.active_field_mut();
        let char_len = text.chars().count();
        if *cursor < char_len {
            let byte_pos = Self::char_pos_to_byte_pos_in(text, *cursor);
            text.remove(byte_pos);
        } else {
            return;
        }
        self.mark_filter_pending();
    }

    /// Moves cursor left in the active field
    pub fn move_cursor_left(&mut self) {
        let (_text, cursor) = self.active_field_mut();
        if *cursor > 0 {
            *cursor -= 1;
        }
    }

    /// Moves cursor right in the active field
    pub fn move_cursor_right(&mut self) {
        let (text, cursor) = self.active_field_mut();
        let char_len = text.chars().count();
        if *cursor < char_len {
            *cursor += 1;
        }
    }

    /// Moves cursor to the beginning of the active field
    pub fn move_cursor_home(&mut self) {
        let (_text, cursor) = self.active_field_mut();
        *cursor = 0;
    }

    /// Moves cursor to the end of the active field
    pub fn move_cursor_end(&mut self) {
        let (text, cursor) = self.active_field_mut();
        *cursor = text.chars().count();
    }
}
