use super::{Editor, FindDirection, FindType, Motions, TagEntry};
use crate::buffer::Buffer;
use crate::editor::MarkManager;
use crate::unicode::GraphemeCol;

/// Clamps a stored mark/tag column to the cursor-on-char range of `line`.
///
/// Marks (and tag-stack entries) record absolute grapheme columns at set
/// time; lines edited afterward may shrink, leaving the column past the
/// last grapheme. Clamping to `len-1` (saturating to 0 on empty lines)
/// mirrors normal-mode cursor invariants. (OV-00190.)
fn clamp_grapheme_col(buffer: &Buffer, line_idx: usize, col: usize) -> usize {
    let line_len = buffer
        .line(line_idx)
        .map(|line| crate::unicode::grapheme_count(line.trim_end_matches('\n')))
        .unwrap_or(0);
    col.min(line_len.saturating_sub(1))
}

impl Editor {
    /// Sets a mark at the current cursor position
    pub fn set_mark(&mut self, name: char) -> bool {
        let cursor_line = self.buffer().cursor().line();
        let cursor_col = self.buffer().cursor().col().0;
        let file_path = self.buffer().file_path().map(|s| s.to_string());
        self.nav
            .marks
            .set_mark(name, cursor_line, cursor_col, file_path.as_deref())
    }

    /// Jumps to a mark (exact position with backtick)
    pub fn jump_to_mark(&mut self, name: char) -> bool {
        // Special exact-position marks
        match name {
            // `.` - last change position
            '.' => {
                if let Some(change) = self.last_change() {
                    let pos = change.cursor_after();
                    self.buffer_mut()
                        .cursor_mut()
                        .set_position(pos.line, pos.col.saturating_sub(1));
                    self.center_cursor_in_viewport();
                    return true;
                }
            }
            // `^ - last insert exit position (cursor-on-char semantics)
            '^' => {
                if let Some(change) = self.last_change() {
                    let inserted = change.get_inserted_text();
                    if !inserted.is_empty() {
                        let pos = change.cursor_after();
                        self.buffer_mut()
                            .cursor_mut()
                            .set_position(pos.line, pos.col.saturating_sub(1));
                        self.center_cursor_in_viewport();
                        return true;
                    }
                }

                if let Some((line, col)) = self.editing.last_insert_position {
                    self.buffer_mut()
                        .cursor_mut()
                        .set_position(line, GraphemeCol(col.saturating_sub(1)));
                    self.center_cursor_in_viewport();
                    return true;
                }
            }
            _ => {}
        }

        // Try local mark first (a-z)
        if name.is_ascii_lowercase() {
            if let Some(mark) = self.nav.marks.get_mark(name) {
                // OV-00190: clamp to current buffer bounds. The mark may point
                // past EOF if lines were deleted after it was set; uncapped,
                // `set_position` would land the cursor outside the rope and
                // subsequent motions / rendering would panic or display
                // garbage. Same shape as the global-mark branch below.
                let max_line = self.buffer().line_count().saturating_sub(1);
                let clamped_line = mark.line.min(max_line);
                let clamped_col = clamp_grapheme_col(self.buffer(), clamped_line, mark.col);

                self.buffer_mut()
                    .cursor_mut()
                    .set_position(clamped_line, GraphemeCol(clamped_col));
                // Center cursor after jump (Vim behavior)
                self.center_cursor_in_viewport();
                return true;
            }
        }

        // Try global mark (A-Z)
        if name.is_ascii_uppercase() {
            if let Some(global_mark) = self.nav.marks.get_global_mark(name).cloned() {
                // Load the file if mark targets a concrete path different from current.
                if let Some(mark_path) = global_mark.file_path.as_deref() {
                    let current_file = self.buffer().file_path().map(|s| s.to_string());
                    if current_file.as_deref() != Some(mark_path)
                        && self.load_file(mark_path).is_err()
                    {
                        return false; // Failed to load file
                    }
                }

                // Validate and clamp mark position to buffer bounds
                let max_line = self.buffer().line_count().saturating_sub(1);
                let clamped_line = global_mark.line.min(max_line);
                let clamped_col =
                    clamp_grapheme_col(self.buffer(), clamped_line, global_mark.col);

                // Jump to the validated position
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(clamped_line, GraphemeCol(clamped_col));
                // Center cursor after jump (Vim behavior)
                self.center_cursor_in_viewport();
                return true;
            }
        }

        false
    }

    /// Jumps to mark line (apostrophe - goes to first non-blank on line)
    pub fn jump_to_mark_line(&mut self, name: char) -> bool {
        // Linewise special marks delegate through exact-mark lookup first, then
        // normalize to first non-blank on that line.
        if matches!(name, '.' | '^') {
            if self.jump_to_mark(name) {
                Motions::first_non_blank(self.buffer_mut());
                return true;
            }
            return false;
        }

        // Try local mark first (a-z)
        if name.is_ascii_lowercase() {
            if let Some(mark) = self.nav.marks.get_mark(name) {
                // OV-00190: clamp the mark line to current buffer bounds so a
                // line deleted after the mark was set doesn't land the cursor
                // past EOF. Mirrors the global-mark branch below.
                let max_line = self.buffer().line_count().saturating_sub(1);
                let clamped_line = mark.line.min(max_line);

                // Find first non-blank character on the line (char index → grapheme)
                let first_non_blank = if let Some(line_text) = self.buffer().line(clamped_line) {
                    let char_col = line_text
                        .chars()
                        .position(|c| !c.is_whitespace())
                        .unwrap_or(0);
                    crate::unicode::char_to_grapheme_col(
                        &line_text,
                        crate::unicode::CharCol(char_col),
                    )
                } else {
                    GraphemeCol::ZERO
                };

                self.buffer_mut()
                    .cursor_mut()
                    .set_position(clamped_line, first_non_blank);
                // Center cursor after jump (Vim behavior)
                self.center_cursor_in_viewport();
                return true;
            }
        }

        // Try global mark (A-Z)
        if name.is_ascii_uppercase() {
            if let Some(global_mark) = self.nav.marks.get_global_mark(name).cloned() {
                // Load the file if mark targets a concrete path different from current.
                if let Some(mark_path) = global_mark.file_path.as_deref() {
                    let current_file = self.buffer().file_path().map(|s| s.to_string());
                    if current_file.as_deref() != Some(mark_path)
                        && self.load_file(mark_path).is_err()
                    {
                        return false; // Failed to load file
                    }
                }

                // Validate and clamp mark line to buffer bounds
                let max_line = self.buffer().line_count().saturating_sub(1);
                let clamped_line = global_mark.line.min(max_line);

                // Find first non-blank character on the line (char index → grapheme)
                let first_non_blank = if let Some(line_text) = self.buffer().line(clamped_line) {
                    let char_col = line_text
                        .chars()
                        .position(|c| !c.is_whitespace())
                        .unwrap_or(0);
                    crate::unicode::char_to_grapheme_col(
                        &line_text,
                        crate::unicode::CharCol(char_col),
                    )
                } else {
                    GraphemeCol::ZERO
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
        self.nav.jump_list.add_jump(cursor.line(), cursor.col().0);
    }

    /// Jumps back in the jump list (Ctrl-O)
    pub fn jump_back(&mut self) -> bool {
        if let Some((line, col)) = self.nav.jump_list.jump_back() {
            self.buffer_mut()
                .cursor_mut()
                .set_position(line, GraphemeCol(col));
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
            self.buffer_mut()
                .cursor_mut()
                .set_position(line, GraphemeCol(col));
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

    /// Repeats the last character-find motion (`;`/`,` in Vim).
    ///
    /// Returns `true` if a motion executed and moved the cursor.
    pub fn repeat_last_find(&mut self, reverse: bool) -> bool {
        let mut moved = false;

        if let Some((ch, find_type, direction)) = self.get_last_find() {
            let count = self.effective_count();

            let direction = if reverse {
                match direction {
                    FindDirection::Forward => FindDirection::Backward,
                    FindDirection::Backward => FindDirection::Forward,
                }
            } else {
                direction
            };

            moved = match (find_type, direction) {
                (FindType::Find, FindDirection::Forward) => {
                    Motions::find_char_forward(self.buffer_mut(), ch, count)
                }
                (FindType::Find, FindDirection::Backward) => {
                    Motions::find_char_backward(self.buffer_mut(), ch, count)
                }
                (FindType::Till, FindDirection::Forward) => {
                    if !reverse {
                        // For ';' after `t`, skip past current target before repeating.
                        let col = self.buffer().cursor().col();
                        self.buffer_mut()
                            .cursor_mut()
                            .set_col(GraphemeCol(col.0 + 1));
                        if !Motions::till_char_forward(self.buffer_mut(), ch, count) {
                            self.buffer_mut().cursor_mut().set_col(col);
                            false
                        } else {
                            true
                        }
                    } else {
                        Motions::till_char_forward(self.buffer_mut(), ch, count)
                    }
                }
                (FindType::Till, FindDirection::Backward) => {
                    if !reverse {
                        // For ';' after `T`, skip past current target before repeating.
                        let col = self.buffer().cursor().col();
                        if col > GraphemeCol::ZERO {
                            self.buffer_mut()
                                .cursor_mut()
                                .set_col(GraphemeCol(col.0 - 1));
                            if !Motions::till_char_backward(self.buffer_mut(), ch, count) {
                                self.buffer_mut().cursor_mut().set_col(col);
                                false
                            } else {
                                true
                            }
                        } else {
                            false
                        }
                    } else {
                        Motions::till_char_backward(self.buffer_mut(), ch, count)
                    }
                }
            };
        }

        self.clear_count();
        moved
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
            let entry = TagEntry::new(file_path, cursor.line(), cursor.col().0);
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
            let clamped_col = clamp_grapheme_col(self.buffer(), clamped_line, entry.col);

            // Jump to the position
            self.buffer_mut()
                .cursor_mut()
                .set_position(clamped_line, GraphemeCol(clamped_col));

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
