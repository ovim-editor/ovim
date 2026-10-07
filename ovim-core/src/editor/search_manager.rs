use super::{Editor, Operator};
use crate::editor::Search;
use crate::unicode::{char_to_grapheme_col, grapheme_count, grapheme_to_char_col, GraphemeCol};

impl Editor {
    /// Gets the search buffer
    pub fn search_buffer(&self) -> &str {
        self.search.search_input.text()
    }

    /// Gets the search cursor as a UTF-8 byte offset.
    pub fn search_cursor(&self) -> usize {
        self.search.search_input.cursor()
    }

    /// Clears the search buffer
    pub fn clear_search_buffer(&mut self) {
        self.search.search_input.clear();
    }

    /// Inserts a character at the search cursor.
    pub fn insert_search_char(&mut self, ch: char) -> bool {
        self.search.search_input.insert(ch)
    }

    /// Inserts text at the search cursor.
    pub fn insert_into_search_buffer(&mut self, text: &str) -> bool {
        self.search.search_input.insert_str(text)
    }

    /// Removes the last character from the search buffer
    pub fn backspace_search_buffer(&mut self) -> bool {
        self.search.search_input.backspace()
    }

    /// Removes everything before the search cursor (`CTRL-U`).
    pub fn delete_search_to_start(&mut self) -> bool {
        self.search.search_input.delete_to_start()
    }

    /// Removes the word before the search cursor (`CTRL-W`).
    pub fn delete_search_word(&mut self) -> bool {
        self.search.search_input.delete_word_backward()
    }

    /// Removes the character at the search cursor.
    pub fn delete_from_search_buffer(&mut self) -> bool {
        self.search.search_input.delete()
    }

    /// Moves the search cursor one character left.
    pub fn move_search_cursor_left(&mut self) {
        self.search.search_input.move_left();
    }

    /// Moves the search cursor one character right.
    pub fn move_search_cursor_right(&mut self) {
        self.search.search_input.move_right();
    }

    /// Moves the search cursor to the beginning of the query.
    pub fn move_search_cursor_home(&mut self) {
        self.search.search_input.move_home();
    }

    /// Moves the search cursor to the end of the query.
    pub fn move_search_cursor_end(&mut self) {
        self.search.search_input.move_end();
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

    /// Makes `pattern` the last search pattern, as `:s` and `:g` do: the `/`
    /// register, `n`/`N` and the highlight all follow it, and the direction
    /// of the last `/` or `?` is kept.
    pub(crate) fn set_last_search_pattern(&mut self, pattern: &str) {
        self.registers.set_last_search(pattern.to_string());
        self.search.current_search = Some(Search::new_with_options(
            pattern.to_string(),
            self.search.search_forward,
            self.options.ignorecase,
            self.options.smartcase,
        ));
    }

    /// Enters Search mode for `/` (forward) or `?`: remembers the cursor to search
    /// from and to restore on Esc, and the count typed before the key.
    pub fn begin_search(&mut self, forward: bool) {
        let count = self.input.count;
        self.clear_search_buffer();
        self.set_search_forward(forward);
        self.save_search_start_position();
        self.set_mode(crate::mode::Mode::Search);
        self.search.search_count = count;
    }

    /// `d/` and friends: like [`Self::begin_search`], with `operator` applied to the
    /// text between where the search began and the match.
    pub fn begin_search_with_operator(&mut self, forward: bool, operator: Operator) {
        self.begin_search(forward);
        self.search.search_operator = Some(operator);
    }

    /// The operator waiting on the running search, if any.
    pub fn take_search_operator(&mut self) -> Option<Operator> {
        self.search.search_operator.take()
    }

    /// Where the running search began.
    pub fn search_origin(&self) -> Option<(usize, usize)> {
        self.search.search_start_pos
    }

    /// Shows where the pattern typed so far would take the cursor, searching
    /// from where the search began and without recording the pattern.
    pub fn preview_search(&mut self) {
        let _ = self.run_search(false);
    }

    /// Executes the typed pattern (`<CR>`), moves to the match and records the
    /// pattern as the last search.
    pub fn execute_search(&mut self) -> bool {
        let found = self.run_search(true);
        self.search.search_start_pos = None;
        self.search.search_count = None;
        found
    }

    /// Returns whether the pattern matched.
    fn run_search(&mut self, commit: bool) -> bool {
        // An empty pattern (`/<CR>` or `?<CR>`) repeats the last search in the
        // requested direction (Vim behavior), rather than wiping the active
        // search. Only fall back to clearing when there is no last search.
        let pattern = if self.search.search_input.is_empty() {
            let last = self.registers.get_last_search().to_string();
            if last.is_empty() {
                self.clear_search_highlight();
                self.restore_search_start_position();
                return false;
            }
            last
        } else {
            let query = self.search.search_input.text().to_owned();
            if commit {
                // Update the / register with the search pattern
                self.registers.set_last_search(query.clone());
            }
            query
        };

        let mut search = Search::new_with_options(
            pattern,
            self.search.search_forward,
            self.options.ignorecase,
            self.options.smartcase,
        );
        // The search always starts where it began, not where the preview of
        // the previous keystroke left the cursor.
        let cursor = self.buffer().cursor();
        let origin = self
            .search
            .search_start_pos
            .filter(|_| self.mode() == crate::mode::Mode::Search)
            .unwrap_or((cursor.line(), cursor.col().0));
        let count = self.search.search_count.unwrap_or(1);

        let found = match self.step_search(&mut search, origin, count) {
            Some((line, col)) => {
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(line, GraphemeCol(col));
                true
            }
            None => {
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(origin.0, GraphemeCol(origin.1));
                false
            }
        };
        // Always update current_search so highlighting reflects the actual pattern.
        // If no match exists, find_all_in_line will return empty for each line,
        // so stale highlights from a previous partial match won't linger.
        self.search.current_search = Some(search);
        found
    }

    /// Where `count` steps of `search` (in its own direction) lead from `origin`:
    /// each step is the first match strictly after (forward) or before (backward)
    /// the previous position, wrapping around the buffer.
    pub(crate) fn step_search(
        &self,
        search: &mut Search,
        origin: (usize, usize),
        count: usize,
    ) -> Option<(usize, usize)> {
        let mut position = origin;
        for _ in 0..count.max(1) {
            let from_col = if search.is_forward() {
                position.1 + 1
            } else {
                position.1
            };
            let (line, col, _) =
                search.find_next(self.buffer(), position.0, GraphemeCol(from_col))?;
            position = (line, col);
        }
        Some(position)
    }

    /// Finds the next search match (n command)
    pub fn search_next(&mut self) {
        self.repeat_search(false);
    }

    /// Finds the previous search match (N command)
    pub fn search_prev(&mut self) {
        self.repeat_search(true);
    }

    /// `n` / `N`: `[count]` matches on in the direction of the last search (or
    /// against it when `reverse`).
    fn repeat_search(&mut self, reverse: bool) {
        let count = self.effective_count();
        self.clear_count();
        if let Some((line, col)) = self.search_target(reverse, count) {
            self.buffer_mut()
                .cursor_mut()
                .set_position(line, GraphemeCol(col));
        }
    }

    /// Where `[count]` `n` (or `N` when `reverse`) would take the cursor, or
    /// `None` without a previous search or a match.
    pub(crate) fn search_target(&self, reverse: bool, count: usize) -> Option<(usize, usize)> {
        let search = self.search.current_search.as_ref()?;
        let mut search = if reverse {
            Search::new_with_options(
                search.pattern().to_string(),
                !search.is_forward(),
                self.options.ignorecase,
                self.options.smartcase,
            )
        } else {
            search.clone()
        };
        let cursor = self.buffer().cursor();
        self.step_search(&mut search, (cursor.line(), cursor.col().0), count)
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
                if let Some(line_text) = self.buffer().line_text(cursor_line) {
                    let cursor_char_col =
                        grapheme_to_char_col(&line_text, GraphemeCol(cursor_col)).0;
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
                                &line_text,
                                crate::unicode::CharCol(*start_col),
                            );
                            let end_grapheme = char_to_grapheme_col(
                                &line_text,
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
                if let Some(line_text) = self.buffer().line_text(cursor_line) {
                    let cursor_char_col =
                        grapheme_to_char_col(&line_text, GraphemeCol(cursor_col)).0;
                    let matches = rev_search.find_all_in_line(&line_text);
                    if let Some((start_col, _end_col)) = matches
                        .iter()
                        .find(|(start, end)| cursor_char_col >= *start && cursor_char_col < *end)
                    {
                        // Cursor is inside a match — search from before this match's start
                        // Convert char-based start_col to grapheme for the search_col
                        let start_grapheme =
                            char_to_grapheme_col(&line_text, crate::unicode::CharCol(*start_col));
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
