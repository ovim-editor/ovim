//! UI features: completion menu, file tree, quickfix, location list, substitute confirmation

use super::{
    CompletionMenu, Editor, FileTree, LocationList, Mode, PathCompletionState, QuickfixEntry,
    QuickfixList,
};

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
        &self.ui_panels.path_completion
    }

    /// Gets a mutable reference to the path completion state
    pub fn path_completion_mut(&mut self) -> &mut PathCompletionState {
        &mut self.ui_panels.path_completion
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

            let cursor_before = (cursor_line, cursor_col);
            let ((), edits) = self.buffer_mut().record(|buf| {
                if cursor_col > trigger_col {
                    buf.delete_range(cursor_line, trigger_col, cursor_line, cursor_col);
                }
                if !text_to_insert.is_empty() {
                    buf.insert_text_at(cursor_line, trigger_col, &text_to_insert);
                }

                // Cursor moves to end of accepted completion text.
                let new_col = trigger_col + text_to_insert.chars().count();
                buf.cursor_mut().set_position(cursor_line, new_col);
            });
            if !edits.is_empty() {
                let cursor_after = self.cursor_position();
                self.push_recorded_undo(edits, cursor_before, cursor_after);
            }
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
        &self.ui_panels.file_tree
    }

    /// Gets mutable file tree
    pub fn file_tree_mut(&mut self) -> &mut FileTree {
        &mut self.ui_panels.file_tree
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
                        &["Cargo.toml".into(), "package.json".into(), ".git".into()],
                    )
                }
            } else {
                // Not in git repo - use language markers or parent
                find_project_root(path, &["Cargo.toml".into(), "package.json".into()])
            }
        } else {
            std::env::current_dir().unwrap_or_default()
        };

        self.ui_panels.file_tree.open(&root);
    }

    /// Toggles the file tree with reveal semantics:
    /// - Tree closed → open, reveal current file, enter FileTree mode
    /// - Tree open + buffer focused (Normal) → reveal current file, enter FileTree mode
    /// - Tree open + tree focused (FileTree) → close tree, enter Normal mode
    pub fn toggle_file_tree(&mut self) {
        if self.mode() == Mode::FileTree {
            // Focused on tree → close it
            self.ui_panels.file_tree.close();
            self.set_mode(Mode::Normal);
        } else {
            // Not focused → open/reveal + focus
            if !self.ui_panels.file_tree.is_visible() {
                self.open_file_tree();
            }
            if self.options.file_tree_reveal {
                if let Some(path) = self.buffer().file_path().map(|s| s.to_string()) {
                    self.ui_panels
                        .file_tree
                        .reveal_path(std::path::Path::new(&path));
                }
            }
            self.set_mode(Mode::FileTree);
        }
    }

    /// Opens the file selected in the file tree.
    /// Opens file and closes the tree, or toggles directory expansion.
    pub fn open_file_from_tree(&mut self) {
        if let Some(node) = self.ui_panels.file_tree.selected_node() {
            if node.is_dir() {
                // Toggle directory expansion
                self.ui_panels.file_tree.toggle_selected();
            } else {
                // Open file (checks for existing buffer)
                let path = node.path().to_path_buf();
                if let Ok(()) = self.open_file(&path) {
                    // Close tree and switch to Normal mode
                    self.ui_panels.file_tree.close();
                    self.set_mode(Mode::Normal);
                }
            }
        }
    }

    /// Gets the quickfix list
    pub fn quickfix_list(&self) -> &QuickfixList {
        &self.ui_panels.quickfix_list
    }

    /// Gets mutable quickfix list
    pub fn quickfix_list_mut(&mut self) -> &mut QuickfixList {
        &mut self.ui_panels.quickfix_list
    }

    /// Sets the quickfix list entries
    pub fn set_quickfix_list(&mut self, entries: Vec<QuickfixEntry>, title: String) {
        self.ui_panels.quickfix_list.set_entries(entries, title);
    }

    /// Opens the quickfix window
    pub fn open_quickfix_window(&mut self) {
        self.ui_panels.quickfix_window_open = true;
    }

    /// Closes the quickfix window
    pub fn close_quickfix_window(&mut self) {
        self.ui_panels.quickfix_window_open = false;
    }

    /// Toggles the quickfix window
    pub fn toggle_quickfix_window(&mut self) {
        self.ui_panels.quickfix_window_open = !self.ui_panels.quickfix_window_open;
    }

    /// Whether the quickfix window is open
    pub fn is_quickfix_window_open(&self) -> bool {
        self.ui_panels.quickfix_window_open
    }

    /// Jumps to the current quickfix entry
    pub fn jump_to_quickfix_entry(&mut self) {
        // Extract values first to avoid borrow issues
        let (path, lnum, qcol) = if let Some(entry) = self.ui_panels.quickfix_list.current_entry() {
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

    /// Sets a pending `:make` background job.
    pub fn set_pending_make(&mut self, pending: super::PendingMake) {
        self.pending_make = Some(pending);
    }

    /// Polls for a completed `:make` job. Returns true if results were applied.
    pub fn poll_pending_make(&mut self) -> bool {
        let pending = match self.pending_make.take() {
            Some(p) => p,
            None => return false,
        };

        match pending.receiver.try_recv() {
            Ok(result) => {
                let entries = crate::commands::parse_compiler_output(&result.output);
                let entry_count = entries.len();
                let title = format!(":make {}", pending.command);
                self.set_quickfix_list(entries, title);

                if entry_count > 0 {
                    self.ui_panels.quickfix_list.first();
                    self.jump_to_quickfix_entry();
                    self.open_quickfix_window();
                    self.set_lsp_status(format!("{} error(s)/warning(s)", entry_count));
                } else if result.success {
                    self.close_quickfix_window();
                    self.set_lsp_status("Build succeeded — no errors".to_string());
                } else {
                    self.set_lsp_status("Build failed (no parseable errors)".to_string());
                }
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Still running — put it back
                self.pending_make = Some(pending);
                false
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.set_lsp_status("Make job failed (thread panicked)".to_string());
                true
            }
        }
    }

    /// Gets the location list
    pub fn location_list(&self) -> &LocationList {
        &self.ui_panels.location_list
    }

    /// Gets mutable location list
    pub fn location_list_mut(&mut self) -> &mut LocationList {
        &mut self.ui_panels.location_list
    }

    /// Sets the location list entries
    pub fn set_location_list(&mut self, entries: Vec<QuickfixEntry>, title: String) {
        self.ui_panels.location_list.set_entries(entries, title);
    }

    /// Opens the location list window
    pub fn open_location_window(&mut self) {
        self.ui_panels.location_window_open = true;
    }

    /// Closes the location list window
    pub fn close_location_window(&mut self) {
        self.ui_panels.location_window_open = false;
    }

    /// Toggles the location list window
    pub fn toggle_location_window(&mut self) {
        self.ui_panels.location_window_open = !self.ui_panels.location_window_open;
    }

    /// Whether the location list window is open
    pub fn is_location_window_open(&self) -> bool {
        self.ui_panels.location_window_open
    }

    /// Jumps to the current location list entry
    pub fn jump_to_location_entry(&mut self) {
        // Extract values first to avoid borrow issues
        let (path, lnum, lcol) = if let Some(entry) = self.ui_panels.location_list.current_entry() {
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
        self.editing.substitute_matches = matches;
        self.editing.substitute_match_index = 0;
        self.editing.substitute_pattern = Some(pattern);
        if !self.editing.substitute_matches.is_empty() {
            self.mode = Mode::SubstituteConfirm;
            // Move cursor to first match
            let (line, col, _, _) = self.editing.substitute_matches[0];
            self.buffer_mut().cursor_mut().set_position(line, col);
        }
    }

    /// Gets the current substitute match info (line, start_col, end_col, replacement)
    pub fn current_substitute_match(&self) -> Option<&(usize, usize, usize, String)> {
        self.editing
            .substitute_matches
            .get(self.editing.substitute_match_index)
    }

    /// Gets the substitute pattern for highlighting
    pub fn substitute_pattern(&self) -> Option<&regex::Regex> {
        self.editing.substitute_pattern.as_ref()
    }

    /// Confirms the current substitution and moves to the next
    pub fn confirm_substitute(&mut self) {
        if let Some((line, start_col, end_col, replacement)) = self
            .editing
            .substitute_matches
            .get(self.editing.substitute_match_index)
            .cloned()
        {
            // Perform the substitution
            let cursor_before = self.cursor_position();
            let ((), edits) = self.buffer_mut().record(|buf| {
                // Always perform delete + insert for confirmed matches so undo
                // round-trips exactly what the user confirmed.
                buf.delete_range(line, start_col, line, end_col);
                if !replacement.is_empty() {
                    buf.insert_text_at(line, start_col, &replacement);
                }
            });
            if !edits.is_empty() {
                let cursor_after = self.cursor_position();
                self.push_recorded_undo(edits, cursor_before, cursor_after);
            }

            self.editing.substitute_match_index += 1;
            if self.editing.substitute_match_index >= self.editing.substitute_matches.len() {
                self.end_substitute_confirm();
            } else {
                // Move cursor to next match
                let (next_line, next_col, _, _) =
                    self.editing.substitute_matches[self.editing.substitute_match_index];
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(next_line, next_col);
            }
        }
    }

    /// Skips the current match and moves to the next
    pub fn skip_substitute(&mut self) {
        self.editing.substitute_match_index += 1;
        if self.editing.substitute_match_index >= self.editing.substitute_matches.len() {
            self.end_substitute_confirm();
        } else {
            // Move cursor to next match
            let (line, col, _, _) =
                self.editing.substitute_matches[self.editing.substitute_match_index];
            self.buffer_mut().cursor_mut().set_position(line, col);
        }
    }

    /// Confirms all remaining substitutions
    pub fn confirm_all_substitutes(&mut self) {
        while self.editing.substitute_match_index < self.editing.substitute_matches.len() {
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
        self.editing.substitute_matches.clear();
        self.editing.substitute_match_index = 0;
        self.editing.substitute_pattern = None;
        self.mode = Mode::Normal;
    }

    // ==================== LSP Manager Panel ====================

    pub fn lsp_manager_panel(&self) -> Option<&super::LspManagerPanel> {
        self.lsp_ui.lsp_manager_panel.as_ref()
    }

    pub fn lsp_manager_panel_mut(&mut self) -> Option<&mut super::LspManagerPanel> {
        self.lsp_ui.lsp_manager_panel.as_mut()
    }

    pub fn open_lsp_manager(&mut self) {
        let running = self.get_running_lsp_servers();
        self.lsp_ui.lsp_manager_panel = Some(super::LspManagerPanel::new(running));
        self.mode = Mode::LspManager;
        // Ensure install channel exists
        if self.lsp_ui.install_progress_tx.is_none() {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            self.lsp_ui.install_progress_tx = Some(tx);
            self.lsp_ui.install_progress_rx = Some(rx);
        }
    }

    pub fn close_lsp_manager(&mut self) {
        self.lsp_ui.lsp_manager_panel = None;
        self.mode = Mode::Normal;
    }

    /// Trigger LSP server install for a language.
    /// Enqueues a pending install request to be picked up by the event loop.
    pub fn request_lsp_install(&mut self, language_id: &str) {
        use super::lsp_manager_panel::{InstallStatus, PendingInstallRequest};
        use crate::language_config::LanguageRegistry;

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
        if let Some(panel) = &mut self.lsp_ui.lsp_manager_panel {
            panel.active_installs.insert(
                language_id.to_string(),
                InstallStatus::Installing("Starting...".to_string()),
            );
        }

        // Queue the request for the event loop to spawn
        self.lsp_ui.pending_installs.push(PendingInstallRequest {
            language_id: language_id.to_string(),
            language_name: lang.name.clone(),
            auto_install_config: auto_install.clone(),
            lsp_command: lsp.command.clone(),
        });
    }

    /// Trigger LSP server uninstall for a language
    pub fn request_lsp_uninstall(&mut self, language_id: &str) {
        use crate::language_config::LanguageRegistry;

        let Some(registry) = LanguageRegistry::try_get() else {
            return;
        };
        let Some(lang) = registry.get_by_id(language_id) else {
            return;
        };
        let Some(lsp) = &lang.lsp else { return };

        // Determine uninstall command based on auto_install config
        let hint = if let Some(auto) = &lsp.auto_install {
            match &auto.method {
                crate::language_config::InstallMethod::Npm { global, .. } => {
                    let packages = auto.method.npm_packages();
                    if packages.is_empty() {
                        return self.set_lsp_status(format!(
                            "No uninstall method configured for {}",
                            lang.name
                        ));
                    }
                    let flag = if *global { " -g" } else { "" };
                    format!("Run: npm uninstall{flag} {}", packages.join(" "))
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

        let Some(rx) = &mut self.lsp_ui.install_progress_rx else {
            return false;
        };

        let mut updated = false;
        while let Ok(progress) = rx.try_recv() {
            if let Some(panel) = &mut self.lsp_ui.lsp_manager_panel {
                panel
                    .active_installs
                    .insert(progress.language_id.clone(), progress.status.clone());

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
    pub fn take_pending_installs(
        &mut self,
    ) -> Vec<super::lsp_manager_panel::PendingInstallRequest> {
        std::mem::take(&mut self.lsp_ui.pending_installs)
    }

    /// Get the install progress sender (for spawning background tasks)
    pub fn install_progress_tx(
        &self,
    ) -> Option<&tokio::sync::mpsc::UnboundedSender<super::lsp_manager_panel::InstallProgress>>
    {
        self.lsp_ui.install_progress_tx.as_ref()
    }

    /// Get language IDs of currently running LSP servers
    fn get_running_lsp_servers(&self) -> Vec<String> {
        self.lsp_state.running_server_languages()
    }
}
