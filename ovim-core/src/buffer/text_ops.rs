use super::Buffer;
use crate::edit::Edit;

impl Buffer {
    /// Inserts text at a specific position (line, col)
    pub fn insert_text_at(&mut self, line: usize, col: usize, text: &str) {
        // Track buffer edit metrics
        crate::metrics::BUFFER_EDITS_TOTAL.inc();

        // Use raw_line_count() to allow inserting at the phantom empty line
        // (which is valid for appending at end of buffer)
        if line >= self.raw_line_count() {
            return;
        }

        let line_start = self.rope.line_to_char(line);
        let insert_pos = line_start + col;

        // Clamp to valid position
        let insert_pos = insert_pos.min(self.rope.len_chars());

        // Create tree-sitter edit BEFORE modifying rope (needs old state)
        let ts_edit = self.create_ts_insert_edit(line, col, text);

        // Shift highlights BEFORE modifying rope
        self.shift_highlights_for_insertion(line, col, text);

        self.rope.insert(insert_pos, text);
        self.modified = true;

        // Record the edit if we're in a recording session
        if let Some(ref mut edits) = self.recording {
            edits.push(Edit::Insert {
                offset: insert_pos,
                text: text.to_string(),
            });
        }

        // Update buffer size metrics
        crate::metrics::BUFFER_SIZE_BYTES.set(self.rope.len_bytes() as i64);
        crate::metrics::BUFFER_LINES.set(self.rope.len_lines() as i64);

        // Apply incremental tree-sitter edit (much faster than full re-parse)
        if let Some(edit) = ts_edit {
            self.apply_incremental_syntax_edit(edit);
        }

        // Increment versions for cache invalidation
        self.version += 1;
        self.highlight_version = self.highlight_version.wrapping_add(1);
        self.pending_rehighlight = true;
    }

    /// Deletes text in a range and returns the deleted text
    pub fn delete_range(
        &mut self,
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
    ) -> String {
        // Track buffer edit metrics
        crate::metrics::BUFFER_EDITS_TOTAL.inc();

        if start_line >= self.line_count() {
            return String::new();
        }

        // Validate start column is within line bounds to prevent addition overflow
        let start_line_len = self.line_len(start_line);
        let actual_start_col = start_col.min(start_line_len);

        let start_line_char = self.rope.line_to_char(start_line);
        let start_pos = start_line_char + actual_start_col;

        // Calculate actual end position and column
        let (end_pos, actual_end_line, actual_end_col) = if end_line >= self.line_count() {
            (
                self.rope.len_chars(),
                self.line_count().saturating_sub(1),
                self.line_len(self.line_count().saturating_sub(1)),
            )
        } else {
            // Validate end column is within line bounds to prevent addition overflow
            let end_line_len = self.line_len(end_line);
            let actual_end_col = end_col.min(end_line_len);

            let end_line_char = self.rope.line_to_char(end_line);
            (end_line_char + actual_end_col, end_line, actual_end_col)
        };

        // Final safety clamp to buffer length (should be redundant after column validation)
        let start_pos = start_pos.min(self.rope.len_chars());
        let end_pos = end_pos.min(self.rope.len_chars());

        if start_pos >= end_pos {
            return String::new();
        }

        let deleted = self.rope.slice(start_pos..end_pos).to_string();

        // Create tree-sitter edit BEFORE modifying rope (needs old state)
        let ts_edit = self.create_ts_delete_edit(
            start_line,
            actual_start_col,
            actual_end_line,
            actual_end_col,
            &deleted,
        );

        // Shift highlights BEFORE modifying rope
        self.shift_highlights_for_deletion(start_line, start_col, end_line, end_col);

        self.rope.remove(start_pos..end_pos);
        self.modified = true;

        // Record the edit if we're in a recording session
        if let Some(ref mut edits) = self.recording {
            edits.push(Edit::Delete {
                offset: start_pos,
                text: deleted.clone(),
            });
        }

        // Update buffer size metrics
        crate::metrics::BUFFER_SIZE_BYTES.set(self.rope.len_bytes() as i64);
        crate::metrics::BUFFER_LINES.set(self.rope.len_lines() as i64);

        // Apply incremental tree-sitter edit (much faster than full re-parse)
        if let Some(edit) = ts_edit {
            self.apply_incremental_syntax_edit(edit);
        }

        // Increment versions for cache invalidation
        self.version += 1;
        self.highlight_version = self.highlight_version.wrapping_add(1);
        self.pending_rehighlight = true;

        deleted
    }

    /// Deletes text by absolute char positions in the rope.
    ///
    /// Unlike `delete_range` which clamps columns to `line_len()` (excluding
    /// newlines), this method uses the char indices directly. This is needed
    /// for undo of insertions at positions past the content of a line (e.g.,
    /// after the newline character).
    pub fn delete_char_range(&mut self, start_char: usize, end_char: usize) {
        crate::metrics::BUFFER_EDITS_TOTAL.inc();

        let start_pos = start_char.min(self.rope.len_chars());
        let end_pos = end_char.min(self.rope.len_chars());

        if start_pos >= end_pos {
            return;
        }

        let start_line = self.rope.char_to_line(start_pos);
        let start_col = start_pos - self.rope.line_to_char(start_line);
        let end_line = self.rope.char_to_line(end_pos);
        let end_col = end_pos - self.rope.line_to_char(end_line);

        let deleted = self.rope.slice(start_pos..end_pos).to_string();

        let ts_edit =
            self.create_ts_delete_edit(start_line, start_col, end_line, end_col, &deleted);
        self.shift_highlights_for_deletion(start_line, start_col, end_line, end_col);

        self.rope.remove(start_pos..end_pos);
        self.modified = true;

        // Record the edit if we're in a recording session
        if let Some(ref mut edits) = self.recording {
            edits.push(Edit::Delete {
                offset: start_pos,
                text: deleted,
            });
        }

        crate::metrics::BUFFER_SIZE_BYTES.set(self.rope.len_bytes() as i64);
        crate::metrics::BUFFER_LINES.set(self.rope.len_lines() as i64);

        if let Some(edit) = ts_edit {
            self.apply_incremental_syntax_edit(edit);
        }

        self.version += 1;
        self.highlight_version = self.highlight_version.wrapping_add(1);
        self.pending_rehighlight = true;
    }

    /// Replaces the entire buffer content
    pub fn replace_all(&mut self, content: &str) {
        self.rope = ropey::Rope::from_str(content);
        self.modified = true;
        // Reset cursor to beginning
        self.cursor = super::Cursor::new(0, 0);
        // Increment version for cache invalidation
        self.version += 1;
    }

    /// Joins the current line with the next line (J command)
    /// Adds a space between the lines unless the current line already ends with whitespace
    pub fn join_lines(&mut self, count: usize) -> anyhow::Result<()> {
        self.join_lines_impl(count, true)
    }

    /// Joins lines without adding a space (gJ command)
    pub fn join_lines_no_space(&mut self, count: usize) -> anyhow::Result<()> {
        self.join_lines_impl(count, false)
    }

    /// Internal implementation for joining lines
    fn join_lines_impl(&mut self, count: usize, add_space: bool) -> anyhow::Result<()> {
        let start_line = self.cursor.line();

        // Join 'count' times (count = 1 means join current with next)
        let lines_to_join = count.max(1);

        for _ in 0..lines_to_join {
            if start_line >= self.line_count().saturating_sub(1) {
                // Already at the last line, nothing to join
                break;
            }

            // Get the current line and next line
            let current_line_text = match self.line(start_line) {
                Some(text) => text.trim_end_matches('\n').to_string(),
                None => break,
            };

            let next_line_text = match self.line(start_line + 1) {
                Some(text) => text.trim_end_matches('\n').to_string(),
                None => break,
            };

            // Determine if we need to add a space
            let separator = if add_space {
                // Add space unless current line ends with whitespace
                if current_line_text.ends_with(|c: char| c.is_whitespace()) {
                    ""
                } else {
                    " "
                }
            } else {
                ""
            };

            // Trim leading whitespace from next line
            let next_trimmed = next_line_text.trim_start();

            // Build the joined line
            let joined = if next_trimmed.is_empty() {
                // Next line is all whitespace, just use current line
                current_line_text.clone()
            } else {
                format!("{}{}{}", current_line_text, separator, next_trimmed)
            };

            // Delete both lines (from start_line to start_line+2)
            self.delete_range(start_line, 0, start_line + 2, 0);

            // Insert the joined line with newline
            self.insert_text_at(start_line, 0, &format!("{}\n", joined));

            // Fix: Position cursor at the junction point (end of original first line)
            // This is where the separator (space) was inserted
            let junction_col = current_line_text.len();
            self.cursor.set_position(start_line, junction_col);
        }

        Ok(())
    }

    /// Gets the word under the cursor
    /// Returns the word and its (start_col, end_col) on the current line
    pub fn word_under_cursor(&self) -> Option<(String, usize, usize)> {
        let line_idx = self.cursor.line();
        let col = self.cursor.col();

        if line_idx >= self.line_count() {
            return None;
        }

        // Use rope slice to avoid allocation until we need the final word
        let line_slice = self.line_slice(line_idx)?;

        // Build a chars vector from the slice (excluding newline)
        let chars: Vec<char> = line_slice.chars().take_while(|&c| c != '\n').collect();

        if chars.is_empty() || col >= chars.len() {
            return None;
        }

        // Check if cursor is on a word character
        let is_word_char = |c: char| c.is_alphanumeric() || c == '_';

        if !is_word_char(chars[col]) {
            return None;
        }

        // Find start of word
        let mut start = col;
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }

        // Find end of word
        let mut end = col;
        while end < chars.len() && is_word_char(chars[end]) {
            end += 1;
        }

        // Only allocate String for the final word
        let word: String = chars[start..end].iter().collect();
        Some((word, start, end))
    }
}
