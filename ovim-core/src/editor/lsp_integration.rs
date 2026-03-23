//! LSP Integration for Editor
//!
//! This module contains all LSP-related functionality extracted from the main editor module.
//! It provides LSP initialization, document synchronization, LSP actions, and workspace editing.

// Submodules for focused functionality
#[path = "lsp_modules/mod.rs"]
mod lsp_modules;

use super::*;
use crate::lsp::{uri_from_file_path, LspManager};

use anyhow::{anyhow, Result};
use lsp_types::Location;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

fn dedupe_key_for_status(status_lower: &str) -> String {
    status_lower
        .split(':')
        .next()
        .unwrap_or(status_lower)
        .trim()
        .to_string()
}

fn is_lsp_toast_candidate(status_lower: &str) -> bool {
    status_lower.starts_with("lsp:")
        || status_lower.starts_with("java:")
        || status_lower.contains("completion")
        || status_lower.contains("hover")
        || status_lower.contains("definition")
        || status_lower.contains("implementation")
        || status_lower.contains("code action")
        || status_lower.contains("semantic token")
        || status_lower.contains("workspace edit")
        || status_lower.contains("organize imports")
        || status_lower.contains("rename")
        || status_lower.contains("diagnostic")
}

fn classify_status_toast(status: &str) -> Option<(ToastLevel, Option<Duration>, bool, String)> {
    if status.is_empty() {
        return None;
    }

    let lower = status.to_ascii_lowercase();
    if !is_lsp_toast_candidate(&lower) {
        return None;
    }

    if lower.contains("failed") || lower.contains("error") {
        return Some((ToastLevel::Error, None, true, dedupe_key_for_status(&lower)));
    }

    if lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("cancelled")
        || lower.contains("canceled")
    {
        return Some((
            ToastLevel::Warning,
            Some(Duration::from_secs(6)),
            false,
            dedupe_key_for_status(&lower),
        ));
    }

    None
}

/// Context for making an LSP request, encapsulating all the common setup.
pub(in crate::editor) struct LspRequestContext {
    pub lsp: Arc<crate::lsp::LspManager>,
    pub uri: lsp_types::Uri,
    pub file_path: String,
    pub line: u32,
    pub character: u32,
    pub language_id: String,
    /// All server_ids serving this language (primary + companions)
    pub server_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DocumentSyncRequestAction {
    Noop,
    DidOpen,
    QueueChangeAndFlush,
    FlushQueued,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DocumentSyncRequestPlan {
    action: DocumentSyncRequestAction,
    old_content: Option<String>,
}

impl Editor {
    /// Enables LSP support
    pub fn enable_lsp(&mut self) {
        let (tx, rx) = mpsc::unbounded_channel();
        self.lsp.state.lsp_manager = Some(Arc::new(LspManager::new()));
        self.lsp.command_tx = Some(tx);
        self.lsp.command_rx = Some(rx);
    }

    /// Gets a reference to the LSP manager
    pub fn lsp_manager(&self) -> Option<Arc<LspManager>> {
        self.lsp.state.lsp_manager.clone()
    }

    /// Gets a reference to the LSP command sender for background tasks
    pub fn lsp_command_sender(&self) -> Option<mpsc::UnboundedSender<LspCommand>> {
        self.lsp.command_tx.clone()
    }

    /// Close the LSP for the current file
    pub async fn close_current_file_lsp(&mut self) {
        let Some(ref lsp) = self.lsp.state.lsp_manager else {
            return;
        };

        let Some(file_path) = self.buffer().file_path().map(|p| p.to_string()) else {
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

        // Send LSP close notification
        let file_path_string = file_path.to_string();
        let _ = lsp.did_close_broadcast(uri, language_id).await;
        self.lsp.state.document_sync.remove(&file_path_string);
    }

    /// Check if LSP initialization is needed for the current file
    pub fn needs_lsp_init(&self) -> Option<String> {
        if self.lsp.state.needs_lsp_init {
            self.buffer().file_path().map(|s| s.to_string())
        } else {
            None
        }
    }

    /// Clear the LSP initialization flag after init is complete
    pub fn clear_lsp_init_flag(&mut self) {
        self.lsp.state.needs_lsp_init = false;
    }

    /// Marks a document as having sent didOpen notification
    /// Used by LSP pre-warming to prevent duplicate didOpen
    pub fn mark_document_opened(&mut self, file_path: &str) {
        let state = self
            .lsp.state
            .document_sync
            .entry(file_path.to_string())
            .or_default();
        state.did_open_sent = true;
    }

    /// Marks a document as opened and synced (didOpen sent with this exact content).
    pub fn mark_document_opened_with_content(&mut self, file_path: &str, content: String) {
        self.mark_document_flushed(file_path, content, 1);
    }

    /// Request LSP initialization for the current file
    pub fn request_lsp_init(&mut self) {
        self.lsp.state.needs_lsp_init = true;
    }

    /// Set LSP status message
    pub fn set_lsp_status(&mut self, status: String) {
        self.lsp.state.lsp_status = status.clone();

        if let Some((level, ttl, sticky, dedupe_key)) = classify_status_toast(&status) {
            let request = ToastRequest::new(ToastSource::Lsp, level, status)
                .with_title("LSP")
                .with_ttl(ttl)
                .with_sticky(sticky)
                .with_dedupe_key(format!("lsp:{dedupe_key}"));
            self.push_toast(request);
        }
    }

    /// Check if there's a pending LSP install awaiting user consent
    pub fn has_pending_lsp_install(&self) -> bool {
        self.lsp.pending_install.is_some()
    }

    /// Get a summary of the pending LSP install for display
    pub fn pending_lsp_install_summary(&self) -> Option<(String, String, String)> {
        self.lsp.pending_install.as_ref().map(|p| {
            (
                p.language_name.clone(),
                p.server_command.clone(),
                p.method_description.clone(),
            )
        })
    }

    /// Resolve the pending LSP install consent dialog
    pub fn resolve_pending_lsp_install(&mut self, consent: super::LspInstallConsent) {
        let pending = self.lsp.pending_install.take();
        match consent {
            super::LspInstallConsent::Yes => {
                if let Some(p) = &pending {
                    self.set_lsp_status(format!("LSP: Installing {}...", p.server_command));
                }
                // The actual install is triggered by the event loop checking
                // lsp_install_approved. Store the approved info.
                self.lsp.approved_install = pending;
            }
            super::LspInstallConsent::Always => {
                self.options.lsp_auto_install = super::AutoInstallMode::Auto;
                if let Some(p) = &pending {
                    self.set_lsp_status(format!("LSP: Installing {}...", p.server_command));
                }
                self.lsp.approved_install = pending;
            }
            super::LspInstallConsent::No => {
                if let Some(p) = &pending {
                    self.set_lsp_status(format!(
                        "LSP: {} skipped. Use :set autoinstall=prompt to re-enable.",
                        p.server_command
                    ));
                }
            }
        }
    }

    /// Take the approved LSP install info (consumed by the event loop)
    pub fn take_approved_lsp_install(&mut self) -> Option<super::PendingLspInstall> {
        self.lsp.approved_install.take()
    }

    /// Get current LSP status
    pub fn lsp_status(&self) -> &str {
        &self.lsp.state.lsp_status
    }

    fn document_sync_request_plan(
        &self,
        file_path: &str,
        current_content: &str,
    ) -> DocumentSyncRequestPlan {
        let Some(state) = self.lsp.state.document_sync.get(file_path) else {
            return DocumentSyncRequestPlan {
                action: DocumentSyncRequestAction::DidOpen,
                old_content: None,
            };
        };

        if !state.did_open_sent {
            return DocumentSyncRequestPlan {
                action: DocumentSyncRequestAction::DidOpen,
                old_content: None,
            };
        }

        let queued_current = state.queued_content() == Some(current_content);
        if state.is_modified() && queued_current {
            return DocumentSyncRequestPlan {
                action: DocumentSyncRequestAction::FlushQueued,
                old_content: None,
            };
        }

        let flushed_current = state.flushed_content() == Some(current_content);
        if state.is_modified() || !flushed_current {
            return DocumentSyncRequestPlan {
                action: DocumentSyncRequestAction::QueueChangeAndFlush,
                old_content: state.last_flushed_content.clone(),
            };
        }

        DocumentSyncRequestPlan {
            action: DocumentSyncRequestAction::Noop,
            old_content: None,
        }
    }

    fn mark_document_flushed(&mut self, file_path: &str, content: String, flushed_version: i32) {
        let current_content = self
            .buffer()
            .file_path()
            .filter(|path| *path == file_path)
            .map(|_| self.buffer().rope().to_string());
        let state = self
            .lsp.state
            .document_sync
            .entry(file_path.to_string())
            .or_default();
        state.did_open_sent = true;
        state.mark_change_flushed(content, flushed_version, current_content.as_deref());
    }

    /// Get the currently queued LSP action, if any.
    pub fn pending_lsp_action(&self) -> Option<&LspAction> {
        self.lsp.state.pending_lsp_action.as_ref()
    }

    /// Invalidate hover cache when buffer is modified
    pub fn invalidate_hover_cache(&mut self) {
        if self.lsp.state.hover_cache.is_some() {
            self.lsp.state.hover_cache = None;
        }
    }

    /// Returns true if there's a pending LSP response being waited for
    pub fn has_pending_lsp_response(&self) -> bool {
        self.lsp.state.pending_lsp_responses.any_pending()
    }

    pub fn has_pending_completion_response(&self) -> bool {
        self.lsp.state.pending_completion.is_some()
    }

    pub fn has_pending_inlay_hint_response(&self) -> bool {
        self.lsp.state.pending_inlay_hints.is_some()
    }

    /// Polls pending LSP responses (non-blocking)
    /// Returns true if a response was processed and UI should redraw
    ///
    /// Returns true if a hover response is pending (spawned but not yet received).
    pub fn has_pending_hover(&self) -> bool {
        self.lsp.state.pending_lsp_responses.hover.is_some()
    }

    /// Each response type is polled independently so that e.g. a hover request
    /// doesn't block or clobber a goto-definition request.
    pub fn poll_pending_lsp_responses(&mut self) -> bool {
        let mut changed = false;

        // --- Poll hover ---
        if self.lsp.state.pending_lsp_responses.hover.is_some() {
            changed |= self.poll_hover_slot();
        }

        // --- Poll definition ---
        if self.lsp.state.pending_lsp_responses.definition.is_some() {
            changed |= self.poll_definition_slot();
        }

        // --- Poll implementation ---
        if self
            .lsp.state
            .pending_lsp_responses
            .implementation
            .is_some()
        {
            changed |= self.poll_implementation_slot();
        }

        // --- Poll type_definition ---
        if self
            .lsp.state
            .pending_lsp_responses
            .type_definition
            .is_some()
        {
            changed |= self.poll_type_definition_slot();
        }

        changed
    }

    /// Poll the hover response slot.
    fn poll_hover_slot(&mut self) -> bool {
        use tokio::sync::oneshot::error::TryRecvError;

        let Some(ref mut pending) = self.lsp.state.pending_lsp_responses.hover else {
            return false;
        };

        match pending.receiver.try_recv() {
            Ok(Ok(Some(hover_text))) => {
                // Take ownership now that we know we have a result
                let _pending = self.lsp.state.pending_lsp_responses.hover.take().unwrap();

                crate::lsp_debug!("LSP-HOVER", "Received hover response");

                let cursor = self.buffer().cursor();
                let buffer_version = self.buffer().version();
                let cursor_line = cursor.line();
                let cursor_col = cursor.col();
                let file_path = self.buffer().file_path().unwrap_or("").to_string();

                self.lsp.state.hover_cache = Some(crate::editor::lsp_state::HoverCache::new(
                    file_path,
                    cursor_line,
                    cursor_col,
                    buffer_version,
                    hover_text.clone(),
                ));

                self.lsp.state.hover_info = Some(hover_text);
                self.lsp.state.hover_scroll = 0;
                self.lsp.state.hover_h_scroll = 0;
                self.lsp.state.hover_position = Some((cursor_line, cursor_col));
                self.lsp.state.hover_content_type =
                    crate::editor::lsp_state::HoverContentType::LspHover;
                self.mode = crate::mode::Mode::HoverPreview;
                self.mark_dirty();
                self.set_lsp_status(String::new());
                true
            }
            Ok(Ok(None)) => {
                let _pending = self.lsp.state.pending_lsp_responses.hover.take().unwrap();
                crate::lsp_debug!("LSP-HOVER", "No hover info available");
                self.set_lsp_status("No hover info available".to_string());
                false
            }
            Ok(Err(e)) => {
                let _pending = self.lsp.state.pending_lsp_responses.hover.take().unwrap();
                crate::lsp_debug!("LSP-HOVER", "Hover request failed: {:?}", e);
                self.set_lsp_status(format!("Hover failed: {}", e));
                false
            }
            Err(TryRecvError::Empty) => {
                // Check for timeout (re-borrow since we still hold the slot)
                let timed_out = self
                    .lsp.state
                    .pending_lsp_responses
                    .hover
                    .as_ref()
                    .is_some_and(|p| p.started.elapsed() > std::time::Duration::from_secs(10));
                if timed_out {
                    let pending = self.lsp.state.pending_lsp_responses.hover.take().unwrap();
                    crate::lsp_debug!("LSP-HOVER", "Hover request timed out, aborting task");
                    pending.task.abort();
                    self.set_lsp_status("Hover request timed out".to_string());
                }
                // Otherwise: still waiting, leave the slot in place
                false
            }
            Err(TryRecvError::Closed) => {
                let _pending = self.lsp.state.pending_lsp_responses.hover.take().unwrap();
                crate::lsp_debug!("LSP-HOVER", "Hover request cancelled (sender dropped)");
                self.set_lsp_status("Hover request cancelled".to_string());
                false
            }
        }
    }

    /// Poll the definition response slot.
    fn poll_definition_slot(&mut self) -> bool {
        use tokio::sync::oneshot::error::TryRecvError;

        let Some((_, ref mut req)) = self.lsp.state.pending_lsp_responses.definition else {
            return false;
        };

        match req.receiver.try_recv() {
            Ok(result) => {
                let (new_tab, pending) = self
                    .lsp.state
                    .pending_lsp_responses
                    .definition
                    .take()
                    .unwrap();
                self.handle_location_result(
                    result,
                    pending,
                    "Definition",
                    "LSP-DEFINITION",
                    new_tab,
                )
            }
            Err(TryRecvError::Empty) => {
                let timed_out = self
                    .lsp.state
                    .pending_lsp_responses
                    .definition
                    .as_ref()
                    .is_some_and(|(_, p)| p.started.elapsed() > std::time::Duration::from_secs(10));
                if timed_out {
                    let (_, pending) = self
                        .lsp.state
                        .pending_lsp_responses
                        .definition
                        .take()
                        .unwrap();
                    crate::lsp_debug!(
                        "LSP-DEFINITION",
                        "Definition request timed out, aborting task"
                    );
                    pending.task.abort();
                    self.set_lsp_status("Definition request timed out".to_string());
                }
                false
            }
            Err(TryRecvError::Closed) => {
                let _pending = self.lsp.state.pending_lsp_responses.definition.take();
                crate::lsp_debug!(
                    "LSP-DEFINITION",
                    "Definition request cancelled (sender dropped)"
                );
                self.set_lsp_status("Definition request cancelled".to_string());
                false
            }
        }
    }

    /// Poll the implementation response slot.
    fn poll_implementation_slot(&mut self) -> bool {
        use tokio::sync::oneshot::error::TryRecvError;

        let Some((_, ref mut req)) = self.lsp.state.pending_lsp_responses.implementation else {
            return false;
        };

        match req.receiver.try_recv() {
            Ok(result) => {
                let (new_tab, pending) = self
                    .lsp.state
                    .pending_lsp_responses
                    .implementation
                    .take()
                    .unwrap();
                self.handle_location_result(
                    result,
                    pending,
                    "Implementation",
                    "LSP-IMPLEMENTATION",
                    new_tab,
                )
            }
            Err(TryRecvError::Empty) => {
                let timed_out = self
                    .lsp.state
                    .pending_lsp_responses
                    .implementation
                    .as_ref()
                    .is_some_and(|(_, p)| p.started.elapsed() > std::time::Duration::from_secs(10));
                if timed_out {
                    let (_, pending) = self
                        .lsp.state
                        .pending_lsp_responses
                        .implementation
                        .take()
                        .unwrap();
                    crate::lsp_debug!(
                        "LSP-IMPLEMENTATION",
                        "Implementation request timed out, aborting task"
                    );
                    pending.task.abort();
                    self.set_lsp_status("Implementation request timed out".to_string());
                }
                false
            }
            Err(TryRecvError::Closed) => {
                let _pending = self.lsp.state.pending_lsp_responses.implementation.take();
                crate::lsp_debug!(
                    "LSP-IMPLEMENTATION",
                    "Implementation request cancelled (sender dropped)"
                );
                self.set_lsp_status("Implementation request cancelled".to_string());
                false
            }
        }
    }

    /// Poll the type_definition response slot.
    fn poll_type_definition_slot(&mut self) -> bool {
        use tokio::sync::oneshot::error::TryRecvError;

        let Some(ref mut req) = self.lsp.state.pending_lsp_responses.type_definition else {
            return false;
        };

        match req.receiver.try_recv() {
            Ok(result) => {
                let pending = self
                    .lsp.state
                    .pending_lsp_responses
                    .type_definition
                    .take()
                    .unwrap();
                self.handle_location_result(result, pending, "Type", "LSP-TYPE", false)
            }
            Err(TryRecvError::Empty) => {
                let timed_out = self
                    .lsp.state
                    .pending_lsp_responses
                    .type_definition
                    .as_ref()
                    .is_some_and(|p| p.started.elapsed() > std::time::Duration::from_secs(10));
                if timed_out {
                    let pending = self
                        .lsp.state
                        .pending_lsp_responses
                        .type_definition
                        .take()
                        .unwrap();
                    crate::lsp_debug!(
                        "LSP-TYPE",
                        "Type definition request timed out, aborting task"
                    );
                    pending.task.abort();
                    self.set_lsp_status("Type request timed out".to_string());
                }
                false
            }
            Err(TryRecvError::Closed) => {
                let _pending = self.lsp.state.pending_lsp_responses.type_definition.take();
                crate::lsp_debug!(
                    "LSP-TYPE",
                    "Type definition request cancelled (sender dropped)"
                );
                self.set_lsp_status("Type request cancelled".to_string());
                false
            }
        }
    }

    /// Poll completion responses (non-blocking)
    /// Returns true if a response was processed and UI should redraw
    pub fn poll_pending_completion_response(&mut self) -> bool {
        use tokio::sync::oneshot::error::TryRecvError;

        let Some(mut pending) = self.lsp.state.pending_completion.take() else {
            return false;
        };

        match pending.request.receiver.try_recv() {
            Ok(Ok(result)) => {
                if pending.seq != self.lsp.state.completion_request_seq {
                    return false; // Stale response
                }

                if self.mode() != crate::mode::Mode::Insert {
                    self.hide_completion_menu();
                    return false;
                }

                if let (Some(synced), Some(flushed_version)) =
                    (result.synced_content, result.synced_lsp_version)
                {
                    self.mark_document_flushed(&result.file_path, synced, flushed_version);
                }

                let (trigger_col, trigger_prefix) = self.completion_trigger_context();
                self.completion_menu_mut()
                    .show(result.items.clone(), trigger_col, trigger_prefix);
                self.lsp.state.available_completions = result.items;
                self.mark_dirty();
                true
            }
            Ok(Err(e)) => {
                if pending.seq == self.lsp.state.completion_request_seq {
                    self.hide_completion_menu();
                    self.set_lsp_status(format!("Completion failed: {}", e));
                    self.mark_dirty();
                    true
                } else {
                    false
                }
            }
            Err(TryRecvError::Empty) => {
                if pending.request.started.elapsed() > std::time::Duration::from_secs(3) {
                    pending.request.task.abort();
                    if pending.seq == self.lsp.state.completion_request_seq {
                        self.hide_completion_menu();
                        self.set_lsp_status("Completion request timed out".to_string());
                        self.mark_dirty();
                        return true;
                    }
                    return false;
                }

                self.lsp.state.pending_completion = Some(pending);
                false
            }
            Err(TryRecvError::Closed) => {
                if pending.seq == self.lsp.state.completion_request_seq {
                    self.hide_completion_menu();
                    self.set_lsp_status("Completion request cancelled".to_string());
                    self.mark_dirty();
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Poll inlay hint responses (non-blocking)
    /// Returns true if a response was processed and UI should redraw
    pub fn poll_pending_inlay_hint_response(&mut self) -> bool {
        use tokio::sync::oneshot::error::TryRecvError;

        let Some(mut pending) = self.lsp.state.pending_inlay_hints.take() else {
            return false;
        };

        match pending.request.receiver.try_recv() {
            Ok(Ok(result)) => {
                if pending.seq != self.lsp.state.inlay_hint_request_seq {
                    return false;
                }

                if result.buffer_version != self.buffer().version() {
                    return false;
                }

                let matches_current_viewport = self
                    .buffer()
                    .file_path()
                    .is_some_and(|path| path == result.request_key.file_path)
                    && self.scroll_offset() == result.request_key.start_line
                    && self.scroll_offset() + self.viewport_height() + 10
                        == result.request_key.end_line;
                if !matches_current_viewport {
                    return false;
                }

                if result.request_key.lsp_version < self.lsp.state.current_file_lsp_sent_version {
                    return false;
                }

                if let (Some(synced), Some(flushed_version)) =
                    (result.synced_content, result.synced_lsp_version)
                {
                    self.mark_document_flushed(
                        &result.request_key.file_path,
                        synced,
                        flushed_version,
                    );
                }

                self.lsp.state.current_file_lsp_version = result.request_key.lsp_version;
                self.lsp.state.current_file_lsp_sent_version = result.request_key.lsp_version;
                self.lsp.state.inlay_hints = result.hints;
                self.lsp.state.applied_inlay_hint_request = Some(result.request_key);
                self.mark_dirty();
                true
            }
            Ok(Err(_)) => false,
            Err(TryRecvError::Empty) => {
                if pending.request.started.elapsed() > std::time::Duration::from_secs(5) {
                    pending.request.task.abort();
                    return false;
                }

                self.lsp.state.pending_inlay_hints = Some(pending);
                false
            }
            Err(TryRecvError::Closed) => false,
        }
    }

    /// Shared handler for a completed location-based LSP result.
    /// Called after try_recv() returned Ok(result) and the slot has been taken.
    fn handle_location_result(
        &mut self,
        result: anyhow::Result<Option<Location>>,
        _pending: crate::editor::lsp_state::PendingLspRequest<Option<Location>>,
        label: &str,
        log_tag: &str,
        new_tab: bool,
    ) -> bool {
        match result {
            Ok(Some(location)) => {
                crate::lsp_debug!(log_tag, "Received {} response", label.to_lowercase());

                let Some(path) = crate::lsp::uri_to_file_path(&location.uri) else {
                    self.set_lsp_status("Invalid file path in LSP response".to_string());
                    return false;
                };

                let target_line = location.range.start.line as usize;
                let target_character = location.range.start.character;

                self.push_tag();

                if new_tab {
                    let tab_title = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "[No Name]".to_string());
                    self.new_tab(Some(tab_title));
                    match crate::buffer::Buffer::load_file(&path) {
                        Ok(buffer) => {
                            self.buffers[self.current_buffer_index] = buffer;
                            if let Some(path) = self.buffer().file_path() {
                                self.registers.set_current_file(path.to_string());
                            }
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
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(target_line, target_col);
                self.buffer_mut().validate_cursor_position();
                self.center_cursor_in_viewport();
                let actual_col = self.buffer().cursor().col();

                let suffix = if new_tab { " (new tab)" } else { "" };
                self.set_lsp_status(format!(
                    "{}{}: {}:{}:{}",
                    label,
                    suffix,
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    target_line + 1,
                    actual_col + 1
                ));
                self.mark_dirty();
                true
            }
            Ok(None) => {
                crate::lsp_debug!(log_tag, "No {} found", label.to_lowercase());
                self.set_lsp_status(format!("No {} found", label.to_lowercase()));
                false
            }
            Err(e) => {
                crate::lsp_debug!(log_tag, "{} request failed: {:?}", label, e);
                self.set_lsp_status(format!("{} failed: {}", label, e));
                false
            }
        }
    }

    /// Register a new LSP server
    pub fn register_lsp_server(&mut self, language_id: String, server_name: String) {
        self.lsp.state.lsp_status = format!("LSP: {} ready", server_name);
        self.lsp.state
            .active_lsp_servers
            .insert(language_id, server_name);
    }

    /// Unregister an LSP server
    pub fn unregister_lsp_server(&mut self, language_id: &str) {
        self.lsp.state.active_lsp_servers.remove(language_id);
        if self.lsp.state.active_lsp_servers.is_empty() {
            self.lsp.state.lsp_status.clear();
        }
    }

    /// Clear all LSP state (hover, code actions, completions, pending action, pending responses)
    pub(crate) fn clear_lsp_state(&mut self) {
        self.lsp.state.hover_info = None;
        self.lsp.state.hover_scroll = 0;
        self.lsp.state.hover_h_scroll = 0;
        self.lsp.state.available_code_actions.clear();
        self.lsp.state.available_completions.clear();
        self.lsp.state.inlay_hints.clear();
        self.lsp.state.last_inlay_hint_request = None;
        self.lsp.state.last_inlay_hint_request_at = None;
        self.lsp.state.applied_inlay_hint_request = None;
        self.lsp.state.pending_lsp_action = None;
        // Abort all pending LSP responses
        self.lsp.state.pending_lsp_responses.abort_all();
        self.lsp.state.hover_cache = None;
        // Reset LSP version tracking (new file has its own version space)
        self.lsp.state.current_file_lsp_version = 0;
        self.lsp.state.current_file_lsp_sent_version = 0;
        self.lsp.state.diagnostics_valid_for = usize::MAX;
        self.lsp.state.diagnostics_file_path = None;
        // OV-00157: Abort pending completion request on buffer switch
        if let Some(pending) = self.lsp.state.pending_completion.take() {
            pending.request.task.abort();
        }
        if let Some(pending) = self.lsp.state.pending_inlay_hints.take() {
            pending.request.task.abort();
        }
    }

    /// Get active LSP servers map
    pub fn active_lsp_servers(&self) -> &HashMap<String, String> {
        &self.lsp.state.active_lsp_servers
    }

    /// Get LSP progress message (e.g., "indexing...")
    pub fn lsp_progress_message(&self) -> Option<String> {
        if let Some(lsp_manager) = &self.lsp.state.lsp_manager {
            lsp_manager.get_progress_message()
        } else {
            None
        }
    }

    /// Get LSP info for status line
    pub fn get_lsp_info(&self) -> String {
        let mut info = String::new();

        // LSP Manager status
        if self.lsp.state.lsp_manager.is_some() {
            info.push_str("LSP: enabled\n");
        } else {
            info.push_str("LSP: disabled\n");
        }

        // Active servers
        if self.lsp.state.active_lsp_servers.is_empty() {
            info.push_str("Servers: none\n");
        } else {
            info.push_str("Servers:\n");
            for (lang_id, server_name) in &self.lsp.state.active_lsp_servers {
                info.push_str(&format!("  - {} ({})\n", server_name, lang_id));
            }
        }

        // Progress messages
        if let Some(progress) = self.lsp_progress_message() {
            info.push_str(&format!("Progress: {}\n", progress));
        }

        // Diagnostic counts
        let (errors, warnings, infos, hints) = self.lsp.state.diagnostic_count;
        info.push_str(&format!(
            "Diagnostics: E:{} W:{} I:{} H:{}\n",
            errors, warnings, infos, hints
        ));

        // Current status
        if !self.lsp.state.lsp_status.is_empty() {
            info.push_str(&format!("\nStatus: {}\n", self.lsp.state.lsp_status));
        }

        info
    }

    // -------------------------------------------------------------------------
    // LSP Action Requests (set pending_lsp_action flag)
    // -------------------------------------------------------------------------

    /// Queue an LSP action and reset the retry count
    fn queue_lsp_action(&mut self, action: LspAction) {
        self.lsp.state.pending_lsp_action = Some(action);
        self.lsp.state.lsp_action_retry_count = 0;
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
        Some(self.lsp.state.document_sync.entry(file_path).or_default())
    }

    fn reconcile_document_sync_with_manager(
        &mut self,
        file_path: &str,
        current_content: Option<&str>,
        manager_version: i32,
        sent_version: i32,
    ) {
        if manager_version <= 0 {
            return;
        }

        self.lsp.state.current_file_lsp_version = manager_version;
        self.lsp.state.current_file_lsp_sent_version = sent_version;

        let state = self
            .lsp.state
            .document_sync
            .entry(file_path.to_string())
            .or_default();

        if sent_version > 0 && !state.did_open_sent {
            state.did_open_sent = true;
        }

        if sent_version > 0 && state.last_flushed_content.is_none() {
            let seeded_content = state
                .last_queued_content
                .clone()
                .or_else(|| current_content.map(|content| content.to_string()));
            if let Some(content) = seeded_content {
                state.mark_change_flushed(content, sent_version, current_content);
            }
        }

        if state
            .target_lsp_version
            .is_some_and(|target_version| sent_version >= target_version)
        {
            let flushed_content = state
                .last_queued_content
                .clone()
                .or_else(|| current_content.map(|content| content.to_string()));
            if let Some(content) = flushed_content {
                state.mark_change_flushed(content, sent_version, current_content);
            }
        }
    }

    pub async fn refresh_current_lsp_sync_versions(&mut self) {
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            self.lsp.state.current_file_lsp_version = 0;
            self.lsp.state.current_file_lsp_sent_version = 0;
            return;
        };

        let Some(file_path) = self.buffer().file_path().map(str::to_string) else {
            self.lsp.state.current_file_lsp_version = 0;
            self.lsp.state.current_file_lsp_sent_version = 0;
            return;
        };

        let Some(uri) = crate::lsp::uri_from_file_path(&file_path) else {
            self.lsp.state.current_file_lsp_version = 0;
            self.lsp.state.current_file_lsp_sent_version = 0;
            return;
        };

        let manager_version = lsp.get_document_version(&uri).await;
        let sent_version = lsp.get_last_sent_version(&uri).await;
        let needs_content = self
            .lsp.state
            .document_sync
            .get(&file_path)
            .is_some_and(|state| {
                (sent_version > 0 && state.last_flushed_content.is_none())
                    || state
                        .target_lsp_version
                        .is_some_and(|target_version| sent_version >= target_version)
            });
        let current_content = needs_content.then(|| self.buffer().rope().to_string());
        self.reconcile_document_sync_with_manager(
            &file_path,
            current_content.as_deref(),
            manager_version,
            sent_version,
        );
    }

    /// Mark buffer as modified (for LSP didChange tracking).
    ///
    /// No longer eagerly clears cached diagnostics — the generation-based
    /// staleness check (`diagnostics_valid_for != buffer.version()`) hides
    /// them automatically once the buffer version advances past the stamp.
    /// This eliminates the 0→N diagnostic count flicker on the status line.
    pub fn mark_buffer_modified(&mut self) {
        if let Some(state) = self.document_sync_state_mut() {
            state.mark_modified();
        }
    }

    pub fn mark_buffer_modified_force_send(&mut self) {
        if let Some(state) = self.document_sync_state_mut() {
            state.mark_modified();
        }
    }

    pub fn request_diagnostics_refresh(&mut self) {
        self.lsp.state.diagnostics_refresh_requested = true;
    }

    /// Invalidate cached diagnostics and request a fresh pull from the LSP server.
    pub fn clear_and_refresh_diagnostics(&mut self) {
        self.lsp.state.diagnostics_valid_for = usize::MAX;
        self.lsp.state.diagnostics_file_path = None;
        self.lsp.state.diagnostics_refresh_requested = true;
    }

    /// Handle LSP/diagnostics state when current buffer path changes (e.g. :w newfile).
    pub fn handle_file_path_transition_after_save(
        &mut self,
        old_path: Option<String>,
        new_path: Option<String>,
    ) {
        if old_path == new_path {
            return;
        }

        if let Some(old) = old_path {
            self.lsp.state.document_sync.remove(&old);
            self.lsp.state.pending_did_close_file = Some(old);
        }
        if let Some(newp) = &new_path {
            self.lsp.state.document_sync.remove(newp);
        }

        self.lsp.state.needs_lsp_init = true;
        self.clear_and_refresh_diagnostics();
    }

    pub fn take_diagnostics_refresh_request(&mut self) -> bool {
        std::mem::take(&mut self.lsp.state.diagnostics_refresh_requested)
    }

    pub fn lsp_document_sync_exists(&self) -> bool {
        let Some(file_path) = self.buffer().file_path() else {
            return false;
        };
        self.lsp.state.document_sync.contains_key(file_path)
    }

    pub fn lsp_document_is_modified(&self) -> Option<bool> {
        let file_path = self.buffer().file_path()?;
        self.lsp.state
            .document_sync
            .get(file_path)
            .map(|s| s.is_modified())
    }

    /// Mark buffer as saved (for LSP didSave tracking)
    pub fn mark_buffer_saved(&mut self) {
        if let Some(state) = self.document_sync_state_mut() {
            state.mark_saved();
        }
    }

    /// Sends buffered text changes to LSP if modified.
    ///
    /// Forwards the latest buffer content to LspManager on every tick where the
    /// buffer is dirty.  Debouncing is handled entirely by LspManager's
    /// `ChangeDebouncer` (single-owner, 150 ms).  The editor side no longer
    /// adds its own 150 ms gate — that was causing a redundant double-debounce
    /// (OV-00165).
    pub async fn send_lsp_changes_if_modified(&mut self) {
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            return;
        };

        let Some(file_path) = self.buffer().file_path().map(|p| p.to_string()) else {
            return;
        };

        let uri = match uri_from_file_path(&file_path) {
            Some(u) => u,
            None => return,
        };

        let state_key = file_path.clone();
        let manager_version = lsp.get_document_version(&uri).await;
        let sent_version = lsp.get_last_sent_version(&uri).await;
        let needs_reconcile = manager_version > 0
            && self
                .lsp.state
                .document_sync
                .get(&state_key)
                .is_some_and(|state| {
                    (sent_version > 0 && state.last_flushed_content.is_none())
                        || state
                            .target_lsp_version
                            .is_some_and(|target_version| sent_version >= target_version)
                });

        let mut content = None;
        if needs_reconcile {
            content = Some(self.buffer().rope().to_string());
            self.reconcile_document_sync_with_manager(
                &state_key,
                content.as_deref(),
                manager_version,
                sent_version,
            );
        } else if manager_version > 0 {
            self.lsp.state.current_file_lsp_version = manager_version;
            self.lsp.state.current_file_lsp_sent_version = sent_version;
        }

        // Check if we need to send — only guard is didOpen + modified
        let should_send = self
            .lsp.state
            .document_sync
            .get(&state_key)
            .is_some_and(|state| state.did_open_sent && state.is_modified());

        if should_send {
            // Snapshot current content once for queue/no-op checks and potential send.
            let content = content.unwrap_or_else(|| self.buffer().rope().to_string());

            {
                let state = self
                    .lsp.state
                    .document_sync
                    .entry(state_key.clone())
                    .or_default();
                if state.target_lsp_version.is_none()
                    && state.flushed_content() == Some(content.as_str())
                {
                    state.buffer_modified = false;
                    state.last_queued_content = None;
                    return;
                }

                if state.queued_content() == Some(content.as_str()) {
                    return;
                }
            }

            // Get language_id from file extension
            let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path)
            {
                Some(id) => id,
                None => return,
            };

            // Get old content for incremental sync
            let old_content = self
                .lsp.state
                .document_sync
                .get(&state_key)
                .and_then(|state| state.last_flushed_content.clone());

            // Send the didChange notification to all servers for this language
            if lsp
                .did_change_broadcast(uri.clone(), language_id, content.clone(), old_content)
                .await
                .is_err()
            {
                return;
            }

            // Track the queued LSP document version (bumped immediately in did_change).
            let queued_version = lsp.get_document_version(&uri).await;
            self.lsp.state.current_file_lsp_version = queued_version;
            self.lsp.state.current_file_lsp_sent_version = lsp.get_last_sent_version(&uri).await;

            // Record the newest queued snapshot; manager reconciliation will only
            // promote it to flushed once last_sent catches up.
            let state = self.lsp.state.document_sync.entry(state_key).or_default();
            state.mark_change_queued(content, queued_version);
        }
    }

    /// Sends didSave notification to LSP if needed
    pub async fn send_lsp_save_if_needed(&mut self) {
        let Some(file_path) = self.buffer().file_path().map(|p| p.to_string()) else {
            return;
        };

        let uri = match uri_from_file_path(&file_path) {
            Some(u) => u,
            None => return,
        };

        let state_key = file_path.to_string();
        let mut should_send = false;

        // Check if we should send save notification
        if let Some(state) = self.lsp.state.document_sync.get(&state_key) {
            if state.should_send_save() {
                should_send = true;
            }
        }

        if should_send {
            // Ensure didOpen/didChange state for this URI before sending didSave.
            self.ensure_lsp_document_synced().await;

            // Get buffer content BEFORE we update the state
            let content = self.buffer().rope().to_string();

            // Get language_id from file extension
            let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path)
            {
                Some(id) => id,
                None => return,
            };

            let Some(ref lsp) = self.lsp.state.lsp_manager else {
                return;
            };

            // Send the didSave notification to all servers for this language
            match lsp
                .did_save_broadcast(uri, language_id, Some(content))
                .await
            {
                Ok(()) => {
                    // Mark as sent AFTER successful send
                    let state = self.lsp.state.document_sync.entry(state_key).or_default();
                    state.mark_save_sent();
                }
                Err(e) => {
                    crate::lsp_warn!("LSP", "didSave failed for {}: {}", file_path, e);
                }
            }
        }
    }

    /// Ensures the LSP server has the latest document content before making a request
    ///
    /// CRITICAL FIX: When we make a hover/goto request immediately after typing,
    /// the debounced didChange (150ms) might not have been sent yet. This causes
    /// LSP to return stale results. We flush pending changes here to ensure LSP
    /// has the latest content.
    async fn ensure_lsp_document_synced(&mut self) -> bool {
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            return false;
        };

        let Some(file_path) = self.buffer().file_path().map(|p| p.to_string()) else {
            return false;
        };

        let uri = match uri_from_file_path(&file_path) {
            Some(u) => u,
            None => return false,
        };

        let state_key = file_path.clone();

        // Get language_id from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path) {
            Some(id) => id,
            None => return false,
        };

        // Get buffer content
        let content = self.buffer().rope().to_string();
        let manager_version = lsp.get_document_version(&uri).await;
        let sent_version = lsp.get_last_sent_version(&uri).await;
        self.reconcile_document_sync_with_manager(
            &state_key,
            Some(&content),
            manager_version,
            sent_version,
        );

        let plan = self.document_sync_request_plan(&state_key, &content);
        match plan.action {
            DocumentSyncRequestAction::Noop => false,
            DocumentSyncRequestAction::DidOpen => {
                match lsp
                    .did_open_broadcast(uri.clone(), language_id, 1, content.clone())
                    .await
                {
                    Ok(_) => {
                        let flushed_version = lsp.get_last_sent_version(&uri).await;
                        self.lsp.state.current_file_lsp_version =
                            lsp.get_document_version(&uri).await;
                        self.lsp.state.current_file_lsp_sent_version = flushed_version;
                        self.mark_document_flushed(&state_key, content, flushed_version);
                    }
                    Err(e) => {
                        crate::lsp_warn!(
                            "LSP",
                            "didOpen failed for {}: {} (will retry)",
                            state_key,
                            e
                        );
                    }
                }
                true
            }
            DocumentSyncRequestAction::FlushQueued => {
                let _ = lsp.flush_pending_changes_broadcast(&uri, language_id).await;
                let flushed_version = lsp.get_last_sent_version(&uri).await;
                self.lsp.state.current_file_lsp_version = lsp.get_document_version(&uri).await;
                self.lsp.state.current_file_lsp_sent_version = flushed_version;
                self.mark_document_flushed(&state_key, content, flushed_version);
                true
            }
            DocumentSyncRequestAction::QueueChangeAndFlush => {
                if lsp
                    .did_change_broadcast(
                        uri.clone(),
                        language_id,
                        content.clone(),
                        plan.old_content,
                    )
                    .await
                    .is_err()
                {
                    return true;
                }

                let queued_version = lsp.get_document_version(&uri).await;
                {
                    let state = self
                        .lsp.state
                        .document_sync
                        .entry(state_key.clone())
                        .or_default();
                    state.mark_change_queued(content.clone(), queued_version);
                }

                let _ = lsp.flush_pending_changes_broadcast(&uri, language_id).await;
                let flushed_version = lsp.get_last_sent_version(&uri).await;
                self.lsp.state.current_file_lsp_version = lsp.get_document_version(&uri).await;
                self.lsp.state.current_file_lsp_sent_version = flushed_version;
                self.mark_document_flushed(&state_key, content, flushed_version);
                true
            }
        }
    }

    /// Sends didClose notification to LSP for the pending file
    pub async fn send_lsp_close_if_needed(&mut self) {
        let Some(file_path) = self.lsp.state.pending_did_close_file.take() else {
            return;
        };

        let Some(ref lsp) = self.lsp.state.lsp_manager else {
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
        let _ = lsp.did_close_broadcast(uri, language_id).await;
        self.lsp.state.document_sync.remove(&file_path_string);
    }

    // -------------------------------------------------------------------------
    // LSP Action Processing (process pending actions from event loop)
    // -------------------------------------------------------------------------

    /// Process pending LSP actions
    /// Called from the event loop to handle LSP requests asynchronously
    pub async fn process_pending_lsp_actions(&mut self) {
        if let Some(action) = self.lsp.state.pending_lsp_action.take() {
            crate::lsp_debug!(
                "LSP-ACTION",
                "process_pending_lsp_actions() - processing action: {:?}",
                action
            );
            let result = match action {
                LspAction::GoToDefinition => self.goto_definition_impl().await,
                LspAction::GoToDefinitionNewTab => self.goto_definition_new_tab_impl().await,
                LspAction::GoToImplementation => self.goto_implementation_impl().await,
                LspAction::GoToImplementationNewTab => {
                    self.goto_implementation_new_tab_impl().await
                }
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
                    if self.lsp.state.lsp_action_retry_count < 1 {
                        self.lsp.state.lsp_action_retry_count += 1;
                        if self.lsp.state.pending_lsp_action.is_none() {
                            self.lsp.state.pending_lsp_action = Some(action);
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
    pub(crate) fn col_to_utf16(&self, line: usize, col: usize) -> u32 {
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
    pub(crate) fn utf16_to_col(&self, line: usize, utf16_col: u32) -> usize {
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
            .lsp.state
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

        // Resolve the server group responsible for this document (primary + companions).
        let server_ids = lsp.servers_for_document(&language_id, std::path::Path::new(&abs_path));
        if server_ids.is_empty() {
            return Err(anyhow!(
                "No LSP server available for {} in {}",
                feature_name,
                abs_path
            ));
        }

        Ok(LspRequestContext {
            lsp,
            uri,
            file_path,
            line,
            character,
            language_id,
            server_ids,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::lsp_state::{
        InlayHintRequestKey, InlayHintTaskResult, PendingInlayHintRequest, PendingLspRequest,
    };
    use crate::lsp::uri_from_file_path;
    use lsp_types::{InlayHint, InlayHintLabel, Location, Position, Range};
    use tokio::sync::oneshot;

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_handle_location_result_new_tab_updates_current_file_register() {
        let test_dir = tempfile::tempdir().expect("tempdir");

        let source = test_dir.path().join("source.rs");
        let target = test_dir.path().join("target.rs");

        std::fs::write(&source, "source\n").unwrap();
        std::fs::write(&target, "target\n").unwrap();

        let source_path = std::fs::canonicalize(&source)
            .unwrap()
            .to_string_lossy()
            .to_string();
        let target_path = std::fs::canonicalize(&target)
            .unwrap()
            .to_string_lossy()
            .to_string();

        let mut editor = Editor::with_content("source\n");
        editor.set_file_path(source_path);

        let uri = uri_from_file_path(&target).unwrap();
        let location = Location::new(uri, Range::new(Position::new(0, 0), Position::new(0, 0)));

        let (_, receiver) = oneshot::channel::<anyhow::Result<Option<Location>>>();
        let pending = PendingLspRequest {
            task: tokio::spawn(async { Ok(None) }),
            receiver,
            started: std::time::Instant::now(),
        };

        let handled = editor.handle_location_result(
            Ok(Some(location)),
            pending,
            "Definition",
            "LSP-DEFINITION",
            true,
        );
        assert!(handled);
        assert_eq!(editor.registers().get(Some('%')), target_path);
    }

    #[test]
    fn document_sync_request_plan_flushes_already_queued_content() {
        let mut editor = Editor::with_content("class Test {}\n");
        let file_path = "/tmp/Test.java".to_string();
        editor.set_file_path(file_path.clone());

        let state = editor
            .lsp.state
            .document_sync
            .entry(file_path.clone())
            .or_default();
        state.did_open_sent = true;
        state.buffer_modified = true;
        state.last_flushed_content = Some("class Test {\n".to_string());
        state.last_queued_content = Some("class Test {}\n".to_string());
        state.target_lsp_version = Some(4);

        let plan = editor.document_sync_request_plan(&file_path, "class Test {}\n");
        assert_eq!(plan.action, DocumentSyncRequestAction::FlushQueued);
        assert!(plan.old_content.is_none());
    }

    #[test]
    fn reconcile_document_sync_with_manager_promotes_flushed_queue() {
        let mut editor = Editor::with_content("class Test {}\n");
        let file_path = "/tmp/Test.java".to_string();
        editor.set_file_path(file_path.clone());

        let state = editor
            .lsp.state
            .document_sync
            .entry(file_path.clone())
            .or_default();
        state.did_open_sent = true;
        state.mark_change_queued("class Test {}\n".to_string(), 4);

        editor.reconcile_document_sync_with_manager(&file_path, Some("class Test {}\n"), 4, 4);

        let state = editor
            .lsp.state
            .document_sync
            .get(&file_path)
            .expect("document sync state");
        assert!(!state.buffer_modified);
        assert!(state.target_lsp_version.is_none());
        assert!(state.last_queued_content.is_none());
        assert_eq!(
            state.last_flushed_content.as_deref(),
            Some("class Test {}\n")
        );
    }

    #[test]
    fn reconcile_document_sync_with_manager_keeps_dirty_flag_for_newer_buffer_content() {
        let mut editor = Editor::with_content("class Test { int value; }\n");
        let file_path = "/tmp/Test.java".to_string();
        editor.set_file_path(file_path.clone());

        let state = editor
            .lsp.state
            .document_sync
            .entry(file_path.clone())
            .or_default();
        state.did_open_sent = true;
        state.mark_change_queued("class Test {}\n".to_string(), 4);

        editor.reconcile_document_sync_with_manager(
            &file_path,
            Some("class Test { int value; }\n"),
            4,
            4,
        );

        let state = editor
            .lsp.state
            .document_sync
            .get(&file_path)
            .expect("document sync state");
        assert!(state.buffer_modified);
        assert!(state.target_lsp_version.is_none());
        assert_eq!(
            state.last_flushed_content.as_deref(),
            Some("class Test {}\n")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn poll_pending_inlay_hint_response_applies_latest_result() {
        let mut editor = Editor::with_content("class Test {}\n");
        let file_path = "/tmp/Test.java".to_string();
        editor.set_file_path(file_path.clone());
        editor.set_viewport_height(20);
        editor.lsp.state.inlay_hint_request_seq = 1;

        let request_key = InlayHintRequestKey {
            file_path: file_path.clone(),
            start_line: 0,
            end_line: 30,
            lsp_version: 4,
        };
        let hint = InlayHint {
            position: Position::new(0, 5),
            label: InlayHintLabel::String(": Test".to_string()),
            kind: None,
            text_edits: None,
            tooltip: None,
            padding_left: Some(true),
            padding_right: None,
            data: None,
        };

        let (tx, receiver) = oneshot::channel::<anyhow::Result<InlayHintTaskResult>>();
        tx.send(Ok(InlayHintTaskResult {
            request_key: request_key.clone(),
            buffer_version: editor.buffer().version(),
            synced_content: Some("class Test {}\n".to_string()),
            synced_lsp_version: Some(4),
            hints: vec![hint],
        }))
        .unwrap();

        let request_key_for_task = request_key.clone();
        editor.lsp.state.pending_inlay_hints = Some(PendingInlayHintRequest {
            seq: 1,
            request_key: request_key.clone(),
            buffer_version: editor.buffer().version(),
            request: PendingLspRequest {
                task: tokio::spawn(async move {
                    Ok(InlayHintTaskResult {
                        request_key: request_key_for_task,
                        buffer_version: 0,
                        synced_content: None,
                        synced_lsp_version: None,
                        hints: Vec::new(),
                    })
                }),
                receiver,
                started: std::time::Instant::now(),
            },
        });

        assert!(editor.poll_pending_inlay_hint_response());
        assert_eq!(editor.lsp.state.current_file_lsp_version, 4);
        assert_eq!(editor.lsp.state.current_file_lsp_sent_version, 4);
        assert_eq!(editor.lsp.state.inlay_hints.len(), 1);
        assert_eq!(
            editor.lsp.state.applied_inlay_hint_request.as_ref(),
            Some(&request_key)
        );

        let sync_state = editor
            .lsp.state
            .document_sync
            .get(&file_path)
            .expect("document sync state");
        assert!(sync_state.did_open_sent);
        assert_eq!(
            sync_state.last_flushed_content.as_deref(),
            Some("class Test {}\n")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn poll_pending_inlay_hint_response_drops_stale_viewport_result() {
        let mut editor = Editor::with_content("class Test {}\n");
        let file_path = "/tmp/Test.java".to_string();
        editor.set_file_path(file_path.clone());
        editor.set_viewport_height(20);
        editor.lsp.state.inlay_hint_request_seq = 1;

        let request_key = InlayHintRequestKey {
            file_path: file_path.clone(),
            start_line: 0,
            end_line: 30,
            lsp_version: 4,
        };

        let (tx, receiver) = oneshot::channel::<anyhow::Result<InlayHintTaskResult>>();
        tx.send(Ok(InlayHintTaskResult {
            request_key: request_key.clone(),
            buffer_version: editor.buffer().version() + 1,
            synced_content: None,
            synced_lsp_version: None,
            hints: Vec::new(),
        }))
        .unwrap();

        let request_key_for_task = request_key.clone();
        editor.lsp.state.pending_inlay_hints = Some(PendingInlayHintRequest {
            seq: 1,
            request_key,
            buffer_version: editor.buffer().version() + 1,
            request: PendingLspRequest {
                task: tokio::spawn(async move {
                    Ok(InlayHintTaskResult {
                        request_key: request_key_for_task,
                        buffer_version: 0,
                        synced_content: None,
                        synced_lsp_version: None,
                        hints: Vec::new(),
                    })
                }),
                receiver,
                started: std::time::Instant::now(),
            },
        });

        assert!(!editor.poll_pending_inlay_hint_response());
        assert!(editor.lsp.state.inlay_hints.is_empty());
        assert!(editor.lsp.state.applied_inlay_hint_request.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn poll_pending_inlay_hint_response_drops_result_behind_current_sent_version() {
        let mut editor = Editor::with_content("class Test {}\n");
        let file_path = "/tmp/Test.java".to_string();
        editor.set_file_path(file_path.clone());
        editor.set_viewport_height(20);
        editor.lsp.state.inlay_hint_request_seq = 1;
        editor.lsp.state.current_file_lsp_sent_version = 5;

        let request_key = InlayHintRequestKey {
            file_path,
            start_line: 0,
            end_line: 30,
            lsp_version: 4,
        };

        let (tx, receiver) = oneshot::channel::<anyhow::Result<InlayHintTaskResult>>();
        tx.send(Ok(InlayHintTaskResult {
            request_key: request_key.clone(),
            buffer_version: editor.buffer().version(),
            synced_content: None,
            synced_lsp_version: None,
            hints: Vec::new(),
        }))
        .unwrap();

        let request_key_for_task = request_key.clone();
        editor.lsp.state.pending_inlay_hints = Some(PendingInlayHintRequest {
            seq: 1,
            request_key,
            buffer_version: editor.buffer().version(),
            request: PendingLspRequest {
                task: tokio::spawn(async move {
                    Ok(InlayHintTaskResult {
                        request_key: request_key_for_task,
                        buffer_version: 0,
                        synced_content: None,
                        synced_lsp_version: None,
                        hints: Vec::new(),
                    })
                }),
                receiver,
                started: std::time::Instant::now(),
            },
        });

        assert!(!editor.poll_pending_inlay_hint_response());
        assert!(editor.lsp.state.inlay_hints.is_empty());
        assert!(editor.lsp.state.applied_inlay_hint_request.is_none());
    }
}
