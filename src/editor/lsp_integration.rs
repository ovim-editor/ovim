//! LSP Integration for Editor
//!
//! This module contains all LSP-related functionality extracted from the main editor module.
//! It provides LSP initialization, document synchronization, LSP actions, and workspace editing.

use super::*;
use crate::lsp::{LspManager, uri_from_file_path, uri_to_file_path};
use super::picker::PickerResult;

use anyhow::{anyhow, Result};
use lsp_types::Location;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

impl Editor {
    /// Enables LSP support
    pub fn enable_lsp(&mut self) {
        let (tx, rx) = mpsc::unbounded_channel();
        self.lsp_state.lsp_manager = Some(Arc::new(LspManager::new()));
        self.lsp_command_tx = Some(tx);
        self.lsp_command_rx = Some(rx);
    }

    /// Gets a reference to the LSP manager
    pub fn lsp_manager(&self) -> Option<Arc<LspManager>> {
        self.lsp_state.lsp_manager.clone()
    }

    /// Gets a reference to the LSP command sender for background tasks
    pub fn lsp_command_sender(&self) -> Option<mpsc::UnboundedSender<LspCommand>> {
        self.lsp_command_tx.clone()
    }

    /// Close the LSP for the current file
    pub async fn close_current_file_lsp(&mut self) {
        let Some(ref lsp) = self.lsp_state.lsp_manager else {
            return;
        };

        let Some(file_path) = self.buffer().file_path() else {
            return;
        };

        let uri = match uri_from_file_path(file_path) {
            Some(u) => u,
            None => return,
        };

        // Get language_id from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => return,
        };

        // Send LSP close notification
        let file_path_string = file_path.to_string();
        let _ = lsp.did_close(uri, &language_id).await;
        self.lsp_state.document_sync.remove(&file_path_string);
    }

    /// Check if LSP initialization is needed for the current file
    pub fn needs_lsp_init(&self) -> Option<String> {
        if self.lsp_state.needs_lsp_init {
            self.buffer().file_path().map(|s| s.to_string())
        } else {
            None
        }
    }

    /// Clear the LSP initialization flag after init is complete
    pub fn clear_lsp_init_flag(&mut self) {
        self.lsp_state.needs_lsp_init = false;
    }

    /// Request LSP initialization for the current file
    pub fn request_lsp_init(&mut self) {
        self.lsp_state.needs_lsp_init = true;
    }

    /// Get current file diagnostics from LSP
    pub async fn get_current_file_diagnostics(&self) -> Option<Vec<lsp_types::Diagnostic>> {
        let lsp = self.lsp_state.lsp_manager.as_ref()?;
        let file_path = self.buffer().file_path()?;
        let uri = uri_from_file_path(file_path)?;
        let diagnostics = lsp.get_diagnostics(&uri).await;
        Some(diagnostics)
    }

    /// Query and cache diagnostics for the current file
    pub async fn update_diagnostics(&mut self) {
        if let Some(lsp) = &self.lsp_state.lsp_manager {
            if let Some(file_path) = self.buffer().file_path() {
                if let Some(uri) = uri_from_file_path(file_path) {
                    let diagnostics = lsp.get_diagnostics(&uri).await;
                    // Update count cache
                    self.lsp_state.diagnostic_count = self.get_diagnostic_count().await;
                    // Cache full diagnostic list
                    self.lsp_state.current_file_diagnostics = diagnostics;
                    return;
                }
            }
        }
        self.lsp_state.current_file_diagnostics.clear();
    }

    /// Get total diagnostic count (errors, warnings, info, hints) from cached diagnostics
    pub async fn get_diagnostic_count(&self) -> (usize, usize, usize, usize) {
        self.lsp_state.diagnostic_count
    }

    /// Updates the cached diagnostic count (should be called when diagnostics change)
    pub async fn update_diagnostic_cache(&mut self) {
        let start = std::time::Instant::now();

        // Query diagnostic count from LSP manager
        let count = if let Some(lsp) = &self.lsp_state.lsp_manager {
            if let Some(file_path) = self.buffer().file_path() {
                if let Some(uri) = crate::lsp::uri_from_file_path(file_path) {
                    lsp.count_diagnostics(&uri).await
                } else {
                    (0, 0, 0, 0)
                }
            } else {
                (0, 0, 0, 0)
            }
        } else {
            (0, 0, 0, 0)
        };

        self.lsp_state.diagnostic_count = count;

        // Also update the full diagnostics list for inline display
        if let Some(diagnostics) = self.get_current_file_diagnostics().await {
            self.lsp_state.current_file_diagnostics = diagnostics;
        } else {
            self.lsp_state.current_file_diagnostics.clear();
        }

        let duration = start.elapsed().as_micros() as u64;
        self.record_diagnostic_query_duration(duration);
    }

    /// Get diagnostics for a specific line from cached diagnostics
    pub fn diagnostics_for_line(&self, line: usize) -> Vec<&lsp_types::Diagnostic> {
        self.lsp_state
            .current_file_diagnostics
            .iter()
            .filter(|d| d.range.start.line as usize == line)
            .collect()
    }

    /// Get the current diagnostic at the cursor position
    pub fn current_diagnostic(&self) -> Option<String> {
        let line = self.buffer().cursor().line();
        let diagnostics = &self.lsp_state.current_file_diagnostics;

        diagnostics
            .iter()
            .find(|d| d.range.start.line as usize == line)
            .map(|d| d.message.clone())
    }

    /// Get the total number of diagnostics
    pub fn diagnostic_count(&self) -> usize {
        let diagnostics = &self.lsp_state.current_file_diagnostics;
        diagnostics.len()
    }

    /// Set LSP status message
    pub fn set_lsp_status(&mut self, status: String) {
        self.lsp_state.lsp_status = status;
    }

    /// Get current LSP status
    pub fn lsp_status(&self) -> &str {
        &self.lsp_state.lsp_status
    }

    /// Register a new LSP server
    pub fn register_lsp_server(&mut self, language_id: String, server_name: String) {
        self.lsp_state.lsp_status = format!("LSP: {} ready", server_name);
        self.lsp_state
            .active_lsp_servers
            .insert(language_id, server_name);
    }

    /// Unregister an LSP server
    pub fn unregister_lsp_server(&mut self, language_id: &str) {
        self.lsp_state.active_lsp_servers.remove(language_id);
        if self.lsp_state.active_lsp_servers.is_empty() {
            self.lsp_state.lsp_status.clear();
        }
    }

    /// Clear all LSP state (hover, code actions, completions, pending action)
    pub(crate) fn clear_lsp_state(&mut self) {
        self.lsp_state.hover_info = None;
        self.lsp_state.hover_scroll = 0;
        self.lsp_state.available_code_actions.clear();
        self.lsp_state.available_completions.clear();
        self.lsp_state.pending_lsp_action = None;
    }

    /// Get active LSP servers map
    pub fn active_lsp_servers(&self) -> &HashMap<String, String> {
        &self.lsp_state.active_lsp_servers
    }

    /// Get LSP progress message (e.g., "indexing...")
    pub fn lsp_progress_message(&self) -> Option<String> {
        if let Some(lsp_manager) = &self.lsp_state.lsp_manager {
            lsp_manager.get_progress_message()
        } else {
            None
        }
    }

    /// Get LSP info for status line
    pub fn get_lsp_info(&self) -> String {
        let mut info = String::new();

        // LSP Manager status
        if self.lsp_state.lsp_manager.is_some() {
            info.push_str("LSP: enabled\n");
        } else {
            info.push_str("LSP: disabled\n");
        }

        // Active servers
        if self.lsp_state.active_lsp_servers.is_empty() {
            info.push_str("Servers: none\n");
        } else {
            info.push_str("Servers:\n");
            for (lang_id, server_name) in &self.lsp_state.active_lsp_servers {
                info.push_str(&format!("  - {} ({})\n", server_name, lang_id));
            }
        }

        // Progress messages
        if let Some(progress) = self.lsp_progress_message() {
            info.push_str(&format!("Progress: {}\n", progress));
        }

        // Diagnostic counts
        let (errors, warnings, infos, hints) = self.lsp_state.diagnostic_count;
        info.push_str(&format!(
            "Diagnostics: E:{} W:{} I:{} H:{}\n",
            errors, warnings, infos, hints
        ));

        // Current status
        if !self.lsp_state.lsp_status.is_empty() {
            info.push_str(&format!("\nStatus: {}\n", self.lsp_state.lsp_status));
        }

        info
    }

    // -------------------------------------------------------------------------
    // LSP Action Requests (set pending_lsp_action flag)
    // -------------------------------------------------------------------------

    /// Request go-to-definition at current cursor position
    pub fn request_goto_definition(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::GoToDefinition);
    }

    /// Request go-to-implementation at current cursor position
    pub fn request_goto_implementation(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::GoToImplementation);
    }

    /// Request go-to-type-definition at current cursor position
    pub fn request_goto_type(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::GoToType);
    }

    /// Request hover info at current cursor position
    /// This will set the pending action flag, which will be processed
    /// in the next event loop iteration via process_pending_lsp_actions()
    pub fn request_hover(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::ShowHover);
    }

    /// Request completion at current cursor position
    pub fn request_completion(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::Completion);
    }

    /// Request document format
    pub fn request_format_document(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::FormatDocument);
    }

    /// Request code actions at current cursor position
    pub fn request_code_actions(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::CodeActions);
    }

    /// Request call hierarchy (incoming calls) at current cursor position
    pub fn request_call_hierarchy_incoming(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::CallHierarchyIncoming);
    }

    /// Request call hierarchy (outgoing calls) at current cursor position
    pub fn request_call_hierarchy_outgoing(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::CallHierarchyOutgoing);
    }

    /// Request type hierarchy at current cursor position
    pub fn request_type_hierarchy(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::TypeHierarchy);
    }

    /// Request organize imports for the current document
    pub fn request_organize_imports(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::OrganizeImports);
    }

    /// Request find references at current cursor position
    pub fn request_find_references(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::FindReferences);
    }

    /// Request document symbols for the current document
    pub fn request_document_symbols(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::DocumentSymbols);
    }

    /// Request workspace symbols
    pub fn request_workspace_symbols(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::WorkspaceSymbols);
    }

    /// Request rename at current cursor position
    pub fn request_rename(&mut self, new_name: String) {
        self.lsp_state.pending_lsp_action = Some(LspAction::Rename(new_name));
    }

    /// Request semantic tokens for the current document
    pub fn request_semantic_tokens(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::SemanticTokens);
    }

    /// Get current hover info text
    pub fn hover_info(&self) -> Option<&str> {
        self.lsp_state.hover_info.as_deref()
    }

    /// Clear hover info
    pub fn clear_hover(&mut self) {
        self.lsp_state.hover_info = None;
        self.lsp_state.hover_scroll = 0;
    }

    /// Get hover scroll position
    pub fn hover_scroll(&self) -> usize {
        self.lsp_state.hover_scroll
    }

    /// Scroll hover window down
    pub fn scroll_hover_down(&mut self, lines: usize) {
        if self.lsp_state.hover_info.is_some() {
            self.lsp_state.hover_scroll = self.lsp_state.hover_scroll.saturating_add(lines);
        }
    }

    /// Scroll hover window up
    pub fn scroll_hover_up(&mut self, lines: usize) {
        self.lsp_state.hover_scroll = self.lsp_state.hover_scroll.saturating_sub(lines);
    }

    fn document_sync_state_mut(&mut self) -> Option<&mut lsp_state::DocumentSyncState> {
        let file_path = self.buffer().file_path()?.to_string();
        Some(self.lsp_state.document_sync.entry(file_path).or_default())
    }

    /// Mark buffer as modified (for LSP didChange tracking)
    pub fn mark_buffer_modified(&mut self) {
        if let Some(state) = self.document_sync_state_mut() {
            state.mark_modified();
        }
    }

    /// Mark buffer as saved (for LSP didSave tracking)
    pub fn mark_buffer_saved(&mut self) {
        if let Some(state) = self.document_sync_state_mut() {
            state.mark_saved();
        }
    }

    /// Sends buffered text changes to LSP if modified (debounced)
    ///
    /// This sends `didChange` notifications to the LSP server when the buffer has been modified.
    /// Changes are debounced (150ms) to reduce LSP traffic during rapid typing.
    pub async fn send_lsp_changes_if_modified(&mut self) {
        let Some(ref lsp) = self.lsp_state.lsp_manager else {
            return;
        };

        let Some(file_path) = self.buffer().file_path() else {
            return;
        };

        let uri = match uri_from_file_path(file_path) {
            Some(u) => u,
            None => return,
        };

        let state_key = file_path.to_string();
        let mut should_send = false;

        // Check if we should send changes (debouncing logic)
        if let Some(state) = self.lsp_state.document_sync.get(&state_key) {
            if state.is_modified() && state.should_send_change() {
                should_send = true;
            }
        }

        if should_send {
            // Get buffer content BEFORE we update the state
            let content = self.buffer().rope().to_string();

            // Get language_id from file extension
            let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
                Some(id) => id,
                None => return,
            };

            // Send the didChange notification
            let _ = lsp.did_change(uri, &language_id, content, None).await;

            // Mark as sent AFTER sending
            let state = self.lsp_state.document_sync.entry(state_key).or_default();
            state.mark_change_sent();
        }
    }

    /// Sends didSave notification to LSP if needed
    pub async fn send_lsp_save_if_needed(&mut self) {
        let Some(ref lsp) = self.lsp_state.lsp_manager else {
            return;
        };

        let Some(file_path) = self.buffer().file_path() else {
            return;
        };

        let uri = match uri_from_file_path(file_path) {
            Some(u) => u,
            None => return,
        };

        let state_key = file_path.to_string();
        let mut should_send = false;

        // Check if we should send save notification
        if let Some(state) = self.lsp_state.document_sync.get(&state_key) {
            if state.should_send_save() {
                should_send = true;
            }
        }

        if should_send {
            // Get buffer content BEFORE we update the state
            let content = self.buffer().rope().to_string();

            // Get language_id from file extension
            let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
                Some(id) => id,
                None => return,
            };

            // Send the didSave notification
            let _ = lsp.did_save(uri, &language_id, Some(content)).await;

            // Mark as sent AFTER sending
            let state = self.lsp_state.document_sync.entry(state_key).or_default();
            state.mark_save_sent();
        }
    }

    /// Ensures the LSP server has the latest document content before making a request
    ///
    /// CRITICAL FIX: When we make a hover/goto request immediately after typing,
    /// the debounced didChange (150ms) might not have been sent yet. This causes
    /// LSP to return stale results. We flush pending changes here to ensure LSP
    /// has the latest content.
    async fn ensure_lsp_document_synced(&mut self) {
        let Some(ref lsp) = self.lsp_state.lsp_manager else {
            return;
        };

        let Some(file_path) = self.buffer().file_path() else {
            return;
        };

        let uri = match uri_from_file_path(file_path) {
            Some(u) => u,
            None => return,
        };

        let state_key = file_path.to_string();
        let mut needs_flush = false;

        // Check if we have pending changes
        if let Some(state) = self.lsp_state.document_sync.get(&state_key) {
            if state.is_modified() {
                needs_flush = true;
            }
        }

        if needs_flush {
            // Get buffer content BEFORE we update the state
            let content = self.buffer().rope().to_string();

            // Get language_id from file extension
            let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
                Some(id) => id,
                None => return,
            };

            // Send the didChange notification immediately (bypass debouncing)
            let _ = lsp.did_change(uri, &language_id, content, None).await;

            // Mark as sent
            let state = self.lsp_state.document_sync.entry(state_key).or_default();
            state.mark_change_sent();
        }
    }

    /// Sends didClose notification to LSP for the pending file
    pub async fn send_lsp_close_if_needed(&mut self) {
        let Some(file_path) = self.lsp_state.pending_did_close_file.take() else {
            return;
        };

        let Some(ref lsp) = self.lsp_state.lsp_manager else {
            return;
        };

        let uri = match uri_from_file_path(&file_path) {
            Some(u) => u,
            None => return,
        };

        // Get language_id from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path) {
            Some(id) => id,
            None => return,
        };

        let file_path_string = file_path.to_string();
        let _ = lsp.did_close(uri, &language_id).await;
        self.lsp_state.document_sync.remove(&file_path_string);
    }

    // -------------------------------------------------------------------------
    // LSP Action Processing (process pending actions from event loop)
    // -------------------------------------------------------------------------

    /// Process pending LSP actions
    /// Called from the event loop to handle LSP requests asynchronously
    pub async fn process_pending_lsp_actions(&mut self) {
        if let Some(action) = self.lsp_state.pending_lsp_action.take() {
            crate::lsp_debug!(
                "LSP-ACTION",
                "process_pending_lsp_actions() - processing action: {:?}",
                action
            );
            let result = match action {
                LspAction::GoToDefinition => self.goto_definition_impl().await,
                LspAction::GoToImplementation => self.goto_implementation_impl().await,
                LspAction::GoToType => self.goto_type_impl().await,
                LspAction::ShowHover => {
                    crate::lsp_debug!("LSP-HOVER", "About to call hover_impl()");
                    self.hover_impl().await
                }
                LspAction::Completion => self.completion_impl().await,
                LspAction::FormatDocument => self.format_document_impl().await,
                LspAction::CodeActions => self.code_actions_impl().await,
                LspAction::TypeHierarchy => self.type_hierarchy_impl().await,
                LspAction::CallHierarchyIncoming => self.call_hierarchy_incoming_impl().await,
                LspAction::CallHierarchyOutgoing => self.call_hierarchy_outgoing_impl().await,
                LspAction::FindReferences => self.find_references_impl().await,
                LspAction::DocumentSymbols => self.document_symbols_impl().await,
                LspAction::WorkspaceSymbols => self.workspace_symbols_impl().await,
                LspAction::OrganizeImports => self.organize_imports_impl().await,
                LspAction::Rename(ref new_name) => self.rename_impl(new_name.clone()).await,
                LspAction::SemanticTokens => self.semantic_tokens_impl().await,
            };

            match result {
                Ok(changed) => {
                    if !changed {
                        // Action didn't change editor state (e.g., no results)
                        // Status message should already be set
                    }
                }
                Err(e) => {
                    // LSP request failed - retry ONCE by re-queueing the action
                    // This handles race conditions where LSP server isn't ready yet
                    eprintln!("LSP action failed: {:?}, retrying once...", e);
                    if self.lsp_state.pending_lsp_action.is_none() {
                        self.lsp_state.pending_lsp_action = Some(action);
                    }
                }
            }
        }
    }

    // -------------------------------------------------------------------------
    // UTF-16 Conversion Helpers (LSP uses UTF-16 code units for positions)
    // -------------------------------------------------------------------------

    /// Converts a column position to UTF-16 code units for LSP
    ///
    /// LSP spec requires character positions in UTF-16 code units, not byte offsets.
    /// This is critical for correct positioning with rust-analyzer and other LSP servers.
    fn col_to_utf16(&self, line: usize, col: usize) -> u32 {
        let rope = self.buffer().rope();
        if line >= rope.len_lines() {
            return 0;
        }

        let line_text = rope.line(line);

        // CRITICAL: rope.line() includes the trailing newline, but LSP positions
        // should NOT include it. Exclude newline when calculating char count
        // and when iterating for UTF-16 conversion to prevent off-by-one errors
        // at end-of-line positions (hover, goto definition, etc.)
        let chars_without_newline = line_text.chars().take_while(|&c| c != '\n').count();
        let safe_col = col.min(chars_without_newline);

        // Convert to UTF-16 code units, excluding the newline
        line_text
            .chars()
            .take_while(|&c| c != '\n')
            .take(safe_col)
            .map(|c| c.len_utf16() as u32)
            .sum()
    }

    /// Converts UTF-16 code units (from LSP) back to character column position
    ///
    /// LSP responses provide positions in UTF-16 code units. This converts them
    /// back to character positions for rope operations.
    fn utf16_to_col(&self, line: usize, utf16_col: u32) -> usize {
        let rope = self.buffer().rope();
        if line >= rope.len_lines() {
            return 0;
        }

        let line_text = rope.line(line);
        let mut utf16_offset = 0u32;
        let mut char_position = 0usize;

        for ch in line_text.chars() {
            if utf16_offset >= utf16_col {
                break;
            }
            utf16_offset += ch.len_utf16() as u32;
            char_position += 1;
        }

        char_position
    }

    /// Go to definition at current cursor position via LSP (implementation)
    async fn goto_definition_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file URI - must be absolute path
        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use goto-definition".to_string());
            return Ok(false);
        };

        // Convert to absolute path if needed
        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = crate::lsp::uri_from_file_path(&abs_path)
            .ok_or_else(|| anyhow::anyhow!("Invalid file path"))?;

        // Get cursor position
        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        crate::lsp_debug!(
            "LSP-REQUEST",
            "goto_definition: file={}, line={}, col={}, char={}, uri={:?}",
            file_path,
            line,
            cursor.col(),
            character,
            uri
        );

        self.set_lsp_status("Requesting definition...".to_string());

        // Ensure document is synced before making the request
        // CRITICAL: If we just typed something, the debounced didChange might not
        // have been sent yet. We need to flush it to get correct results.
        self.ensure_lsp_document_synced().await;

        // Give LSP server a moment to process the change
        // This prevents race conditions where the request arrives before the change
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Make the request WITHOUT holding self lock (already released after ensure_lsp_document_synced)
        let result = lsp
            .goto_definition(&uri, line, character, &language_id)
            .await;

        match result {
            Ok(Some(location)) => {
                crate::lsp_debug!(
                    "LSP-RESPONSE",
                    "goto_definition: found location"
                );

                // Navigate to the location
                if let Some(path) = crate::lsp::uri_to_file_path(&location.uri) {
                    let target_line = location.range.start.line as usize;
                    let target_col = self.utf16_to_col(target_line, location.range.start.character);

                    // Save current position to jump list
                    self.add_jump();

                    // Open file if different from current
                    if self.buffer().file_path() != Some(path.to_string_lossy().as_ref()) {
                        if let Ok(()) = self.open_file(path.to_string_lossy().as_ref()) {
                            // File opened successfully
                        } else {
                            self.set_lsp_status("Failed to open file".to_string());
                            return Ok(false);
                        }
                    }

                    // Move cursor to location
                    self.buffer_mut().cursor_mut().set_position(target_line, target_col);
                    self.set_lsp_status(format!(
                        "Definition: {}:{}:{}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        target_line + 1,
                        target_col + 1
                    ));

                    Ok(true) // Changed editor state
                } else {
                    self.set_lsp_status("Invalid file path in LSP response".to_string());
                    Ok(false)
                }
            }
            Ok(None) => {
                self.set_lsp_status("No definition found".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Definition request failed: {}", e));
                Err(e)
            }
        }
    }

    async fn goto_implementation_impl(&mut self) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use goto-implementation".to_string());
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = uri_from_file_path(&abs_path).ok_or_else(|| anyhow!("Invalid file path"))?;

        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        self.set_lsp_status("Requesting implementation...".to_string());

        self.ensure_lsp_document_synced().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let result = lsp
            .implementation(&uri, line, character, &language_id)
            .await;

        match result {
            Ok(Some(location)) => {
                if let Some(path) = uri_to_file_path(&location.uri) {
                    let target_line = location.range.start.line as usize;
                    let target_col = self.utf16_to_col(target_line, location.range.start.character);

                    self.add_jump();

                    if self.buffer().file_path() != Some(path.to_string_lossy().as_ref()) {
                        if let Err(_) = self.open_file(path.to_string_lossy().as_ref()) {
                            self.set_lsp_status("Failed to open file".to_string());
                            return Ok(false);
                        }
                    }

                    self.buffer_mut().cursor_mut().set_position(target_line, target_col);
                    self.set_lsp_status(format!(
                        "Implementation: {}:{}:{}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        target_line + 1,
                        target_col + 1
                    ));

                    Ok(true)
                } else {
                    self.set_lsp_status("Invalid file path in LSP response".to_string());
                    Ok(false)
                }
            }
            Ok(None) => {
                self.set_lsp_status("No implementation found".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Implementation request failed: {}", e));
                Err(e)
            }
        }
    }

    async fn goto_type_impl(&mut self) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use goto-type".to_string());
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = uri_from_file_path(&abs_path).ok_or_else(|| anyhow!("Invalid file path"))?;

        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        self.set_lsp_status("Requesting type definition...".to_string());

        self.ensure_lsp_document_synced().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let result = lsp
            .type_definition(&uri, line, character, &language_id)
            .await;

        match result {
            Ok(Some(location)) => {

                if let Some(path) = uri_to_file_path(&location.uri) {
                    let target_line = location.range.start.line as usize;
                    let target_col = self.utf16_to_col(target_line, location.range.start.character);

                    self.add_jump();

                    if self.buffer().file_path() != Some(path.to_string_lossy().as_ref()) {
                        if let Err(_) = self.open_file(path.to_string_lossy().as_ref()) {
                            self.set_lsp_status("Failed to open file".to_string());
                            return Ok(false);
                        }
                    }

                    self.buffer_mut().cursor_mut().set_position(target_line, target_col);
                    self.set_lsp_status(format!(
                        "Type: {}:{}:{}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        target_line + 1,
                        target_col + 1
                    ));

                    Ok(true)
                } else {
                    self.set_lsp_status("Invalid file path in LSP response".to_string());
                    Ok(false)
                }
            }
            Ok(None) => {
                self.set_lsp_status("No type definition found".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Type request failed: {}", e));
                Err(e)
            }
        }
    }

    async fn find_references_impl(&mut self) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use find-references".to_string());
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = uri_from_file_path(&abs_path).ok_or_else(|| anyhow!("Invalid file path"))?;

        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        self.set_lsp_status("Finding references...".to_string());

        self.ensure_lsp_document_synced().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let result = lsp.references(&uri, line, character, &language_id, true).await;

        match result {
            Ok(locations) if !locations.is_empty() => {
                // Store references for picker navigation
                self.lsp_state.available_references = locations.clone();
                self.lsp_state.active_lsp_result_type = Some(LspResultType::References);

                // Create picker items from locations
                let items: Vec<PickerResult> = locations
                    .iter()
                    .filter_map(|loc| {
                        let path = uri_to_file_path(&loc.uri)?;
                        let line = loc.range.start.line as usize;
                        let col = self.utf16_to_col(line, loc.range.start.character);
                        Some(PickerResult {
                            display: format!("{}:{}:{}", path.file_name().unwrap_or_default().to_string_lossy(), line + 1, col + 1),
                            location: path.to_string_lossy().to_string(),
                            line,
                            col,
                        })
                    })
                    .collect();

                // Open picker with results
                self.open_location_picker(items, "References");
                self.set_lsp_status(format!("Found {} references", locations.len()));

                Ok(true)
            }
            Ok(_) => {
                self.set_lsp_status("No references found".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("References request failed: {}", e));
                Err(e)
            }
        }
    }

    async fn document_symbols_impl(&mut self) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path().map(|s| s.to_string()) else {
            self.set_lsp_status("Save file first to use document-symbols".to_string());
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(&file_path).is_absolute() {
            file_path.clone()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(&file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = uri_from_file_path(&abs_path).ok_or_else(|| anyhow!("Invalid file path"))?;

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        self.set_lsp_status("Fetching document symbols...".to_string());

        self.ensure_lsp_document_synced().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let result = lsp.document_symbols(&uri, &language_id).await;

        match result {
            Ok(symbols) if !symbols.is_empty() => {
                // Store symbols for picker navigation
                self.lsp_state.available_document_symbols = symbols.clone();
                self.lsp_state.active_lsp_result_type = Some(LspResultType::DocumentSymbols);

                // Create picker items from symbols
                let items: Vec<PickerResult> = symbols
                    .iter()
                    .filter_map(|sym| {
                        let line = sym.range.start.line as usize;
                        let col = self.utf16_to_col(line, sym.range.start.character);
                        Some(PickerResult {
                            display: format!("{}:{}:{} {}", file_path, line + 1, col + 1, sym.name),
                            location: file_path.to_string(),
                            line,
                            col,
                        })
                    })
                    .collect();

                self.open_location_picker(items, "Document Symbols");
                self.set_lsp_status(format!("Found {} symbols", symbols.len()));

                Ok(true)
            }
            Ok(_) => {
                self.set_lsp_status("No symbols found".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Document symbols request failed: {}", e));
                Err(e)
            }
        }
    }

    async fn workspace_symbols_impl(&mut self) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use workspace-symbols".to_string());
            return Ok(false);
        };

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        self.set_lsp_status("Fetching workspace symbols...".to_string());

        self.ensure_lsp_document_synced().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // TODO: Support query parameter for filtering
        let query = String::new();
        let result = lsp.workspace_symbols(&language_id, query).await;

        match result {
            Ok(symbols) if !symbols.is_empty() => {
                // Store symbols for picker navigation
                self.lsp_state.available_workspace_symbols = symbols.clone();
                self.lsp_state.active_lsp_result_type = Some(LspResultType::WorkspaceSymbols);

                // Create picker items from symbols
                let items: Vec<PickerResult> = symbols
                    .iter()
                    .filter_map(|sym| {
                        let path = uri_to_file_path(&sym.location.uri)?;
                        let line = sym.location.range.start.line as usize;
                        let col = self.utf16_to_col(line, sym.location.range.start.character);
                        Some(PickerResult {
                            display: format!("{}:{}:{}", path.file_name().unwrap_or_default().to_string_lossy(), line + 1, col + 1),
                            location: path.to_string_lossy().to_string(),
                            line,
                            col,
                        })
                    })
                    .collect();

                self.open_location_picker(items, "Workspace Symbols");
                self.set_lsp_status(format!("Found {} symbols", symbols.len()));

                Ok(true)
            }
            Ok(_) => {
                self.set_lsp_status("No workspace symbols found".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Workspace symbols request failed: {}", e));
                Err(e)
            }
        }
    }

    async fn call_hierarchy_incoming_impl(&mut self) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use call-hierarchy".to_string());
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = uri_from_file_path(&abs_path).ok_or_else(|| anyhow!("Invalid file path"))?;

        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        self.set_lsp_status("Fetching incoming calls...".to_string());

        self.ensure_lsp_document_synced().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // First prepare call hierarchy to get items at cursor position
        let items = lsp
            .prepare_call_hierarchy(uri, line, character, &language_id)
            .await;

        match items {
            Ok(Some(items)) if !items.is_empty() => {
                // Get incoming calls for the first item
                let incoming = lsp.incoming_calls(items[0].clone(), &language_id).await;

                match incoming {
                    Ok(Some(calls)) if !calls.is_empty() => {
                        // Convert incoming calls to locations
                        let locations: Vec<Location> = calls
                            .iter()
                            .map(|call| Location {
                                uri: call.from.uri.clone(),
                                range: call.from.selection_range,
                            })
                            .collect();

                // Store for navigation
                self.lsp_state.available_call_hierarchy = locations
                    .iter()
                    .map(|loc| {
                        let path = uri_to_file_path(&loc.uri)
                            .map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string())
                            .unwrap_or_default();
                        (path, loc.clone())
                    })
                    .collect();
                self.lsp_state.active_lsp_result_type = Some(LspResultType::CallHierarchy);

                // Create picker items
                let items: Vec<PickerResult> = locations
                    .iter()
                    .filter_map(|loc| {
                        let path = uri_to_file_path(&loc.uri)?;
                        let line = loc.range.start.line as usize;
                        let col = self.utf16_to_col(line, loc.range.start.character);
                        Some(PickerResult {
                            display: format!("{}:{}:{}", path.file_name().unwrap_or_default().to_string_lossy(), line + 1, col + 1),
                            location: path.to_string_lossy().to_string(),
                            line,
                            col,
                        })
                    })
                    .collect();

                        self.open_location_picker(items, "Incoming Calls");
                        self.set_lsp_status(format!("Found {} incoming calls", locations.len()));

                        Ok(true)
                    }
                    Ok(_) => {
                        self.set_lsp_status("No incoming calls found".to_string());
                        Ok(false)
                    }
                    Err(e) => {
                        self.set_lsp_status(format!("Incoming calls request failed: {}", e));
                        Err(e)
                    }
                }
            }
            Ok(_) => {
                self.set_lsp_status("Call hierarchy not available at cursor position".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Call hierarchy prepare failed: {}", e));
                Err(e)
            }
        }
    }

    async fn call_hierarchy_outgoing_impl(&mut self) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use call-hierarchy".to_string());
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
            };

        let uri = uri_from_file_path(&abs_path).ok_or_else(|| anyhow!("Invalid file path"))?;

        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        self.set_lsp_status("Fetching outgoing calls...".to_string());

        self.ensure_lsp_document_synced().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // First prepare call hierarchy to get items at cursor position
        let items = lsp
            .prepare_call_hierarchy(uri, line, character, &language_id)
            .await;

        match items {
            Ok(Some(items)) if !items.is_empty() => {
                // Get outgoing calls for the first item
                let outgoing = lsp.outgoing_calls(items[0].clone(), &language_id).await;

                match outgoing {
                    Ok(Some(calls)) if !calls.is_empty() => {
                        // Convert outgoing calls to locations
                        let locations: Vec<Location> = calls
                            .iter()
                            .map(|call| Location {
                                uri: call.to.uri.clone(),
                                range: call.to.selection_range,
                            })
                            .collect();

                // Store for navigation
                self.lsp_state.available_call_hierarchy = locations
                    .iter()
                    .map(|loc| {
                        let path = uri_to_file_path(&loc.uri)
                            .map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string())
                            .unwrap_or_default();
                        (path, loc.clone())
                    })
                    .collect();
                self.lsp_state.active_lsp_result_type = Some(LspResultType::CallHierarchy);

                // Create picker items
                let items: Vec<PickerResult> = locations
                    .iter()
                    .filter_map(|loc| {
                        let path = uri_to_file_path(&loc.uri)?;
                        let line = loc.range.start.line as usize;
                        let col = self.utf16_to_col(line, loc.range.start.character);
                        Some(PickerResult {
                            display: format!("{}:{}:{}", path.file_name().unwrap_or_default().to_string_lossy(), line + 1, col + 1),
                            location: path.to_string_lossy().to_string(),
                            line,
                            col,
                        })
                    })
                    .collect();

                        self.open_location_picker(items, "Outgoing Calls");
                        self.set_lsp_status(format!("Found {} outgoing calls", locations.len()));

                        Ok(true)
                    }
                    Ok(_) => {
                        self.set_lsp_status("No outgoing calls found".to_string());
                        Ok(false)
                    }
                    Err(e) => {
                        self.set_lsp_status(format!("Outgoing calls request failed: {}", e));
                        Err(e)
                    }
                }
            }
            Ok(_) => {
                self.set_lsp_status("Call hierarchy not available at cursor position".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Call hierarchy prepare failed: {}", e));
                Err(e)
            }
        }
    }

    async fn type_hierarchy_impl(&mut self) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use type-hierarchy".to_string());
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = uri_from_file_path(&abs_path).ok_or_else(|| anyhow!("Invalid file path"))?;

        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        self.set_lsp_status("Fetching type hierarchy...".to_string());

        self.ensure_lsp_document_synced().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // First prepare the type hierarchy to get the item at the cursor
        let prepare_result = lsp
            .prepare_type_hierarchy(uri.clone(), line, character, &language_id)
            .await;

        let items = match prepare_result {
            Ok(Some(items)) => items,
            Ok(None) => {
                self.set_lsp_status("No type hierarchy available at cursor".to_string());
                return Ok(false);
            }
            Err(e) => {
                self.set_lsp_status(format!("Type hierarchy request failed: {}", e));
                return Err(e);
            }
        };

        // Use the first item to fetch supertypes and subtypes
        let item = &items[0];

        let mut all_types = Vec::new();
        let mut all_types_data = Vec::new();

        // Fetch supertypes
        if let Ok(Some(supertypes)) = lsp.supertypes(item.clone(), &language_id).await {
            for supertype in supertypes {
                let location = Location {
                    uri: supertype.uri.clone(),
                    range: supertype.selection_range,
                };
                all_types.push(location.clone());
                all_types_data.push((format!("↑ {}", supertype.name), location));
            }
        }

        // Fetch subtypes
        if let Ok(Some(subtypes)) = lsp.subtypes(item.clone(), &language_id).await {
            for subtype in subtypes {
                let location = Location {
                    uri: subtype.uri.clone(),
                    range: subtype.selection_range,
                };
                all_types.push(location.clone());
                all_types_data.push((format!("↓ {}", subtype.name), location));
            }
        }

        if !all_types.is_empty() {
            // Store for navigation
            self.lsp_state.available_type_hierarchy = all_types_data;
            self.lsp_state.active_lsp_result_type = Some(LspResultType::TypeHierarchy);

            // Create picker items
            let items: Vec<PickerResult> = all_types
                .iter()
                .filter_map(|loc| {
                    let path = uri_to_file_path(&loc.uri)?;
                    let line = loc.range.start.line as usize;
                    let col = self.utf16_to_col(line, loc.range.start.character);
                    Some(PickerResult {
                            display: format!("{}:{}:{}", path.file_name().unwrap_or_default().to_string_lossy(), line + 1, col + 1),
                            location: path.to_string_lossy().to_string(),
                        line,
                        col,
                    })
                })
                .collect();

            self.open_location_picker(items, "Type Hierarchy");
            self.set_lsp_status(format!("Found {} types", all_types.len()));

            Ok(true)
        } else {
            self.set_lsp_status("No type hierarchy found".to_string());
            Ok(false)
        }
    }

    async fn hover_impl(&mut self) -> Result<bool> {
        crate::lsp_debug!("LSP-HOVER", "hover_impl() called");
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => {
                crate::lsp_debug!("LSP-HOVER", "LSP manager found, cloning Arc");
                lsp.clone()
            }
            None => {
                crate::lsp_debug!("LSP-HOVER", "No LSP manager in hover_impl");
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use hover".to_string());
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = uri_from_file_path(&abs_path).ok_or_else(|| anyhow!("Invalid file path"))?;

        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        crate::lsp_debug!(
            "LSP-HOVER",
            "Requesting hover: file={}, line={}, col={}, char={}, uri={:?}",
            file_path,
            line,
            cursor.col(),
            character,
            uri
        );

        self.set_lsp_status("Requesting hover info...".to_string());

        // Ensure document is synced before making the request
        self.ensure_lsp_document_synced().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let result = lsp.hover(&uri, line, character, language_id).await;

        match result {
            Ok(Some(hover_text)) => {
                self.lsp_state.hover_info = Some(hover_text);
                self.lsp_state.hover_scroll = 0; // Reset scroll position
                self.set_lsp_status(String::new()); // Clear status on success
                if self.lsp_state.hover_info.is_some() {
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            Ok(None) => {
                crate::lsp_debug!("LSP-HOVER", "No hover info returned");
                self.set_lsp_status("No hover info available".to_string());
                Ok(false)
            }
            Err(e) => {
                crate::lsp_debug!("LSP-HOVER", "Hover request failed: {:?}", e);
                self.set_lsp_status(format!("Hover request failed: {}", e));
                Err(e)
            }
        }
    }

    async fn completion_impl(&mut self) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use completion".to_string());
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = uri_from_file_path(&abs_path).ok_or_else(|| anyhow!("Invalid file path"))?;

        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        self.set_lsp_status("Requesting completions...".to_string());

        self.ensure_lsp_document_synced().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let result = lsp.completion(&uri, line, character, language_id).await;

        match result {
            Ok(items) if !items.is_empty() => {
                self.lsp_state.available_completions = items.clone();
                // TODO: Open completion popup menu
                self.set_lsp_status(format!("Found {} completions", items.len()));
                Ok(true)
            }
            Ok(_) => {
                self.set_lsp_status("No completions available".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Completion request failed: {}", e));
                Err(e)
            }
        }
    }

    async fn format_document_impl(&mut self) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use format".to_string());
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = uri_from_file_path(&abs_path).ok_or_else(|| anyhow!("Invalid file path"))?;

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        self.set_lsp_status("Formatting document...".to_string());

        self.ensure_lsp_document_synced().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Get tab settings from buffer
        let tab_size = 4; // TODO: get from config
        let insert_spaces = true; // TODO: get from config

        let result = lsp.format_document(&uri, &language_id, tab_size, insert_spaces).await;

        match result {
            Ok(edits) if !edits.is_empty() => {
                self.apply_lsp_edits(edits);
                self.set_lsp_status("Document formatted".to_string());
                Ok(true)
            }
            Ok(_) => {
                self.set_lsp_status("No formatting changes".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Format request failed: {}", e));
                Err(e)
            }
        }
    }

    async fn code_actions_impl(&mut self) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use code actions".to_string());
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = uri_from_file_path(&abs_path).ok_or_else(|| anyhow!("Invalid file path"))?;

        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        self.set_lsp_status("Fetching code actions...".to_string());

        self.ensure_lsp_document_synced().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Get diagnostics for the current line to provide context for code actions
        let diagnostics = lsp.get_diagnostics_for_line(&uri, line).await;
        let result = lsp.code_actions(&uri, line, character, language_id, diagnostics).await;

        match result {
            Ok(actions) if !actions.is_empty() => {
                self.lsp_state.available_code_actions = actions.clone();
                // TODO: Show code actions in a picker/menu
                self.set_lsp_status(format!("Found {} code actions", actions.len()));
                Ok(true)
            }
            Ok(_) => {
                self.set_lsp_status("No code actions available".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Code actions request failed: {}", e));
                Err(e)
            }
        }
    }

    /// Apply LSP text edits to the current buffer
    fn apply_lsp_edits(&mut self, edits: Vec<lsp_types::TextEdit>) {
        // Sort edits in reverse order (bottom to top) to maintain correct positions
        let mut sorted_edits = edits;
        sorted_edits.sort_by(|a, b| {
            b.range
                .start
                .line
                .cmp(&a.range.start.line)
                .then(b.range.start.character.cmp(&a.range.start.character))
        });

        for edit in sorted_edits {
            let start_line = edit.range.start.line as usize;
            let end_line = edit.range.end.line as usize;
            // Convert UTF-16 positions to character positions
            let start_col = self.utf16_to_col(start_line, edit.range.start.character);
            let end_line_for_col = end_line;
            let end_col = self.utf16_to_col(end_line_for_col, edit.range.end.character);

            // Delete the range first, then insert the new text
            self.buffer_mut().delete_range(start_line, start_col, end_line, end_col);
            self.buffer_mut().insert_text_at(start_line, start_col, &edit.new_text);
        }
    }

    /// Apply a code action by index from available code actions
    pub fn apply_code_action(&mut self, action_index: usize) {
        if action_index >= self.lsp_state.available_code_actions.len() {
            self.set_lsp_status("Invalid code action index".to_string());
            return;
        }

        let action = self.lsp_state.available_code_actions[action_index].clone();

        // Code actions can contain either:
        // 1. Direct edits (workspace edit)
        // 2. Commands to execute on the server
        match action {
            lsp_types::CodeActionOrCommand::CodeAction(code_action) => {
                let workspace_edit = match code_action.edit {
                    Some(edit) => edit,
                    None => {
                        self.set_lsp_status("Code action has no edit".to_string());
                        return;
                    }
                };

                // Apply workspace edits directly
                if let Some(changes) = &workspace_edit.changes {
                    // changes: HashMap<Uri, Vec<TextEdit>>
                    for (uri, edits) in changes {
                        // Find buffer for this URI (or open it)
                        if let Some(path) = uri_to_file_path(uri) {
                            let current_path = self.buffer().file_path().map(|s| s.to_string());

                            if current_path.as_deref() == Some(path.to_string_lossy().as_ref()) {
                                // Edits for current buffer
                                self.apply_lsp_edits(edits.clone());
                            } else {
                                // TODO: Handle edits for other files
                                // Need to load buffer, apply edits, save
                                eprintln!("Code action edits other files - not yet supported");
                            }
                        }
                    }
                }

                if let Some(document_changes) = &workspace_edit.document_changes {
                    match document_changes {
                        lsp_types::DocumentChanges::Edits(edits) => {
                            for text_doc_edit in edits {
                                if let Some(path) = uri_to_file_path(&text_doc_edit.text_document.uri) {
                                    let current_path = self.buffer().file_path().map(|s| s.to_string());

                                    if current_path.as_deref()
                                        == Some(path.to_string_lossy().as_ref())
                                    {
                                        // Extract TextEdit from OneOf
                                        let text_edits: Vec<lsp_types::TextEdit> = text_doc_edit
                                            .edits
                                            .iter()
                                            .filter_map(|e| match e {
                                                lsp_types::OneOf::Left(edit) => Some(edit.clone()),
                                                lsp_types::OneOf::Right(annot_edit) => {
                                                    Some(annot_edit.text_edit.clone())
                                                }
                                            })
                                            .collect();

                                        self.apply_lsp_edits(text_edits);
                                    } else {
                                        eprintln!("Code action edits other files - not yet supported");
                                    }
                                }
                            }
                        }
                        lsp_types::DocumentChanges::Operations(ops) => {
                            for op in ops {
                                match op {
                                    lsp_types::DocumentChangeOperation::Edit(text_doc_edit) => {
                                        if let Some(path) = uri_to_file_path(&text_doc_edit.text_document.uri)
                                        {
                                            let current_path =
                                                self.buffer().file_path().map(|s| s.to_string());

                                            if current_path.as_deref()
                                                == Some(path.to_string_lossy().as_ref())
                                            {
                                                // Extract TextEdit from OneOf
                                                let text_edits: Vec<lsp_types::TextEdit> =
                                                    text_doc_edit
                                                        .edits
                                                        .iter()
                                                        .filter_map(|e| match e {
                                                            lsp_types::OneOf::Left(edit) => {
                                                                Some(edit.clone())
                                                            }
                                                            lsp_types::OneOf::Right(annot_edit) => {
                                                                Some(annot_edit.text_edit.clone())
                                                            }
                                                        })
                                                        .collect();

                                                self.apply_lsp_edits(text_edits);
                                            }
                                        }
                                    }
                                    lsp_types::DocumentChangeOperation::Op(_resource_op) => {
                                        eprintln!("Resource operations not yet supported");
                                    }
                                }
                            }
                        }
                    }
                }

                self.set_lsp_status("Code action applied".to_string());
            }
            lsp_types::CodeActionOrCommand::Command(command) => {
                // Commands need to be executed on the LSP server
                // Requires sending workspace/executeCommand request
                let lsp = match &self.lsp_state.lsp_manager {
                    Some(lsp) => lsp.clone(),
                    None => {
                        self.set_lsp_status("LSP not available".to_string());
                        return;
                    }
                };

                // Get language_id from current file
                let language_id = match self.buffer().file_path() {
                    Some(path) => {
                        match crate::syntax::LanguageRegistry::get_lsp_language_id(path) {
                            Some(id) => id,
                            None => {
                                self.set_lsp_status("Language not supported for LSP".to_string());
                                return;
                            }
                        }
                    }
                    None => {
                        self.set_lsp_status("No file open for command execution".to_string());
                        return;
                    }
                };

                // Spawn async task to execute command
                let command_str = command.command.clone();
                let command_args = command.arguments.clone();
                tokio::spawn(async move {
                    let result = lsp.execute_command(command_str, command_args, language_id).await;
                    match result {
                        Ok(_) => eprintln!("Command executed successfully"),
                        Err(e) => eprintln!("Command execution failed: {:?}", e),
                    }
                });

                self.set_lsp_status("Executing code action command...".to_string());
            }
        }

        // Clear available actions after applying
        self.lsp_state.available_code_actions.clear();
    }

    /// Apply a completion by index from available completions
    pub fn apply_completion(&mut self, completion_index: usize) {
        if completion_index >= self.lsp_state.available_completions.len() {
            self.set_lsp_status("Invalid completion index".to_string());
            return;
        }

        let completion = self.lsp_state.available_completions[completion_index].clone();

        // Extract the text to insert
        let insert_text = if let Some(text_edit) = completion.text_edit {
            match text_edit {
                lsp_types::CompletionTextEdit::Edit(edit) => edit.new_text,
                lsp_types::CompletionTextEdit::InsertAndReplace(insert_replace) => {
                    insert_replace.new_text
                }
            }
        } else if let Some(insert_text) = completion.insert_text {
            insert_text
        } else {
            completion.label
        };

        // Insert at cursor position
        let (line, col) = {
            let cursor = self.buffer().cursor();
            (cursor.line(), cursor.col())
        };
        self.buffer_mut().insert_text_at(line, col, &insert_text);

        // Clear completions after applying
        self.lsp_state.available_completions.clear();
        self.set_lsp_status("Completion applied".to_string());
    }

    /// Navigate to an LSP location by index (from references, symbols, call hierarchy, etc.)
    pub fn navigate_to_lsp_location(&mut self, index: usize) {
        // Determine which result type we're navigating
        let result_type = match &self.lsp_state.active_lsp_result_type {
            Some(t) => t,
            None => {
                self.set_lsp_status("No LSP results available".to_string());
                return;
            }
        };

        // Get the location based on result type
        let location = match result_type {
            LspResultType::References => {
                if index >= self.lsp_state.available_references.len() {
                    self.set_lsp_status("Invalid reference index".to_string());
                    return;
                }
                self.lsp_state.available_references[index].clone()
            }
            LspResultType::DocumentSymbols => {
                if index >= self.lsp_state.available_document_symbols.len() {
                    self.set_lsp_status("Invalid symbol index".to_string());
                    return;
                }
                let symbol = &self.lsp_state.available_document_symbols[index];
                let file_path = self.buffer().file_path().expect("Document symbols require a file");
                let uri = uri_from_file_path(file_path).expect("Invalid file path");
                Location {
                    uri,
                    range: symbol.selection_range,
                }
            }
            LspResultType::WorkspaceSymbols => {
                if index >= self.lsp_state.available_workspace_symbols.len() {
                    self.set_lsp_status("Invalid symbol index".to_string());
                    return;
                }
                self.lsp_state.available_workspace_symbols[index]
                    .location
                    .clone()
            }
            LspResultType::CallHierarchy | LspResultType::TypeHierarchy => {
                let hierarchy_items = if matches!(result_type, LspResultType::CallHierarchy) {
                    &self.lsp_state.available_call_hierarchy
                } else {
                    &self.lsp_state.available_type_hierarchy
                };

                if index >= hierarchy_items.len() {
                    self.set_lsp_status("Invalid hierarchy index".to_string());
                    return;
                }

                // Extract Location from hierarchy data (stored as tuples)
                hierarchy_items[index].1.clone()
            }
        };

        // Navigate to the location
        if let Some(path) = uri_to_file_path(&location.uri) {
            let target_line = location.range.start.line as usize;
            let target_col = self.utf16_to_col(target_line, location.range.start.character);

            // Save current position to jump list
            self.add_jump();

            // Open file if different from current
            if self.buffer().file_path() != Some(path.to_string_lossy().as_ref()) {
                if let Err(_) = self.open_file(path.to_string_lossy().as_ref()) {
                    self.set_lsp_status("Failed to open file".to_string());
                    return;
                }
            }

            // Move cursor to location
            self.buffer_mut().cursor_mut().set_position(target_line, target_col);
            self.set_lsp_status(format!(
                "Navigated to {}:{}:{}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                target_line + 1,
                target_col + 1
            ));
        } else {
            self.set_lsp_status("Invalid file path in LSP response".to_string());
        }
    }

    async fn organize_imports_impl(&mut self) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to organize imports".to_string());
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = uri_from_file_path(&abs_path).ok_or_else(|| anyhow!("Invalid file path"))?;

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        self.set_lsp_status("Organizing imports...".to_string());

        self.ensure_lsp_document_synced().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Request code actions for organize imports (at file start, no diagnostics needed)
        let diagnostics = Vec::new();
        let result = lsp.code_actions(&uri, 0, 0, language_id, diagnostics).await;

        match result {
            Ok(actions) => {
                // Find organize imports action
                let organize_action = actions.into_iter().find(|action| match action {
                    lsp_types::CodeActionOrCommand::CodeAction(code_action) => {
                        // Check if this looks like an organize imports edit
                        code_action.edit.as_ref().map_or(false, |edit| {
                            edit.changes.is_some() || edit.document_changes.is_some()
                        })
                    }
                    lsp_types::CodeActionOrCommand::Command(cmd) => cmd.command.contains("organizeImports"),
                });

                if let Some(action) = organize_action {
                    // Apply the action
                    self.lsp_state.available_code_actions = vec![action];
                    self.apply_code_action(0);
                    self.set_lsp_status("Imports organized".to_string());
                    Ok(true)
                } else {
                    self.set_lsp_status("No organize imports action available".to_string());
                    Ok(false)
                }
            }
            Err(e) => {
                self.set_lsp_status(format!("Organize imports failed: {}", e));
                Err(e)
            }
        }
    }

    async fn rename_impl(&mut self, new_name: String) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use rename".to_string());
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = uri_from_file_path(&abs_path).ok_or_else(|| anyhow!("Invalid file path"))?;

        // Get cursor position (convert to UTF-16 for LSP)
        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        self.set_lsp_status(format!("Renaming to '{}'...", new_name));

        self.ensure_lsp_document_synced().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Call the rename method with individual parameters (using UTF-16 character position)
        let result = lsp.rename(&uri, line, character, language_id, new_name.to_string()).await;

        match result {
            Ok(Some(workspace_edit)) => {
                // Apply the workspace edit
                let applied = self.apply_workspace_edit(workspace_edit).await?;
                if applied {
                    self.set_lsp_status("Rename completed".to_string());
                    Ok(true)
                } else {
                    self.set_lsp_status("Rename failed to apply".to_string());
                    Ok(false)
                }
            }
            Ok(None) => {
                self.set_lsp_status("Rename not available at this location".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Rename request failed: {}", e));
                Err(e)
            }
        }
    }

    async fn semantic_tokens_impl(&mut self) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use semantic tokens".to_string());
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = uri_from_file_path(&abs_path).ok_or_else(|| anyhow!("Invalid file path"))?;

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        self.set_lsp_status("Fetching semantic tokens...".to_string());

        let result = lsp.semantic_tokens_full(&uri, language_id).await;

        match result {
            Ok(Some(_tokens)) => {
                // TODO: Store and use semantic tokens for enhanced syntax highlighting
                self.set_lsp_status("Semantic tokens received".to_string());
                Ok(true)
            }
            Ok(None) => {
                self.set_lsp_status("No semantic tokens available".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Semantic tokens request failed: {}", e));
                Err(e)
            }
        }
    }

    /// Apply a workspace edit (used for rename, organize imports, etc.)
    pub async fn apply_workspace_edit(&mut self, edit: lsp_types::WorkspaceEdit) -> Result<bool> {
        let mut all_applied = true;
        let mut modified_files = Vec::new();

        // Handle `changes` (deprecated but still widely used)
        if let Some(changes) = edit.changes {
            for (uri, text_edits) in changes {
                // Find or load the buffer for this URI
                if let Some(buffer_index) = self.find_or_load_buffer_index_by_uri(&uri) {
                    // Track modified file
                    if let Some(path) = uri_to_file_path(&uri) {
                        let file_name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown");
                        if !modified_files.contains(&file_name.to_string()) {
                            modified_files.push(file_name.to_string());
                        }
                    }

                    // Apply edits to the buffer
                    if !self.apply_lsp_edits_to_buffer_index(buffer_index, text_edits) {
                        all_applied = false;
                    }
                } else {
                    all_applied = false;
                }
            }
        }

        // Handle `document_changes` (newer, more powerful format)
        if let Some(document_changes) = edit.document_changes {
            match document_changes {
                lsp_types::DocumentChanges::Edits(edits) => {
                    for text_doc_edit in edits {
                        let uri = &text_doc_edit.text_document.uri;

                        // Find or load the buffer for this URI
                        if let Some(buffer_index) = self.find_or_load_buffer_index_by_uri(uri) {
                            // Track modified file
                            if let Some(path) = uri_to_file_path(uri) {
                                let file_name = path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("unknown");
                                if !modified_files.contains(&file_name.to_string()) {
                                    modified_files.push(file_name.to_string());
                                }
                            }

                            // Extract text edits from OneOf wrapper
                            let text_edits: Vec<lsp_types::TextEdit> = text_doc_edit
                                .edits
                                .iter()
                                .filter_map(|e| match e {
                                    lsp_types::OneOf::Left(edit) => Some(edit.clone()),
                                    lsp_types::OneOf::Right(annot_edit) => {
                                        Some(annot_edit.text_edit.clone())
                                    }
                                })
                                .collect();

                            // Apply edits to the buffer
                            if !self.apply_lsp_edits_to_buffer_index(buffer_index, text_edits) {
                                all_applied = false;
                            }
                        } else {
                            all_applied = false;
                        }
                    }
                }
                lsp_types::DocumentChanges::Operations(ops) => {
                    for op in ops {
                        match op {
                            lsp_types::DocumentChangeOperation::Edit(text_doc_edit) => {
                                let uri = &text_doc_edit.text_document.uri;

                                // Find or load the buffer for this URI
                                if let Some(buffer_index) = self.find_or_load_buffer_index_by_uri(uri) {
                                    // Track modified file
                                    if let Some(path) = uri_to_file_path(uri) {
                                        let file_name = path
                                            .file_name()
                                            .and_then(|n| n.to_str())
                                            .unwrap_or("unknown");
                                        if !modified_files.contains(&file_name.to_string()) {
                                            modified_files.push(file_name.to_string());
                                        }
                                    }

                                    // Extract text edits
                                    let text_edits: Vec<lsp_types::TextEdit> = text_doc_edit
                                        .edits
                                        .iter()
                                        .filter_map(|e| match e {
                                            lsp_types::OneOf::Left(edit) => Some(edit.clone()),
                                            lsp_types::OneOf::Right(annot_edit) => {
                                                Some(annot_edit.text_edit.clone())
                                            }
                                        })
                                        .collect();

                                    // Apply edits to the buffer
                                    if !self.apply_lsp_edits_to_buffer_index(buffer_index, text_edits) {
                                        all_applied = false;
                                    }
                                } else {
                                    all_applied = false;
                                }
                            }
                            lsp_types::DocumentChangeOperation::Op(resource_op) => {
                                // Handle resource operations (create, rename, delete files)
                                match resource_op {
                                    lsp_types::ResourceOp::Create(create_file) => {
                                        // Create a new file
                                        let file_path = match uri_to_file_path(&create_file.uri) {
                                            Some(p) => p,
                                            None => {
                                                all_applied = false;
                                                continue;
                                            }
                                        };

                                        // Check if file already exists
                                        let should_create = create_file
                                            .options
                                            .as_ref()
                                            .map(|opts| {
                                                if file_path.exists() {
                                                    opts.overwrite.unwrap_or(false)
                                                } else {
                                                    true
                                                }
                                            })
                                            .unwrap_or(!file_path.exists());

                                        if should_create {
                                            if std::fs::write(&file_path, "").is_err() {
                                                all_applied = false;
                                            }
                                        }
                                    }
                                    lsp_types::ResourceOp::Rename(rename_file) => {
                                        // Rename/move a file
                                        let old_path = match uri_to_file_path(&rename_file.old_uri) {
                                            Some(p) => p,
                                            None => {
                                                all_applied = false;
                                                continue;
                                            }
                                        };
                                        let new_path = match uri_to_file_path(&rename_file.new_uri) {
                                            Some(p) => p,
                                            None => {
                                                all_applied = false;
                                                continue;
                                            }
                                        };

                                        // Create parent directories if needed
                                        if let Some(parent) = new_path.parent() {
                                            if !parent.exists() {
                                                if std::fs::create_dir_all(parent).is_err() {
                                                    all_applied = false;
                                                    continue;
                                                }
                                            }
                                        }

                                        // Perform the rename
                                        if std::fs::rename(&old_path, &new_path).is_err() {
                                            all_applied = false;
                                        }
                                    }
                                    lsp_types::ResourceOp::Delete(delete_file) => {
                                        // Delete a file
                                        let file_path = match uri_to_file_path(&delete_file.uri) {
                                            Some(p) => p,
                                            None => {
                                                all_applied = false;
                                                continue;
                                            }
                                        };

                                        if file_path.exists() {
                                            if std::fs::remove_file(&file_path).is_err() {
                                                all_applied = false;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Show summary of changes
        if !modified_files.is_empty() {
            let summary = if modified_files.len() == 1 {
                format!("Modified {}", modified_files[0])
            } else {
                format!("Modified {} files", modified_files.len())
            };
            self.set_lsp_status(summary);
        }

        Ok(all_applied)
    }

    /// Helper method to open a location picker with LSP results
    fn open_location_picker(&mut self, items: Vec<PickerResult>, title: &str) {
        use std::path::PathBuf;

        let base_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let display_items: Vec<String> = items.iter().map(|r| r.display.clone()).collect();

        let picker = Picker::new_lsp_locations(base_dir, display_items);
        self.set_picker(picker);
        self.set_mode(Mode::Picker);
        self.mark_picker_selection_changed();
    }
}
