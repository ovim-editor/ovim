use super::Editor;

impl Editor {
    /// Gets the command line buffer
    pub fn command_line(&self) -> &str {
        &self.command.command_line
    }

    /// Gets the command-line cursor position (byte index)
    pub fn command_cursor(&self) -> usize {
        self.command.command_cursor
    }

    /// Clears the command line buffer
    pub fn clear_command_line(&mut self) {
        self.command.command_line.clear();
        self.command.command_cursor = 0;
        self.command.command_history_index = None;
    }

    /// Inserts a character at the command-line cursor
    pub fn append_to_command_line(&mut self, ch: char) {
        let cursor = self.command.command_cursor.min(self.command.command_line.len());
        self.command.command_line.insert(cursor, ch);
        self.command.command_cursor = cursor + ch.len_utf8();
        self.command.command_history_index = None;
    }

    /// Inserts text at the command-line cursor
    pub fn insert_into_command_line(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let cursor = self.command.command_cursor.min(self.command.command_line.len());
        self.command.command_line.insert_str(cursor, text);
        self.command.command_cursor = cursor + text.len();
        self.command.command_history_index = None;
    }

    /// Removes the character before the command-line cursor
    pub fn backspace_command_line(&mut self) {
        let cursor = self.command.command_cursor.min(self.command.command_line.len());
        if cursor == 0 {
            return;
        }
        let prev = self.command.command_line[..cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.command.command_line.drain(prev..cursor);
        self.command.command_cursor = prev;
        self.command.command_history_index = None;
    }

    /// Removes the character at the command-line cursor
    pub fn delete_command_line_char(&mut self) {
        let cursor = self.command.command_cursor.min(self.command.command_line.len());
        if cursor >= self.command.command_line.len() {
            return;
        }
        let next = self.command.command_line[cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| cursor + i)
            .unwrap_or(self.command.command_line.len());
        self.command.command_line.drain(cursor..next);
        self.command.command_history_index = None;
    }

    /// Moves command-line cursor one character left
    pub fn move_command_cursor_left(&mut self) {
        let cursor = self.command.command_cursor.min(self.command.command_line.len());
        if cursor == 0 {
            return;
        }
        let prev = self.command.command_line[..cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.command.command_cursor = prev;
    }

    /// Moves command-line cursor one character right
    pub fn move_command_cursor_right(&mut self) {
        let cursor = self.command.command_cursor.min(self.command.command_line.len());
        if cursor >= self.command.command_line.len() {
            return;
        }
        let next = self.command.command_line[cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| cursor + i)
            .unwrap_or(self.command.command_line.len());
        self.command.command_cursor = next;
    }

    /// Moves command-line cursor to start
    pub fn move_command_cursor_home(&mut self) {
        self.command.command_cursor = 0;
    }

    /// Moves command-line cursor to end
    pub fn move_command_cursor_end(&mut self) {
        self.command.command_cursor = self.command.command_line.len();
    }

    /// Replaces the entire command line content
    pub fn set_command_line(&mut self, cmd: &str) {
        self.command.command_line = cmd.to_string();
        self.command.command_cursor = self.command.command_line.len();
    }

    /// Adds current command to history
    pub fn add_command_to_history(&mut self) {
        let cmd = self.command.command_line.trim().to_string();
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
                self.command.command_line = cmd.clone();
                self.command.command_cursor = self.command.command_line.len();
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
                self.command.command_line.clear();
                self.command.command_history_index = None;
                return;
            }
        };

        if let Some(idx) = new_index {
            if let Some(cmd) = self.command.command_history.get(idx) {
                self.command.command_line = cmd.clone();
                self.command.command_cursor = self.command.command_line.len();
                self.command.command_history_index = Some(idx);
            }
        }
    }
}
