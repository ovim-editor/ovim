use super::result::PickerField;
use super::Picker;
use crate::editor::SingleLineInput;

impl Picker {
    /// Returns the active text field.
    fn active_field_mut(&mut self) -> &mut SingleLineInput {
        match self.active_field {
            PickerField::Query => &mut self.query,
            PickerField::FileFilter => &mut self.file_filter,
        }
    }

    /// Inserts a character at the cursor position in the active field
    pub fn insert_char(&mut self, ch: char) {
        if self.active_field_mut().insert(ch) {
            self.mark_filter_pending();
        }
    }

    /// Inserts a string at the cursor position in the active field
    pub fn insert_text(&mut self, s: &str) {
        if self.active_field_mut().insert_str(s) {
            self.mark_filter_pending();
        }
    }

    /// Appends a character to the query (legacy method, inserts at cursor)
    pub fn append_query(&mut self, ch: char) {
        self.insert_char(ch);
    }

    /// Removes the character before the cursor in the active field
    pub fn backspace_query(&mut self) {
        if self.active_field_mut().backspace() {
            self.mark_filter_pending();
        }
    }

    /// Removes the character at the cursor in the active field (delete key)
    pub fn delete_char(&mut self) {
        if self.active_field_mut().delete() {
            self.mark_filter_pending();
        }
    }

    /// Moves cursor left in the active field
    pub fn move_cursor_left(&mut self) {
        self.active_field_mut().move_left();
    }

    /// Moves cursor right in the active field
    pub fn move_cursor_right(&mut self) {
        self.active_field_mut().move_right();
    }

    /// Moves cursor to the beginning of the active field
    pub fn move_cursor_home(&mut self) {
        self.active_field_mut().move_home();
    }

    /// Moves cursor to the end of the active field
    pub fn move_cursor_end(&mut self) {
        self.active_field_mut().move_end();
    }
}
