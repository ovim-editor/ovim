use super::Editor;

impl Editor {
    /// Gets the command line buffer
    pub fn command_line(&self) -> &str {
        self.command.input.text()
    }

    /// Gets the command-line cursor position (byte index)
    pub fn command_cursor(&self) -> usize {
        self.command.input.cursor()
    }

    /// Clears the command line buffer
    pub fn clear_command_line(&mut self) {
        self.command.input.clear();
        self.command.command_history_index = None;
    }

    /// Inserts a character at the command-line cursor
    pub fn append_to_command_line(&mut self, ch: char) {
        if self.command.input.insert(ch) {
            self.command.command_history_index = None;
        }
    }

    /// Inserts text at the command-line cursor
    pub fn insert_into_command_line(&mut self, text: &str) {
        if self.command.input.insert_str(text) {
            self.command.command_history_index = None;
        }
    }

    /// Removes the character before the command-line cursor
    pub fn backspace_command_line(&mut self) {
        if self.command.input.backspace() {
            self.command.command_history_index = None;
        }
    }

    /// Removes the character at the command-line cursor
    pub fn delete_command_line_char(&mut self) {
        if self.command.input.delete() {
            self.command.command_history_index = None;
        }
    }

    /// Moves command-line cursor one character left
    pub fn move_command_cursor_left(&mut self) {
        self.command.input.move_left();
    }

    /// Moves command-line cursor one character right
    pub fn move_command_cursor_right(&mut self) {
        self.command.input.move_right();
    }

    /// Moves command-line cursor to start
    pub fn move_command_cursor_home(&mut self) {
        self.command.input.move_home();
    }

    /// Moves command-line cursor to end
    pub fn move_command_cursor_end(&mut self) {
        self.command.input.move_end();
    }

    /// Replaces the entire command line content
    pub fn set_command_line(&mut self, cmd: &str) {
        self.command.input = super::SingleLineInput::new(cmd);
    }

    /// Adds current command to history
    pub fn add_command_to_history(&mut self) {
        let cmd = self.command.input.text().trim().to_string();
        if !cmd.is_empty() {
            // Don't add duplicate if it's the same as the last command
            if self.command.command_history.last() != Some(&cmd) {
                self.command.command_history.push(cmd);
                // Limit history size to 100 commands
                if self.command.command_history.len() > 100 {
                    self.command.command_history.drain(0..1);
                }
            }
        }
        self.command.command_history_index = None;
    }

    /// Navigate to previous command in history (up arrow)
    pub fn history_prev(&mut self) {
        if self.command.command_history.is_empty() {
            return;
        }

        let new_index = match self.command.command_history_index {
            None => {
                // First time pressing up - go to last command
                Some(self.command.command_history.len() - 1)
            }
            Some(idx) if idx > 0 => {
                // Go to previous command
                Some(idx - 1)
            }
            Some(_) => {
                // Already at oldest command
                return;
            }
        };

        if let Some(idx) = new_index {
            if let Some(cmd) = self.command.command_history.get(idx) {
                self.command.input = super::SingleLineInput::new(cmd);
                self.command.command_history_index = Some(idx);
            }
        }
    }

    /// Navigate to next command in history (down arrow)
    pub fn history_next(&mut self) {
        if self.command.command_history.is_empty() {
            return;
        }

        let new_index = match self.command.command_history_index {
            None => {
                // Not navigating history, do nothing
                return;
            }
            Some(idx) if idx < self.command.command_history.len() - 1 => {
                // Go to next command
                Some(idx + 1)
            }
            Some(_) => {
                // At newest command, clear to empty line
                self.command.input.clear();
                self.command.command_history_index = None;
                return;
            }
        };

        if let Some(idx) = new_index {
            if let Some(cmd) = self.command.command_history.get(idx) {
                self.command.input = super::SingleLineInput::new(cmd);
                self.command.command_history_index = Some(idx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Editor;

    #[test]
    fn command_editing_delegates_unicode_boundaries_to_the_input_state() {
        let mut editor = Editor::new();
        editor.set_command_line("a🙂z");

        editor.move_command_cursor_left();
        editor.backspace_command_line();
        editor.append_to_command_line('é');
        editor.delete_command_line_char();

        assert_eq!(editor.command_line(), "aé");
        assert_eq!(editor.command_cursor(), 3);
    }

    #[test]
    fn leaving_the_newest_history_entry_clears_text_and_cursor() {
        let mut editor = Editor::new();
        editor.set_command_line("write");
        editor.add_command_to_history();

        editor.history_prev();
        assert_eq!(editor.command_cursor(), 5);
        editor.history_next();

        assert!(editor.command_line().is_empty());
        assert_eq!(editor.command_cursor(), 0);
    }
}
