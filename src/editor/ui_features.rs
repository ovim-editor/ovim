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

    /// Toggles the file tree with reveal semantics:
    /// - Tree closed → open, reveal current file, enter FileTree mode
    /// - Tree open + buffer focused (Normal) → reveal current file, enter FileTree mode
    /// - Tree open + tree focused (FileTree) → close tree, enter Normal mode
    pub fn toggle_file_tree(&mut self) {
        if self.mode() == Mode::FileTree {
            // Focused on tree → close it
            self.file_tree.close();
            self.set_mode(Mode::Normal);
        } else {
            // Not focused → open/reveal + focus
            if !self.file_tree.is_visible() {
                self.open_file_tree();
            }
            if self.options.file_tree_reveal {
                if let Some(path) = self.buffer().file_path().map(|s| s.to_string()) {
                    self.file_tree.reveal_path(std::path::Path::new(&path));
                }
            }
            self.set_mode(Mode::FileTree);
        }
    }

    /// Opens the file selected in the file tree.
    /// Opens file and closes the tree, or toggles directory expansion.
    pub fn open_file_from_tree(&mut self) {
        if let Some(node) = self.file_tree.selected_node() {
            if node.is_dir() {
                // Toggle directory expansion
                self.file_tree.toggle_selected();
            } else {
                // Open file (checks for existing buffer)
                let path = node.path().to_path_buf();
                if let Ok(()) = self.open_file(&path) {
                    // Close tree and switch to Normal mode
                    self.file_tree.close();
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

    // ==================== LSP Manager Panel ====================

    pub fn lsp_manager_panel(&self) -> Option<&super::LspManagerPanel> {
        self.lsp_manager_panel.as_ref()
    }

    pub fn lsp_manager_panel_mut(&mut self) -> Option<&mut super::LspManagerPanel> {
        self.lsp_manager_panel.as_mut()
    }

    pub fn open_lsp_manager(&mut self) {
        let running = self.get_running_lsp_servers();
        self.lsp_manager_panel = Some(super::LspManagerPanel::new(running));
        self.mode = Mode::LspManager;
        // Ensure install channel exists
        if self.install_progress_tx.is_none() {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            self.install_progress_tx = Some(tx);
            self.install_progress_rx = Some(rx);
        }
    }

    pub fn close_lsp_manager(&mut self) {
        self.lsp_manager_panel = None;
        self.mode = Mode::Normal;
    }

    /// Trigger LSP server install for a language.
    /// Enqueues a pending install request to be picked up by the event loop.
    pub fn request_lsp_install(&mut self, language_id: &str) {
        use crate::language_config::LanguageRegistry;
        use super::lsp_manager_panel::{InstallStatus, PendingInstallRequest};

        let Some(registry) = LanguageRegistry::try_get() else {
            self.set_lsp_status("Language registry not initialized".to_string());
            return;
        };

        let Some(lang) = registry.get_by_id(language_id) else {
            self.set_lsp_status(format!("Unknown language: {language_id}"));
            return;
        };

        let Some(lsp) = &lang.lsp else {
            self.set_lsp_status(format!("No LSP configured for {}", lang.name));
            return;
        };

        let Some(auto_install) = &lsp.auto_install else {
            if let Some(hint) = &lsp.install_hint {
                self.set_lsp_status(hint.clone());
            } else {
                self.set_lsp_status(format!("No install method for {}", lang.name));
            }
            return;
        };

        // Set installing status in panel
        if let Some(panel) = &mut self.lsp_manager_panel {
            panel.active_installs.insert(
                language_id.to_string(),
                InstallStatus::Installing("Starting...".to_string()),
            );
        }

        // Queue the request for the event loop to spawn
        self.pending_installs.push(PendingInstallRequest {
            language_id: language_id.to_string(),
            language_name: lang.name.clone(),
            auto_install_config: auto_install.clone(),
            lsp_command: lsp.command.clone(),
        });
    }

    /// Trigger LSP server uninstall for a language
    pub fn request_lsp_uninstall(&mut self, language_id: &str) {
        use crate::language_config::LanguageRegistry;

        let Some(registry) = LanguageRegistry::try_get() else { return };
        let Some(lang) = registry.get_by_id(language_id) else { return };
        let Some(lsp) = &lang.lsp else { return };

        // Determine uninstall command based on auto_install config
        let hint = if let Some(auto) = &lsp.auto_install {
            match &auto.method {
                crate::language_config::InstallMethod::Npm { package, global } => {
                    let flag = if *global { " -g" } else { "" };
                    format!("Run: npm uninstall{flag} {package}")
                }
                crate::language_config::InstallMethod::Cargo { package } => {
                    format!("Run: cargo uninstall {package}")
                }
                crate::language_config::InstallMethod::Shell { command } => {
                    format!("Installed via shell. Remove manually: {command}")
                }
                crate::language_config::InstallMethod::Github { install_path, .. } => {
                    format!("Remove: {install_path}")
                }
            }
        } else if let Some(hint) = &lsp.install_hint {
            format!("Manual removal needed. Install method: {hint}")
        } else {
            format!("No uninstall method for {}", lang.name)
        };

        self.set_lsp_status(hint);
    }

    /// Poll install progress channel and update panel state
    pub fn poll_install_progress(&mut self) -> bool {
        use super::lsp_manager_panel::InstallStatus;

        let Some(rx) = &mut self.install_progress_rx else {
            return false;
        };

        let mut updated = false;
        while let Ok(progress) = rx.try_recv() {
            if let Some(panel) = &mut self.lsp_manager_panel {
                panel.active_installs.insert(
                    progress.language_id.clone(),
                    progress.status.clone(),
                );

                // On success, rebuild entries to reflect new state
                if matches!(progress.status, InstallStatus::Success) {
                    let running = self.lsp_state.running_server_languages();
                    panel.update_running_servers(running);
                }
            }
            updated = true;
        }
        updated
    }

    /// Drain pending install requests (called by event loop)
    pub fn take_pending_installs(&mut self) -> Vec<super::lsp_manager_panel::PendingInstallRequest> {
        std::mem::take(&mut self.pending_installs)
    }

    /// Get the install progress sender (for spawning background tasks)
    pub fn install_progress_tx(&self) -> Option<&tokio::sync::mpsc::UnboundedSender<super::lsp_manager_panel::InstallProgress>> {
        self.install_progress_tx.as_ref()
    }

    /// Get language IDs of currently running LSP servers
    fn get_running_lsp_servers(&self) -> Vec<String> {
        self.lsp_state.running_server_languages()
    }
}
