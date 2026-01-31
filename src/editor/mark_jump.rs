use super::{Editor, FindDirection, FindType, TagEntry};
use crate::editor::MarkManager;

impl Editor {
    /// Sets a mark at the current cursor position
    pub fn set_mark(&mut self, name: char) -> bool {
        let cursor_line = self.buffer().cursor().line();
        let cursor_col = self.buffer().cursor().col();
        let file_path = self.buffer().file_path().map(|s| s.to_string());
        self.nav.marks
            .set_mark(name, cursor_line, cursor_col, file_path.as_deref())
    }

    /// Jumps to a mark (exact position with backtick)
    pub fn jump_to_mark(&mut self, name: char) -> bool {
        // Try local mark first (a-z)
        if name.is_ascii_lowercase() {
            if let Some(mark) = self.nav.marks.get_mark(name) {
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(mark.line, mark.col);
                // Center cursor after jump (Vim behavior)
                self.center_cursor_in_viewport();
                return true;
            }
        }

        // Try global mark (A-Z)
        if name.is_ascii_uppercase() {
            if let Some(global_mark) = self.nav.marks.get_global_mark(name).cloned() {
                // Load the file if it's different from current file
                let current_file = self.buffer().file_path().map(|s| s.to_string());
                if current_file.as_deref() != Some(&global_mark.file_path)
                    && self.load_file(&global_mark.file_path).is_err()
                {
                    return false; // Failed to load file
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
                // Center cursor after jump (Vim behavior)
                self.center_cursor_in_viewport();
                return true;
            }
        }

        false
    }

    /// Jumps to mark line (apostrophe - goes to first non-blank on line)
    pub fn jump_to_mark_line(&mut self, name: char) -> bool {
        // Try local mark first (a-z)
        if name.is_ascii_lowercase() {
            if let Some(mark) = self.nav.marks.get_mark(name) {
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
                // Center cursor after jump (Vim behavior)
                self.center_cursor_in_viewport();
                return true;
            }
        }

        // Try global mark (A-Z)
        if name.is_ascii_uppercase() {
            if let Some(global_mark) = self.nav.marks.get_global_mark(name).cloned() {
                // Load the file if it's different from current file
                let current_file = self.buffer().file_path().map(|s| s.to_string());
                if current_file.as_deref() != Some(&global_mark.file_path)
                    && self.load_file(&global_mark.file_path).is_err()
                {
                    return false; // Failed to load file
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
                // Center cursor after jump (Vim behavior)
                self.center_cursor_in_viewport();
                return true;
            }
        }

        false
    }

    /// Adds current position to jump list
    pub fn add_jump(&mut self) {
        let cursor = self.buffer().cursor();
        self.nav.jump_list.add_jump(cursor.line(), cursor.col());
    }

    /// Jumps back in the jump list (Ctrl-O)
    pub fn jump_back(&mut self) -> bool {
        if let Some((line, col)) = self.nav.jump_list.jump_back() {
            self.buffer_mut().cursor_mut().set_position(line, col);
            // Center cursor after jump (Vim behavior)
            self.center_cursor_in_viewport();
            true
        } else {
            false
        }
    }

    /// Jumps forward in the jump list (Ctrl-I)
    pub fn jump_forward(&mut self) -> bool {
        if let Some((line, col)) = self.nav.jump_list.jump_forward() {
            self.buffer_mut().cursor_mut().set_position(line, col);
            // Center cursor after jump (Vim behavior)
            self.center_cursor_in_viewport();
            true
        } else {
            false
        }
    }

    /// Sets the last find motion for ; and , repeat
    pub fn set_last_find(&mut self, ch: char, find_type: FindType, direction: FindDirection) {
        self.nav.last_find = Some((ch, find_type, direction));
    }

    /// Gets the last find motion
    pub fn get_last_find(&self) -> Option<(char, FindType, FindDirection)> {
        self.nav.last_find
    }

    /// Gets the mark manager (for reading marks)
    pub fn marks(&self) -> &MarkManager {
        &self.nav.marks
    }

    /// Pushes current position to tag stack before LSP navigation (gd/gD/gy)
    /// Called just before jumping to definition/implementation/type
    pub fn push_tag(&mut self) {
        if let Some(file_path) = self.buffer().file_path().map(|s| s.to_string()) {
            let cursor = self.buffer().cursor();
            let entry = TagEntry::new(file_path, cursor.line(), cursor.col());
            self.nav.tag_stack.push(entry);
        }
    }

    /// Pops from tag stack and navigates to that location (Ctrl-T)
    /// Returns true if successfully jumped, false if stack was empty or navigation failed
    pub fn tag_pop(&mut self) -> bool {
        if let Some(entry) = self.nav.tag_stack.pop() {
            // Check if we need to switch files
            let current_file = self.buffer().file_path().map(|s| s.to_string());
            let needs_file_switch = current_file.as_deref() != Some(&entry.file_path);

            if needs_file_switch {
                // Load the target file
                if self.load_file(&entry.file_path).is_err() {
                    self.set_lsp_status(format!("Tag pop failed: cannot load {}", entry.file_path));
                    return false;
                }
            }

            // Validate and clamp position to buffer bounds
            let max_line = self.buffer().line_count().saturating_sub(1);
            let clamped_line = entry.line.min(max_line);

            let line_len = if let Some(line) = self.buffer().line(clamped_line) {
                line.trim_end_matches('\n').chars().count()
            } else {
                0
            };
            let clamped_col = entry.col.min(line_len.saturating_sub(1));

            // Jump to the position
            self.buffer_mut()
                .cursor_mut()
                .set_position(clamped_line, clamped_col);

            // Center cursor in viewport (like jump_back does)
            self.center_cursor_in_viewport();

            // Trigger re-render
            self.mark_dirty();

            // Show status
            let remaining = self.nav.tag_stack.len();
            let file_name = std::path::Path::new(&entry.file_path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&entry.file_path);
            self.set_lsp_status(format!(
                "Tag: {}:{}:{} ({} remaining)",
                file_name,
                clamped_line + 1,
                clamped_col + 1,
                remaining
            ));

            true
        } else {
            self.set_lsp_status("Tag stack empty".to_string());
            false
        }
    }

    /// Returns the number of entries in the tag stack
    pub fn tag_stack_len(&self) -> usize {
        self.nav.tag_stack.len()
    }

    /// Returns true if the tag stack is empty
    pub fn tag_stack_is_empty(&self) -> bool {
        self.nav.tag_stack.is_empty()
    }
}
