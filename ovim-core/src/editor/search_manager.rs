use super::Editor;
use crate::editor::Search;
use crate::unicode::{char_to_grapheme_col, grapheme_count, grapheme_to_char_col, GraphemeCol};

impl Editor {
    /// Gets the search buffer
    pub fn search_buffer(&self) -> &str {
        &self.search.search_buffer
    }

    /// Clears the search buffer
    pub fn clear_search_buffer(&mut self) {
        self.search.search_buffer.clear();
    }

    /// Appends a character to the search buffer
    pub fn append_to_search_buffer(&mut self, ch: char) {
        self.search.search_buffer.push(ch);
    }

    /// Removes the last character from the search buffer
    pub fn backspace_search_buffer(&mut self) {
        self.search.search_buffer.pop();
    }

    /// Sets the search direction
    pub fn set_search_forward(&mut self, forward: bool) {
        self.search.search_forward = forward;
    }

    /// Gets the search direction
    pub fn search_forward(&self) -> bool {
        self.search.search_forward
    }

    /// Saves the current cursor position when entering search mode
    /// This allows restoring the position if search is canceled with ESC
    pub fn save_search_start_position(&mut self) {
        let cursor = self.buffer().cursor();
        self.search.search_start_pos = Some((cursor.line(), cursor.col().0));
    }

    /// Restores the cursor to the position saved when search mode was entered
    /// Used when canceling search with ESC
    pub fn restore_search_start_position(&mut self) {
        if let Some((line, col)) = self.search.search_start_pos {
            self.buffer_mut()
                .cursor_mut()
                .set_position(line, GraphemeCol(col));
            self.search.search_start_pos = None;
        }
    }

    /// Gets the current search
    pub fn current_search(&self) -> Option<&Search> {
        self.search.current_search.as_ref()
    }

    /// Sets the current search
    pub fn set_current_search(&mut self, search: Search) {
        self.search.current_search = Some(search);
    }

    /// Clears the current search (stops highlighting)
    pub fn clear_search_highlight(&mut self) {
        self.search.current_search = None;
    }

    /// Executes the current search and moves cursor to first match
    pub fn execute_search(&mut self) {
        if self.search.search_buffer.is_empty() {
            // Clear search highlight and restore cursor position on empty search
            self.clear_search_highlight();
            self.restore_search_start_position();
            return;
        }

        // Update the / register with the search pattern
        self.registers
            .set_last_search(self.search.search_buffer.clone());

        let mut search = Search::new_with_options(
            self.search.search_buffer.clone(),
            self.search.search_forward,
            self.options.ignorecase,
            self.options.smartcase,
        );
        let cursor = self.buffer().cursor();

        // Start search from current cursor position (inclusive)
        if let Some((line, col, _)) = search.find_next(self.buffer(), cursor.line(), cursor.col()) {
            self.buffer_mut()
                .cursor_mut()
                .set_position(line, GraphemeCol(col));
        }
        // Always update current_search so highlighting reflects the actual pattern.
        // If no match exists, find_all_in_line will return empty for each line,
        // so stale highlights from a previous partial match won't linger.
        self.search.current_search = Some(search);
    }

    /// Finds the next search match (n command)
    pub fn search_next(&mut self) {
        // Get cursor position before borrowing
        let cursor_line = self.buffer().cursor().line();
        let cursor_col = self.buffer().cursor().col().0;

        // Clone search to avoid borrow conflicts
        if let Some(ref search) = self.search.current_search {
            let is_forward = search.is_forward();
            let mut search_clone = search.clone();

            // For forward search, start from col+1; for backward, start from col-1 or col
            // cursor_col is a grapheme index — use grapheme_count for line length comparison
            let (search_line, search_col) = if is_forward {
                if let Some(line) = self.buffer().line(cursor_line) {
                    let line_len = grapheme_count(line.trim_end_matches('\n'));
                    if cursor_col + 1 >= line_len {
                        // OV-00040: Advance to next line instead of col 0 of same line
                        let next_line = (cursor_line + 1) % self.buffer().line_count().max(1);
                        (next_line, 0)
                    } else {
                        (cursor_line, cursor_col + 1)
                    }
                } else {
                    (cursor_line, cursor_col + 1)
                }
            } else if cursor_col > 0 {
                (cursor_line, cursor_col - 1)
            } else {
                (cursor_line, 0)
            };

            if let Some((line, col, _)) =
                search_clone.find_next(self.buffer(), search_line, GraphemeCol(search_col))
            {
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(line, GraphemeCol(col));
            }
        }
    }

    /// Finds the previous search match (N command)
    pub fn search_prev(&mut self) {
        if let Some(ref search) = self.search.current_search {
            // Create a reversed search
            let is_forward = search.is_forward();
            let mut rev_search = Search::new_with_options(
                search.pattern().to_string(),
                !is_forward,
                self.options.ignorecase,
                self.options.smartcase,
            );
            let cursor_line = self.buffer().cursor().line();
            let cursor_col = self.buffer().cursor().col().0;

            // For reverse direction: if original was forward, now going backward (use col-1)
            // if original was backward, now going forward (use col+1)
            let (search_line, search_col) = if is_forward {
                // Original was forward, now backward
                if cursor_col > 0 {
                    (cursor_line, cursor_col - 1)
                } else {
                    (cursor_line, 0)
                }
            } else {
                // Original was backward, now forward - clamp to avoid exceeding line length
                // cursor_col is grapheme index — use grapheme_count for comparison
                if let Some(line) = self.buffer().line(cursor_line) {
                    let line_len = grapheme_count(line.trim_end_matches('\n'));
                    if cursor_col + 1 >= line_len {
                        // OV-00040: Advance to next line instead of col 0 of same line
                        let next_line = (cursor_line + 1) % self.buffer().line_count().max(1);
                        (next_line, 0)
                    } else {
                        (cursor_line, cursor_col + 1)
                    }
                } else {
                    (cursor_line, cursor_col + 1)
                }
            };

            if let Some((line, col, _)) =
                rev_search.find_next(self.buffer(), search_line, GraphemeCol(search_col))
            {
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(line, GraphemeCol(col));
            }
        }
    }

    /// Saves the visual search state when entering search from visual mode
    pub fn set_visual_search_state(&mut self, anchor: (usize, usize), mode: crate::mode::Mode) {
        self.search.visual_search_state = Some(crate::editor::VisualSearchState { anchor, mode });
    }

    /// Takes and clears the visual search state (returns None if not set)
    pub fn take_visual_search_state(&mut self) -> Option<crate::editor::VisualSearchState> {
        self.search.visual_search_state.take()
    }

    /// Finds the next search match and enters/extends visual mode (gn command)
    /// Returns true if a match was found
    #[must_use = "ignoring the return value means you won't know if the search succeeded"]
    pub fn search_select_next(&mut self) -> bool {
        use crate::mode::Mode;

        // Check if we have an active search
        let search_exists = self.search.current_search.is_some();
        if !search_exists {
            return false;
        }

        let cursor_line = self.buffer().cursor().line();
        let cursor_col = self.buffer().cursor().col().0;
        let mode = self.mode();
        let in_visual_mode =
            mode == Mode::Visual || mode == Mode::VisualLine || mode == Mode::VisualBlock;

        // Clone search to avoid borrow conflicts
        if let Some(ref search) = self.search.current_search {
            let mut search_clone = search.clone();

            // In normal mode, check if cursor is within a match at current position.
            // find_all_in_line returns char-based cols; cursor_col is grapheme-based.
            // Convert cursor_col → char for comparison, then char → grapheme for positions.
            if !in_visual_mode {
                if let Some(line_text) = self.buffer().line(cursor_line) {
                    let line_trimmed = line_text.trim_end_matches('\n');
                    let cursor_char_col =
                        grapheme_to_char_col(line_trimmed, GraphemeCol(cursor_col)).0;
                    let matches = search_clone.find_all_in_line(&line_text);
                    let cursor_in_match = matches.iter().any(|(start_col, end_col)| {
                        cursor_char_col >= *start_col && cursor_char_col < *end_col
                    });

                    if cursor_in_match {
                        // If cursor is within a match, select the current match
                        if let Some((start_col, end_col)) = matches.iter().find(|(start, end)| {
                            cursor_char_col >= *start && cursor_char_col < *end
                        }) {
                            // Convert char cols → grapheme for visual start and cursor
                            let start_grapheme = char_to_grapheme_col(
                                line_trimmed,
                                crate::unicode::CharCol(*start_col),
                            );
                            let end_grapheme = char_to_grapheme_col(
                                line_trimmed,
                                crate::unicode::CharCol(end_col - 1),
                            );
                            self.set_visual_start(cursor_line, start_grapheme.0);
                            self.buffer_mut()
                                .cursor_mut()
                                .set_position(cursor_line, end_grapheme);
                            self.set_mode(Mode::Visual);
                            return true;
                        }
                    }
                }
            }

            // Find the next match (always search from cursor + 1 to skip current position)
            let search_col = GraphemeCol(cursor_col + 1);
            if let Some((line, col, match_text)) =
                search_clone.find_next(self.buffer(), cursor_line, search_col)
            {
                // col is now grapheme-based (from find_next); use grapheme_count for match length
                let match_grapheme_len = grapheme_count(&match_text);
                let match_end = col + match_grapheme_len - 1;

                if in_visual_mode {
                    // In visual mode, extend selection to include the next match
                    self.buffer_mut()
                        .cursor_mut()
                        .set_position(line, GraphemeCol(match_end));
                } else {
                    // In normal mode, enter visual mode and select the next match
                    self.set_visual_start(line, col);
                    self.buffer_mut()
                        .cursor_mut()
                        .set_position(line, GraphemeCol(match_end));
                    self.set_mode(Mode::Visual);
                }
                return true;
            }
        }

        false
    }

    /// Finds the previous search match and enters/extends visual mode (gN command)
    /// Returns true if a match was found
    #[must_use = "ignoring the return value means you won't know if the search succeeded"]
    pub fn search_select_prev(&mut self) -> bool {
        use crate::mode::Mode;

        // Check if we have an active search
        let search_exists = self.search.current_search.is_some();
        if !search_exists {
            return false;
        }

        let cursor_line = self.buffer().cursor().line();
        let cursor_col = self.buffer().cursor().col().0;
        let mode = self.mode();
        let in_visual_mode =
            mode == Mode::Visual || mode == Mode::VisualLine || mode == Mode::VisualBlock;

        // Clone search to avoid borrow conflicts
        if let Some(ref search) = self.search.current_search {
            // Create a reversed search
            let is_forward = search.is_forward();
            let mut rev_search = Search::new_with_options(
                search.pattern().to_string(),
                !is_forward,
                self.options.ignorecase,
                self.options.smartcase,
            );

            // Find the previous match
            // If cursor is within a match, start searching from before that match.
            // find_all_in_line returns char-based cols; convert for cursor comparison.
            let search_col = if in_visual_mode {
                cursor_col
            } else {
                let mut col = if cursor_col > 0 { cursor_col - 1 } else { 0 };
                if let Some(line_text) = self.buffer().line(cursor_line) {
                    let line_trimmed = line_text.trim_end_matches('\n');
                    let cursor_char_col =
                        grapheme_to_char_col(line_trimmed, GraphemeCol(cursor_col)).0;
                    let matches = rev_search.find_all_in_line(&line_text);
                    if let Some((start_col, _end_col)) = matches
                        .iter()
                        .find(|(start, end)| cursor_char_col >= *start && cursor_char_col < *end)
                    {
                        // Cursor is inside a match — search from before this match's start
                        // Convert char-based start_col to grapheme for the search_col
                        let start_grapheme =
                            char_to_grapheme_col(line_trimmed, crate::unicode::CharCol(*start_col));
                        col = if start_grapheme.0 > 0 {
                            start_grapheme.0 - 1
                        } else {
                            0
                        };
                    }
                }
                col
            };
            if let Some((line, col, match_text)) =
                rev_search.find_next(self.buffer(), cursor_line, GraphemeCol(search_col))
            {
                // col is now grapheme-based (from find_next); use grapheme_count for match length
                let match_grapheme_len = grapheme_count(&match_text);
                let match_end = col + match_grapheme_len - 1;

                if in_visual_mode {
                    // In visual mode, extend selection to include the previous match
                    self.buffer_mut()
                        .cursor_mut()
                        .set_position(line, GraphemeCol(match_end));
                } else {
                    // In normal mode, enter visual mode and select the previous match
                    self.set_visual_start(line, col);
                    self.buffer_mut()
                        .cursor_mut()
                        .set_position(line, GraphemeCol(match_end));
                    self.set_mode(Mode::Visual);
                }
                return true;
            }
        }

        false
    }
}
