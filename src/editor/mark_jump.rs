use super::{Editor, FindDirection, FindType};
use crate::editor::MarkManager;

impl Editor {
    /// Sets a mark at the current cursor position
    pub fn set_mark(&mut self, name: char) -> bool {
        let cursor_line = self.buffer().cursor().line();
        let cursor_col = self.buffer().cursor().col();
        let file_path = self.buffer().file_path().map(|s| s.to_string());
        self.marks
            .set_mark(name, cursor_line, cursor_col, file_path.as_deref())
    }

    /// Jumps to a mark (exact position with backtick)
    pub fn jump_to_mark(&mut self, name: char) -> bool {
        // Try local mark first (a-z)
        if name.is_ascii_lowercase() {
            if let Some(mark) = self.marks.get_mark(name) {
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(mark.line, mark.col);
                return true;
            }
        }

        // Try global mark (A-Z)
        if name.is_ascii_uppercase() {
            if let Some(global_mark) = self.marks.get_global_mark(name).cloned() {
                // Load the file if it's different from current file
                let current_file = self.buffer().file_path().map(|s| s.to_string());
                if current_file.as_deref() != Some(&global_mark.file_path) {
                    // Load the file (synchronously for now)
                    if let Ok(_) = self.load_file(&global_mark.file_path) {
                        // File loaded successfully
                    } else {
                        return false; // Failed to load file
                    }
                }

                // Validate and clamp mark position to buffer bounds
                let max_line = self.buffer().line_count().saturating_sub(1);
                let clamped_line = global_mark.line.min(max_line);

                let line_len = if let Some(line) = self.buffer().line(clamped_line) {
                    line.trim_end_matches('\n').chars().count()
                } else {
                    0
                };
                let clamped_col = global_mark.col.min(line_len);

                // Jump to the validated position
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(clamped_line, clamped_col);
                return true;
            }
        }

        false
    }

    /// Jumps to mark line (apostrophe - goes to first non-blank on line)
    pub fn jump_to_mark_line(&mut self, name: char) -> bool {
        // Try local mark first (a-z)
        if name.is_ascii_lowercase() {
            if let Some(mark) = self.marks.get_mark(name) {
                // Find first non-blank character on the line
                let first_non_blank = if let Some(line_text) = self.buffer().line(mark.line) {
                    line_text
                        .chars()
                        .position(|c| !c.is_whitespace())
                        .unwrap_or(0)
                } else {
                    0
                };

                self.buffer_mut()
                    .cursor_mut()
                    .set_position(mark.line, first_non_blank);
                return true;
            }
        }

        // Try global mark (A-Z)
        if name.is_ascii_uppercase() {
            if let Some(global_mark) = self.marks.get_global_mark(name).cloned() {
                // Load the file if it's different from current file
                let current_file = self.buffer().file_path().map(|s| s.to_string());
                if current_file.as_deref() != Some(&global_mark.file_path) {
                    // Load the file (synchronously for now)
                    if let Ok(_) = self.load_file(&global_mark.file_path) {
                        // File loaded successfully
                    } else {
                        return false; // Failed to load file
                    }
                }

                // Validate and clamp mark line to buffer bounds
                let max_line = self.buffer().line_count().saturating_sub(1);
                let clamped_line = global_mark.line.min(max_line);

                // Find first non-blank character on the line
                let first_non_blank = if let Some(line_text) = self.buffer().line(clamped_line) {
                    line_text
                        .chars()
                        .position(|c| !c.is_whitespace())
                        .unwrap_or(0)
                } else {
                    0
                };

                self.buffer_mut()
                    .cursor_mut()
                    .set_position(clamped_line, first_non_blank);
                return true;
            }
        }

        false
    }

    /// Adds current position to jump list
    pub fn add_jump(&mut self) {
        let cursor = self.buffer().cursor();
        self.jump_list.add_jump(cursor.line(), cursor.col());
    }

    /// Jumps back in the jump list (Ctrl-O)
    pub fn jump_back(&mut self) -> bool {
        if let Some((line, col)) = self.jump_list.jump_back() {
            self.buffer_mut().cursor_mut().set_position(line, col);
            true
        } else {
            false
        }
    }

    /// Jumps forward in the jump list (Ctrl-I)
    pub fn jump_forward(&mut self) -> bool {
        if let Some((line, col)) = self.jump_list.jump_forward() {
            self.buffer_mut().cursor_mut().set_position(line, col);
            true
        } else {
            false
        }
    }

    /// Sets the last find motion for ; and , repeat
    pub fn set_last_find(&mut self, ch: char, find_type: FindType, direction: FindDirection) {
        self.last_find = Some((ch, find_type, direction));
    }

    /// Gets the last find motion
    pub fn get_last_find(&self) -> Option<(char, FindType, FindDirection)> {
        self.last_find
    }

    /// Gets the mark manager (for reading marks)
    pub fn marks(&self) -> &MarkManager {
        &self.marks
    }
}
