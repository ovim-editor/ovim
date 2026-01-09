use super::Editor;

impl Editor {
    /// Gets the command line buffer
    pub fn command_line(&self) -> &str {
        &self.command.command_line
    }

    /// Clears the command line buffer
    pub fn clear_command_line(&mut self) {
        self.command.command_line.clear();
    }

    /// Appends a character to the command line
    pub fn append_to_command_line(&mut self, ch: char) {
        self.command.command_line.push(ch);
    }

    /// Removes the last character from the command line
    pub fn backspace_command_line(&mut self) {
        self.command.command_line.pop();
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
                self.command.command_history_index = Some(idx);
            }
        }
    }
}
