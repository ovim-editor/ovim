//! UI features: completion menu, file tree, quickfix, location list, substitute confirmation

use super::{Change, CompletionMenu, Editor, FileTree, LocationList, Mode, PathCompletionState, QuickfixEntry, QuickfixList, Range};

impl Editor {
    /// Gets a reference to the completion menu
    pub fn completion_menu(&self) -> &CompletionMenu {
        &self.completion_menu
    }

    /// Gets a mutable reference to the completion menu
    pub fn completion_menu_mut(&mut self) -> &mut CompletionMenu {
        &mut self.completion_menu
    }

    /// Gets a reference to the path completion state
    pub fn path_completion(&self) -> &PathCompletionState {
        &self.path_completion
    }

    /// Gets a mutable reference to the path completion state
    pub fn path_completion_mut(&mut self) -> &mut PathCompletionState {
        &mut self.path_completion
    }

    /// Hides the completion menu
    pub fn hide_completion_menu(&mut self) {
        self.completion_menu.hide();
    }

    /// Selects the next completion item
    pub fn completion_next(&mut self) {
        self.completion_menu.select_next();
    }

    /// Selects the previous completion item
    pub fn completion_previous(&mut self) {
        self.completion_menu.select_previous();
    }

    /// Accepts the currently selected completion
    pub fn accept_completion(&mut self) {
        if let Some(item) = self.completion_menu.selected_item() {
            // Get the text to insert (prefer insertText, fallback to label)
            let text_to_insert = if let Some(ref insert_text) = item.insert_text {
                insert_text.clone()
            } else {
                item.label.clone()
            };

            // Get cursor position
            let cursor_line = self.buffer().cursor().line();
            let cursor_col = self.buffer().cursor().col();

            // Calculate the range to replace
            let trigger_col = self.completion_menu.trigger_col();

            // Delete the partial word from trigger position to cursor
            if cursor_col > trigger_col {
                self.buffer_mut()
                    .delete_range(cursor_line, trigger_col, cursor_line, cursor_col);
            }

            // Insert the completion text
            self.buffer_mut()
                .insert_text_at(cursor_line, trigger_col, &text_to_insert);

            // Move cursor to end of inserted text
            let new_col = trigger_col + text_to_insert.chars().count();
            self.buffer_mut()
                .cursor_mut()
                .set_position(cursor_line, new_col);

            // Mark buffer as modified
            self.mark_buffer_modified();
        }

        // Hide the completion menu
        self.hide_completion_menu();
    }

    /// Gets the inlay hints for the current file
    pub fn inlay_hints(&self) -> &[lsp_types::InlayHint] {
        &self.lsp_state.inlay_hints
    }

    /// Gets the file tree
    pub fn file_tree(&self) -> &FileTree {
        &self.file_tree
    }

    /// Gets mutable file tree
    pub fn file_tree_mut(&mut self) -> &mut FileTree {
        &mut self.file_tree
    }

    /// Opens the file tree explorer at the project root
    pub fn open_file_tree(&mut self) {
        use crate::language_config::find_project_root;
        use git2::Repository;

        let file_path = self.buffer().file_path().map(|s| s.to_string());

        let root = if let Some(ref file_path) = file_path {
            let path = std::path::Path::new(file_path);

            // Try git root first (most reliable for project boundary)
            if let Ok(repo) = Repository::discover(path) {
                if let Some(workdir) = repo.workdir() {
                    workdir.to_path_buf()
                } else {
                    // Fallback: use language-specific markers
                    find_project_root(
                        path,
                        &[
                            "Cargo.toml".into(),
                            "package.json".into(),
                            ".git".into(),
                        ],
                    )
                }
            } else {
                // Not in git repo - use language markers or parent
                find_project_root(path, &["Cargo.toml".into(), "package.json".into()])
            }
        } else {
            std::env::current_dir().unwrap_or_default()
        };

        self.file_tree.open(&root);
    }

    /// Toggles the file tree visibility
    pub fn toggle_file_tree(&mut self) {
        if !self.file_tree.is_visible() {
            self.open_file_tree();
            self.set_mode(Mode::FileTree);
        } else {
            self.file_tree.toggle();
            self.set_mode(Mode::Normal);
        }
    }

    /// Opens the file selected in the file tree
    pub fn open_file_from_tree(&mut self) {
        if let Some(node) = self.file_tree.selected_node() {
            if node.is_dir() {
                // Toggle directory expansion
                self.file_tree.toggle_selected();
            } else {
                // Open file (checks for existing buffer)
                let path = node.path().to_path_buf();
                if let Ok(()) = self.open_file(&path) {
                    // Switch back to Normal mode and keep file tree visible
                    self.set_mode(Mode::Normal);
                }
            }
        }
    }

    /// Gets the quickfix list
    pub fn quickfix_list(&self) -> &QuickfixList {
        &self.quickfix_list
    }

    /// Gets mutable quickfix list
    pub fn quickfix_list_mut(&mut self) -> &mut QuickfixList {
        &mut self.quickfix_list
    }

    /// Sets the quickfix list entries
    pub fn set_quickfix_list(&mut self, entries: Vec<QuickfixEntry>, title: String) {
        self.quickfix_list.set_entries(entries, title);
    }

    /// Opens the quickfix window
    pub fn open_quickfix_window(&mut self) {
        self.quickfix_window_open = true;
    }

    /// Closes the quickfix window
    pub fn close_quickfix_window(&mut self) {
        self.quickfix_window_open = false;
    }

    /// Toggles the quickfix window
    pub fn toggle_quickfix_window(&mut self) {
        self.quickfix_window_open = !self.quickfix_window_open;
    }

    /// Whether the quickfix window is open
    pub fn is_quickfix_window_open(&self) -> bool {
        self.quickfix_window_open
    }

    /// Jumps to the current quickfix entry
    pub fn jump_to_quickfix_entry(&mut self) {
        // Extract values first to avoid borrow issues
        let (path, lnum, qcol) = if let Some(entry) = self.quickfix_list.current_entry() {
            (entry.filename.clone(), entry.lnum, entry.col)
        } else {
            return;
        };

        if let Some(path) = path {
            // Open file (checks for existing buffer)
            if let Ok(()) = self.open_file(&path) {
                // Move cursor to the location
                if lnum > 0 {
                    let line = lnum.saturating_sub(1);
                    let col = if qcol > 0 { qcol.saturating_sub(1) } else { 0 };
                    self.buffer_mut().cursor_mut().set_position(line, col);
                }
            }
        }
    }

    /// Gets the location list
    pub fn location_list(&self) -> &LocationList {
        &self.location_list
    }

    /// Gets mutable location list
    pub fn location_list_mut(&mut self) -> &mut LocationList {
        &mut self.location_list
    }

    /// Sets the location list entries
    pub fn set_location_list(&mut self, entries: Vec<QuickfixEntry>, title: String) {
        self.location_list.set_entries(entries, title);
    }

    /// Opens the location list window
    pub fn open_location_window(&mut self) {
        self.location_window_open = true;
    }

    /// Closes the location list window
    pub fn close_location_window(&mut self) {
        self.location_window_open = false;
    }

    /// Toggles the location list window
    pub fn toggle_location_window(&mut self) {
        self.location_window_open = !self.location_window_open;
    }

    /// Whether the location list window is open
    pub fn is_location_window_open(&self) -> bool {
        self.location_window_open
    }

    /// Jumps to the current location list entry
    pub fn jump_to_location_entry(&mut self) {
        // Extract values first to avoid borrow issues
        let (path, lnum, lcol) = if let Some(entry) = self.location_list.current_entry() {
            (entry.filename.clone(), entry.lnum, entry.col)
        } else {
            return;
        };

        if let Some(path) = path {
            // Open file (checks for existing buffer)
            if let Ok(()) = self.open_file(&path) {
                // Move cursor to the location
                if lnum > 0 {
                    let line = lnum.saturating_sub(1);
                    let col = if lcol > 0 { lcol.saturating_sub(1) } else { 0 };
                    self.buffer_mut().cursor_mut().set_position(line, col);
                }
            }
        }
    }

    // ========== Substitute Confirmation ==========

    /// Starts substitute confirmation mode with the given matches
    pub fn start_substitute_confirm(
        &mut self,
        matches: Vec<(usize, usize, usize, String)>,
        pattern: regex::Regex,
    ) {
        self.substitute_matches = matches;
        self.substitute_match_index = 0;
        self.substitute_pattern = Some(pattern);
        if !self.substitute_matches.is_empty() {
            self.mode = Mode::SubstituteConfirm;
            // Move cursor to first match
            let (line, col, _, _) = self.substitute_matches[0];
            self.buffer_mut().cursor_mut().set_position(line, col);
        }
    }

    /// Gets the current substitute match info (line, start_col, end_col, replacement)
    pub fn current_substitute_match(&self) -> Option<&(usize, usize, usize, String)> {
        self.substitute_matches.get(self.substitute_match_index)
    }

    /// Gets the substitute pattern for highlighting
    pub fn substitute_pattern(&self) -> Option<&regex::Regex> {
        self.substitute_pattern.as_ref()
    }

    /// Confirms the current substitution and moves to the next
    pub fn confirm_substitute(&mut self) {
        if let Some((line, start_col, end_col, replacement)) =
            self.substitute_matches.get(self.substitute_match_index).cloned()
        {
            // Perform the substitution
            let cursor_before = (self.buffer().cursor().line(), self.buffer().cursor().col());

            // Delete the matched text
            let deleted = self.buffer_mut().delete_range(line, start_col, line, end_col);
            let delete_range = Range::new((line, start_col), (line, end_col));
            let delete_change = Change::delete(delete_range, deleted, cursor_before);

            // Insert the replacement
            let insert_change = Change::insert((line, start_col), replacement.clone(), cursor_before);
            insert_change.apply(self.buffer_mut());

            self.add_change(delete_change);
            self.add_change(insert_change);

            self.substitute_match_index += 1;
            if self.substitute_match_index >= self.substitute_matches.len() {
                self.end_substitute_confirm();
            } else {
                // Move cursor to next match
                let (next_line, next_col, _, _) = self.substitute_matches[self.substitute_match_index];
                self.buffer_mut().cursor_mut().set_position(next_line, next_col);
            }
        }
    }

    /// Skips the current match and moves to the next
    pub fn skip_substitute(&mut self) {
        self.substitute_match_index += 1;
        if self.substitute_match_index >= self.substitute_matches.len() {
            self.end_substitute_confirm();
        } else {
            // Move cursor to next match
            let (line, col, _, _) = self.substitute_matches[self.substitute_match_index];
            self.buffer_mut().cursor_mut().set_position(line, col);
        }
    }

    /// Confirms all remaining substitutions
    pub fn confirm_all_substitutes(&mut self) {
        while self.substitute_match_index < self.substitute_matches.len() {
            self.confirm_substitute();
        }
    }

    /// Confirms current and quits
    pub fn confirm_substitute_and_quit(&mut self) {
        self.confirm_substitute();
        self.end_substitute_confirm();
    }

    /// Ends substitute confirmation mode
    pub fn end_substitute_confirm(&mut self) {
        self.substitute_matches.clear();
        self.substitute_match_index = 0;
        self.substitute_pattern = None;
        self.mode = Mode::Normal;
    }
}
