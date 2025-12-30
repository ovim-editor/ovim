use super::Editor;
use crate::editor::Search;

impl Editor {
    /// Gets the search buffer
    pub fn search_buffer(&self) -> &str {
        &self.search_buffer
    }

    /// Clears the search buffer
    pub fn clear_search_buffer(&mut self) {
        self.search_buffer.clear();
    }

    /// Appends a character to the search buffer
    pub fn append_to_search_buffer(&mut self, ch: char) {
        self.search_buffer.push(ch);
    }

    /// Removes the last character from the search buffer
    pub fn backspace_search_buffer(&mut self) {
        self.search_buffer.pop();
    }

    /// Sets the search direction
    pub fn set_search_forward(&mut self, forward: bool) {
        self.search_forward = forward;
    }

    /// Gets the search direction
    pub fn search_forward(&self) -> bool {
        self.search_forward
    }

    /// Gets the current search
    pub fn current_search(&self) -> Option<&Search> {
        self.current_search.as_ref()
    }

    /// Sets the current search
    pub fn set_current_search(&mut self, search: Search) {
        self.current_search = Some(search);
    }

    /// Clears the current search (stops highlighting)
    pub fn clear_search_highlight(&mut self) {
        self.current_search = None;
    }

    /// Executes the current search and moves cursor to first match
    pub fn execute_search(&mut self) {
        if self.search_buffer.is_empty() {
            return;
        }

        // Update the / register with the search pattern
        self.registers.set_last_search(self.search_buffer.clone());

        let mut search = Search::new_with_options(
            self.search_buffer.clone(),
            self.search_forward,
            self.options.ignorecase,
            self.options.smartcase,
        );
        let cursor = self.buffer().cursor();

        // Start search from current cursor position (inclusive)
        if let Some((line, col, _)) = search.find_next(self.buffer(), cursor.line(), cursor.col()) {
            self.buffer_mut().cursor_mut().set_position(line, col);
            self.current_search = Some(search);
        }
    }

    /// Finds the next search match (n command)
    pub fn search_next(&mut self) {
        // Get cursor position before borrowing
        let cursor_line = self.buffer().cursor().line();
        let cursor_col = self.buffer().cursor().col();

        // Clone search to avoid borrow conflicts
        if let Some(ref search) = self.current_search {
            let is_forward = search.is_forward();
            let mut search_clone = search.clone();

            // For forward search, start from col+1; for backward, start from col-1 or col
            let search_col = if is_forward {
                cursor_col + 1
            } else {
                if cursor_col > 0 {
                    cursor_col - 1
                } else {
                    0
                }
            };

            if let Some((line, col, _)) =
                search_clone.find_next(self.buffer(), cursor_line, search_col)
            {
                self.buffer_mut().cursor_mut().set_position(line, col);
            }
        }
    }

    /// Finds the previous search match (N command)
    pub fn search_prev(&mut self) {
        if let Some(ref search) = self.current_search {
            // Create a reversed search
            let is_forward = search.is_forward();
            let mut rev_search = Search::new_with_options(
                search.pattern().to_string(),
                !is_forward,
                self.options.ignorecase,
                self.options.smartcase,
            );
            let cursor = self.buffer().cursor();

            // For reverse direction: if original was forward, now going backward (use col-1)
            // if original was backward, now going forward (use col+1)
            let search_col = if is_forward {
                // Original was forward, now backward
                if cursor.col() > 0 {
                    cursor.col() - 1
                } else {
                    0
                }
            } else {
                // Original was backward, now forward
                cursor.col() + 1
            };

            if let Some((line, col, _)) =
                rev_search.find_next(self.buffer(), cursor.line(), search_col)
            {
                self.buffer_mut().cursor_mut().set_position(line, col);
            }
        }
    }
}
