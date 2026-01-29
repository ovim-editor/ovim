//! LSP Integration for Editor
//!
//! This module contains all LSP-related functionality extracted from the main editor module.
//! It provides LSP initialization, document synchronization, LSP actions, and workspace editing.

// Submodules for focused functionality
#[path = "lsp_modules/mod.rs"]
mod lsp_modules;

use super::*;
use crate::lsp::{LspManager, uri_from_file_path};

use anyhow::{anyhow, Result};
use lsp_types::Location;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Context for making an LSP request, encapsulating all the common setup.
pub(in crate::editor) struct LspRequestContext {
    pub lsp: Arc<crate::lsp::LspManager>,
    pub uri: lsp_types::Uri,
    pub file_path: String,
    pub line: u32,
    pub character: u32,
    pub language_id: String,
}

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
        let _ = lsp.did_close(uri, language_id).await;
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

    /// Marks a document as having sent didOpen notification
    /// Used by LSP pre-warming to prevent duplicate didOpen
    pub fn mark_document_opened(&mut self, file_path: &str) {
        let state = self.lsp_state.document_sync.entry(file_path.to_string()).or_default();
        state.did_open_sent = true;
    }

    /// Request LSP initialization for the current file
    pub fn request_lsp_init(&mut self) {
        self.lsp_state.needs_lsp_init = true;
    }

    /// Set LSP status message
    pub fn set_lsp_status(&mut self, status: String) {
        self.lsp_state.lsp_status = status;
    }

    /// Get current LSP status
    pub fn lsp_status(&self) -> &str {
        &self.lsp_state.lsp_status
    }

    /// Invalidate hover cache when buffer is modified
    pub fn invalidate_hover_cache(&mut self) {
        if self.lsp_state.hover_cache.is_some() {
            self.lsp_state.hover_cache = None;
        }
    }

    /// Returns true if there's a pending LSP response being waited for
    pub fn has_pending_lsp_response(&self) -> bool {
        self.lsp_state.pending_lsp_response.is_some()
    }

    /// Polls pending LSP responses (non-blocking)
    /// Returns true if a response was processed and UI should redraw
    pub fn poll_pending_lsp_responses(&mut self) -> bool {
        let Some(pending) = self.lsp_state.pending_lsp_response.take() else {
            return false; // No pending request
        };

        match pending {
            crate::editor::lsp_state::PendingLspResponse::Hover(mut pending) => {
                use tokio::sync::oneshot::error::TryRecvError;
                match pending.receiver.try_recv() {
                    Ok(Ok(Some(hover_text))) => {
                        // Success! Set hover info and cache
                        crate::lsp_debug!("LSP-HOVER", "Received hover response");

                        // Get current position and buffer version for caching
                        let cursor = self.buffer().cursor();
                        let buffer_version = self.buffer().version();
                        let cursor_line = cursor.line();
                        let cursor_col = cursor.col();
                        let file_path = self.buffer().file_path().unwrap_or("").to_string();

                        // Cache the hover result
                        self.lsp_state.hover_cache = Some(crate::editor::lsp_state::HoverCache::new(
                            file_path,
                            cursor_line,
                            cursor_col,
                            buffer_version,
                            hover_text.clone(),
                        ));

                        self.lsp_state.hover_info = Some(hover_text);
                        self.lsp_state.hover_scroll = 0;
                        self.lsp_state.hover_position = Some((cursor_line, cursor_col));
                        self.lsp_state.hover_content_type = crate::editor::lsp_state::HoverContentType::LspHover;
                        self.mode = crate::mode::Mode::HoverPreview;
                        self.mark_dirty();
                        self.set_lsp_status(String::new());
                        true // UI should redraw
                    }
                    Ok(Ok(None)) => {
                        // No hover info available
                        crate::lsp_debug!("LSP-HOVER", "No hover info available");
                        self.set_lsp_status("No hover info available".to_string());
                        false
                    }
                    Ok(Err(e)) => {
                        // LSP error
                        crate::lsp_debug!("LSP-HOVER", "Hover request failed: {:?}", e);
                        self.set_lsp_status(format!("Hover failed: {}", e));
                        false
                    }
                    Err(TryRecvError::Empty) => {
                        // Check for timeout
                        if pending.started.elapsed() > std::time::Duration::from_secs(10) {
                            crate::lsp_debug!("LSP-HOVER", "Hover request timed out, aborting task");
                            pending.task.abort();
                            self.set_lsp_status("Hover request timed out".to_string());
                            return false;
                        }

                        // Still waiting - put it back
                        self.lsp_state.pending_lsp_response =
                            Some(crate::editor::lsp_state::PendingLspResponse::Hover(pending));
                        false
                    }
                    Err(TryRecvError::Closed) => {
                        // Sender dropped (shouldn't happen)
                        crate::lsp_debug!("LSP-HOVER", "Hover request cancelled (sender dropped)");
                        self.set_lsp_status("Hover request cancelled".to_string());
                        false
                    }
                }
            }

            crate::editor::lsp_state::PendingLspResponse::Definition(pending) => {
                self.poll_location_response(pending, "Definition", "LSP-DEFINITION", false)
            }

            crate::editor::lsp_state::PendingLspResponse::DefinitionNewTab(pending) => {
                self.poll_location_response(pending, "Definition", "LSP-DEFINITION", true)
            }

            crate::editor::lsp_state::PendingLspResponse::Implementation(pending) => {
                self.poll_location_response(pending, "Implementation", "LSP-IMPLEMENTATION", false)
            }

            crate::editor::lsp_state::PendingLspResponse::TypeDefinition(pending) => {
                self.poll_location_response(pending, "Type", "LSP-TYPE", false)
            }
        }
    }

    /// Shared handler for location-based pending LSP responses (definition, implementation, type).
    fn poll_location_response(
        &mut self,
        mut pending: crate::editor::lsp_state::PendingLspRequest<Option<Location>>,
        label: &str,
        log_tag: &str,
        new_tab: bool,
    ) -> bool {
        use tokio::sync::oneshot::error::TryRecvError;

        match pending.receiver.try_recv() {
            Ok(Ok(Some(location))) => {
                crate::lsp_debug!(log_tag, "Received {} response", label.to_lowercase());

                let Some(path) = crate::lsp::uri_to_file_path(&location.uri) else {
                    self.set_lsp_status("Invalid file path in LSP response".to_string());
                    return false;
                };

                let target_line = location.range.start.line as usize;
                let target_character = location.range.start.character;

                self.push_tag();

                if new_tab {
                    self.new_tab(Some(path.to_string_lossy().to_string()));
                    match crate::buffer::Buffer::load_file(&path) {
                        Ok(buffer) => {
                            self.buffers[self.current_buffer_index] = buffer;
                        }
                        Err(_) => {
                            self.set_lsp_status("Failed to open file".to_string());
                            return false;
                        }
                    }
                } else if self.buffer().file_path() != Some(path.to_string_lossy().as_ref())
                    && self.open_file(path.to_string_lossy().as_ref()).is_err()
                {
                    self.set_lsp_status("Failed to open file".to_string());
                    return false;
                }

                let target_col = self.utf16_to_col(target_line, target_character);
                self.buffer_mut().cursor_mut().set_position(target_line, target_col);
                self.buffer_mut().validate_cursor_position();
                self.center_cursor_in_viewport();
                let actual_col = self.buffer().cursor().col();

                let suffix = if new_tab { " (new tab)" } else { "" };
                self.set_lsp_status(format!(
                    "{}{}: {}:{}:{}",
                    label, suffix,
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    target_line + 1,
                    actual_col + 1
                ));
                self.mark_dirty();
                true
            }
            Ok(Ok(None)) => {
                crate::lsp_debug!(log_tag, "No {} found", label.to_lowercase());
                self.set_lsp_status(format!("No {} found", label.to_lowercase()));
                false
            }
            Ok(Err(e)) => {
                crate::lsp_debug!(log_tag, "{} request failed: {:?}", label, e);
                self.set_lsp_status(format!("{} failed: {}", label, e));
                false
            }
            Err(TryRecvError::Empty) => {
                if pending.started.elapsed() > std::time::Duration::from_secs(10) {
                    crate::lsp_debug!(log_tag, "{} request timed out, aborting task", label);
                    pending.task.abort();
                    self.set_lsp_status(format!("{} request timed out", label));
                    return false;
                }
                // Still waiting - reconstruct the appropriate variant and put it back
                let variant = if new_tab && label == "Definition" {
                    crate::editor::lsp_state::PendingLspResponse::DefinitionNewTab(pending)
                } else if label == "Definition" {
                    crate::editor::lsp_state::PendingLspResponse::Definition(pending)
                } else if label == "Implementation" {
                    crate::editor::lsp_state::PendingLspResponse::Implementation(pending)
                } else {
                    crate::editor::lsp_state::PendingLspResponse::TypeDefinition(pending)
                };
                self.lsp_state.pending_lsp_response = Some(variant);
                false
            }
            Err(TryRecvError::Closed) => {
                crate::lsp_debug!(log_tag, "{} request cancelled (sender dropped)", label);
                self.set_lsp_status(format!("{} request cancelled", label));
                false
            }
        }
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

    /// Queue an LSP action and reset the retry count
    fn queue_lsp_action(&mut self, action: LspAction) {
        self.lsp_state.pending_lsp_action = Some(action);
        self.lsp_state.lsp_action_retry_count = 0;
    }


    /// Request document format
    pub fn request_format_document(&mut self) {
        self.queue_lsp_action(LspAction::FormatDocument);
    }

    /// Request code actions at current cursor position
    pub fn request_code_actions(&mut self) {
        self.queue_lsp_action(LspAction::CodeActions);
    }

    /// Request call hierarchy (incoming calls) at current cursor position
    pub fn request_call_hierarchy_incoming(&mut self) {
        self.queue_lsp_action(LspAction::CallHierarchyIncoming);
    }

    /// Request call hierarchy (outgoing calls) at current cursor position
    pub fn request_call_hierarchy_outgoing(&mut self) {
        self.queue_lsp_action(LspAction::CallHierarchyOutgoing);
    }

    /// Request type hierarchy at current cursor position
    pub fn request_type_hierarchy(&mut self) {
        self.queue_lsp_action(LspAction::TypeHierarchy);
    }

    /// Request organize imports for the current document
    pub fn request_organize_imports(&mut self) {
        self.queue_lsp_action(LspAction::OrganizeImports);
    }

    /// Request find references at current cursor position
    pub fn request_find_references(&mut self) {
        self.queue_lsp_action(LspAction::FindReferences);
    }

    /// Request document symbols for the current document
    pub fn request_document_symbols(&mut self) {
        self.queue_lsp_action(LspAction::DocumentSymbols);
    }

    /// Request workspace symbols
    pub fn request_workspace_symbols(&mut self) {
        self.queue_lsp_action(LspAction::WorkspaceSymbols);
    }

    /// Request rename at current cursor position
    pub fn request_rename(&mut self, new_name: String) {
        self.queue_lsp_action(LspAction::Rename(new_name));
    }

    /// Request semantic tokens for the current document
    pub fn request_semantic_tokens(&mut self) {
        self.queue_lsp_action(LspAction::SemanticTokens);
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

            // Get old content for incremental sync
            let old_content = self.lsp_state.document_sync
                .get(&state_key)
                .and_then(|state| state.last_synced_content.clone());

            // Send the didChange notification with old_text for incremental sync
            let _ = lsp.did_change(uri, language_id, content.clone(), old_content).await;

            // Mark as sent AFTER sending and store the synced content
            let state = self.lsp_state.document_sync.entry(state_key).or_default();
            state.mark_change_sent(content);
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
            let _ = lsp.did_save(uri, language_id, Some(content)).await;

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
    async fn ensure_lsp_document_synced(&mut self) -> bool {
        let Some(ref lsp) = self.lsp_state.lsp_manager else {
            return false;
        };

        let Some(file_path) = self.buffer().file_path() else {
            return false;
        };

        let uri = match uri_from_file_path(file_path) {
            Some(u) => u,
            None => return false,
        };

        let state_key = file_path.to_string();

        // Get language_id from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => return false,
        };

        // Get buffer content
        let content = self.buffer().rope().to_string();

        // Check if we need to send didOpen first (CRITICAL for LSP protocol)
        let needs_did_open = self
            .lsp_state
            .document_sync
            .get(&state_key)
            .is_none_or(|state| !state.did_open_sent);

        if needs_did_open {
            // Send didOpen notification (uri, language_id, version, text)
            let _ = lsp
                .did_open(uri.clone(), language_id, 1, content.clone())
                .await;

            // Mark didOpen as sent
            let state = self.lsp_state.document_sync.entry(state_key.clone()).or_default();
            state.did_open_sent = true;
            state.mark_change_sent(content.clone());
            return true; // We sent didOpen (includes content flush)
        }

        // Check if we have pending changes
        let needs_flush = self
            .lsp_state
            .document_sync
            .get(&state_key)
            .is_some_and(|state| state.is_modified());

        if needs_flush {
            // Get old content for incremental sync
            let old_content = self.lsp_state.document_sync
                .get(&state_key)
                .and_then(|state| state.last_synced_content.clone());

            // Send the didChange notification immediately (bypass debouncing) with incremental sync
            let _ = lsp.did_change(uri, language_id, content.clone(), old_content).await;

            // Mark as sent and store synced content
            let state = self.lsp_state.document_sync.entry(state_key).or_default();
            state.mark_change_sent(content);
            return true; // We flushed changes
        }

        false // No flush needed
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
        let _ = lsp.did_close(uri, language_id).await;
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
                LspAction::GoToDefinitionNewTab => self.goto_definition_new_tab_impl().await,
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
                    if changed {
                        // Action changed editor state (e.g., jumped to definition)
                        // Mark dirty to trigger redraw
                        self.mark_dirty();
                    } else {
                        // Action didn't change editor state (e.g., no results)
                        // Status message should already be set
                    }
                }
                Err(_e) => {
                    // LSP request failed - retry ONCE by re-queueing the action
                    // This handles race conditions where LSP server isn't ready yet
                    // Only retry if we haven't already retried (prevents infinite loop)
                    if self.lsp_state.lsp_action_retry_count < 1 {
                        self.lsp_state.lsp_action_retry_count += 1;
                        if self.lsp_state.pending_lsp_action.is_none() {
                            self.lsp_state.pending_lsp_action = Some(action);
                        }
                    }
                    // If retry_count >= 1, we've already retried once, so give up silently
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

    /// Prepare common context for an LSP request.
    /// Handles: LSP manager check, file path resolution, URI creation,
    /// cursor position (UTF-16), language detection, and document sync flush.
    pub(in crate::editor) async fn prepare_lsp_request(
        &mut self,
        feature_name: &str,
    ) -> Result<LspRequestContext> {
        let lsp = self
            .lsp_state
            .lsp_manager
            .clone()
            .ok_or_else(|| anyhow!("LSP not available"))?;

        let file_path = self
            .buffer()
            .file_path()
            .ok_or_else(|| anyhow!("Save file first to use {}", feature_name))?
            .to_string();

        let abs_path = if std::path::Path::new(&file_path).is_absolute() {
            file_path.clone()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(&file_path).to_string_lossy().to_string())
                .map_err(|_| anyhow!("Failed to resolve file path"))?
        };

        let uri = crate::lsp::uri_from_file_path(&abs_path)
            .ok_or_else(|| anyhow!("Invalid file path"))?;

        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        let language_id = crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path)
            .ok_or_else(|| anyhow!("Language not supported for LSP"))?
            .to_string();

        // Flush pending document changes so LSP has the latest content
        let did_flush = self.ensure_lsp_document_synced().await;
        if did_flush {
            tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        Ok(LspRequestContext {
            lsp,
            uri,
            file_path,
            line,
            character,
            language_id,
        })
    }

}
