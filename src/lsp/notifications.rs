//! LSP notification handling for LspManager
//!
//! This module contains all notification-related methods including:
//! - did_open, did_change, did_save, did_close
//! - Debouncing and flushing mechanisms
//! - Processing incoming notifications from servers

use super::{
    protocol, utils::compute_simple_diff, ChangeDebouncer, JsonRpcMessage, LspManager,
    LspNotification, CHANGE_DEBOUNCE_MS, MAX_DOCUMENT_SIZE,
};
use anyhow::{anyhow, Result};
use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, PublishDiagnosticsParams, TextDocumentContentChangeEvent,
    TextDocumentIdentifier, TextDocumentItem, Uri, VersionedTextDocumentIdentifier,
};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};

impl LspManager {
    pub async fn did_open(
        &self,
        uri: Uri,
        language_id: &str,
        version: i32,
        text: String,
    ) -> Result<()> {
        lsp_debug!(
            "LSP-NOTIFY",
            "textDocument/didOpen | URI: {} | Language: {} | Version: {} | Size: {} bytes",
            uri.as_str(),
            language_id,
            version,
            text.len()
        );

        // Check document size to prevent OOM
        if text.len() > MAX_DOCUMENT_SIZE {
            return Err(anyhow!(
                "Document '{}' too large: {} bytes (max {} bytes / {:.1} MB)",
                uri.as_str(),
                text.len(),
                MAX_DOCUMENT_SIZE,
                MAX_DOCUMENT_SIZE as f64 / (1024.0 * 1024.0)
            ));
        }

        let server = self
            .servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        let params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: language_id.to_string(),
                version,
                text,
            },
        };

        server
            .notify("textDocument/didOpen", serde_json::to_value(params)?)
            .await?;

        lsp_debug!("LSP-NOTIFY", "textDocument/didOpen sent successfully");

        // Initialize version tracking
        let mut versions = self.document_versions.lock().await;
        versions.insert(uri, version);

        Ok(())
    }

    /// Internal method to send textDocument/didChange notification immediately
    /// Supports both full and incremental sync
    #[allow(clippy::print_stderr)]
    async fn send_did_change_immediate(
        &self,
        uri: Uri,
        language_id: &str,
        text: String,
        old_text: Option<String>,
    ) -> Result<()> {
        // Get server reference
        let server = self
            .servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports incremental sync and we have old content
        let supports_incremental = server.supports_incremental_sync().await;

        let full_doc_size = text.len();
        let content_changes = if supports_incremental && old_text.is_some() {
            // Try incremental sync
            if let Some(old) = old_text {
                if let Some((range, new_text)) = compute_simple_diff(&old, &text) {
                    // Log bandwidth savings
                    let incremental_size = new_text.len();
                    let reduction_ratio = if full_doc_size > 0 {
                        full_doc_size as f64 / incremental_size.max(1) as f64
                    } else {
                        1.0
                    };
                    if std::env::var("OVIM_LSP_DEBUG").is_ok() {
                        eprintln!(
                            "[LSP-SYNC] Incremental: {} bytes (was {} bytes, {:.1}x reduction) | Range: {}:{}-{}:{} | File: {}",
                            incremental_size,
                            full_doc_size,
                            reduction_ratio,
                            range.start.line,
                            range.start.character,
                            range.end.line,
                            range.end.character,
                            uri.path()
                        );
                    }

                    // Use incremental change
                    vec![TextDocumentContentChangeEvent {
                        range: Some(range),
                        range_length: None, // Optional, we don't compute it
                        text: new_text,
                    }]
                } else {
                    // No changes detected or identical content
                    if std::env::var("OVIM_LSP_DEBUG").is_ok() {
                        eprintln!("[LSP-SYNC] No changes detected (identical content) | File: {}", uri.path());
                    }
                    return Ok(());
                }
            } else {
                // Fallback to full sync
                if std::env::var("OVIM_LSP_DEBUG").is_ok() {
                    eprintln!("[LSP-SYNC] Full sync (no old_text): {} bytes | File: {}", full_doc_size, uri.path());
                }
                vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text,
                }]
            }
        } else {
            // Use full document sync
            let reason = if !supports_incremental {
                "server doesn't support incremental"
            } else {
                "no old_text provided"
            };
            if std::env::var("OVIM_LSP_DEBUG").is_ok() {
                eprintln!("[LSP-SYNC] Full sync ({}): {} bytes | File: {}", reason, full_doc_size, uri.path());
            }
            vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text,
            }]
        };

        // BUG FIX #1: Hold version lock until AFTER sending to prevent race condition.
        // Critical section: increment version and send notification atomically.
        // This prevents concurrent changes from getting versions (1,2) but sending as (2,1).
        // We must hold the lock across both version increment AND the send operation.
        {
            let mut versions = self.document_versions.lock().await;
            let version = versions.entry(uri.clone()).or_insert(0);
            *version += 1;
            let current_version = *version;

            // Build params while holding lock
            let params = DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: uri.clone(),
                    version: current_version,
                },
                content_changes,
            };

            // Send notification while still holding version lock
            // This ensures version ordering matches send ordering
            crate::metrics::LSP_DIDCHANGE_TOTAL.inc();
            server
                .notify("textDocument/didChange", serde_json::to_value(params)?)
                .await?;
        } // Lock released here, after send is queued

        Ok(())
    }

    /// Flushes pending changes for a document (sends immediately)
    /// BUG FIX: Added timeout to prevent indefinite blocking if LSP server hangs
    pub async fn flush_pending_changes(&self, uri: &Uri) -> Result<()> {
        // Remove debouncer and get pending change
        if let Some((_, debouncer_arc)) = self.change_debouncers.remove(uri) {
            let mut debouncer = debouncer_arc.lock().await;
            debouncer.cancel_timer(); // Cancel timer

            // Send the pending change
            let language_id = debouncer.language_id.clone();
            let text = debouncer.pending_text.clone();
            let old_text = debouncer.old_text.clone();
            let uri = debouncer.uri.clone();
            drop(debouncer); // Release lock before async call

            // BUG FIX: Wrap send_did_change_immediate with timeout (5 seconds)
            // If LSP server hangs, we don't want to block indefinitely
            // This is critical for operations like hover/goto_definition that flush before requesting
            match tokio::time::timeout(
                Duration::from_secs(5),
                self.send_did_change_immediate(uri.clone(), &language_id, text, old_text)
            ).await {
                Ok(Ok(())) => {
                    // Success: change sent to LSP server
                }
                Ok(Err(e)) => {
                    // LSP send failed (server might be down)
                    lsp_error!("Manager", "Failed to flush changes for {}: {}", uri.as_str(), e);
                    // Don't propagate error - allow operation to continue with stale data
                }
                Err(_) => {
                    // Timeout: LSP server is hanging
                    lsp_error!("Manager", "Timeout flushing changes for {} (5s)", uri.as_str());
                    // Don't propagate error - allow operation to continue with stale data
                }
            }
        }
        Ok(())
    }

    /// Sends textDocument/didChange notification with debouncing
    /// Coalesces rapid changes to reduce LSP traffic by ~1000x
    pub async fn did_change(
        &self,
        uri: Uri,
        language_id: &str,
        text: String,
        old_text: Option<String>,
    ) -> Result<()> {
        // Get or create debouncer for this document atomically to prevent race conditions
        let debouncer_arc = self
            .change_debouncers
            .entry(uri.clone())
            .or_insert_with(|| {
                Arc::new(Mutex::new(ChangeDebouncer::new(
                    uri.clone(),
                    language_id.to_string(),
                    text.clone(),
                    old_text.clone(),
                )))
            })
            .clone();

        // Update pending change and restart timer
        let mut debouncer = debouncer_arc.lock().await;

        // Cancel existing timer if any
        debouncer.cancel_timer();

        // Update pending text and old text
        debouncer.pending_text = text;
        // Only set old_text if we don't already have it (first change after sync)
        if debouncer.old_text.is_none() {
            debouncer.old_text = old_text;
        }

        // Clone flush channel for timer closure
        let flush_tx = self.flush_tx.clone();
        let uri_clone = uri.clone();

        // Start new timer
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(CHANGE_DEBOUNCE_MS)).await;

            // Timer expired - request flush via channel
            if let Err(e) = flush_tx.send(uri_clone).await {
                lsp_error!("Debounce", "Error sending flush request: {}", e);
            }
        });

        debouncer.timer_handle = Some(handle);

        Ok(())
    }

    /// Sends textDocument/didSave notification
    pub async fn did_save(&self, uri: Uri, language_id: &str, text: Option<String>) -> Result<()> {
        // Flush any pending changes before saving
        self.flush_pending_changes(&uri).await?;

        let server = self
            .servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        let params = DidSaveTextDocumentParams {
            text_document: TextDocumentIdentifier { uri },
            text,
        };

        server
            .notify("textDocument/didSave", serde_json::to_value(params)?)
            .await?;

        Ok(())
    }

    /// Sends textDocument/didClose notification
    pub async fn did_close(&self, uri: Uri, language_id: &str) -> Result<()> {
        // Flush any pending changes before closing
        self.flush_pending_changes(&uri).await?;

        let server = self
            .servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        let params = DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
        };

        server
            .notify("textDocument/didClose", serde_json::to_value(params)?)
            .await?;

        // Clean up internal state
        let mut versions = self.document_versions.lock().await;
        versions.remove(&uri);
        drop(versions);

        // Remove debouncer for this document
        self.change_debouncers.remove(&uri);

        // Note: We keep diagnostics - they should remain visible even after file is closed

        Ok(())
    }

    /// Handles incoming requests from language servers that expect a response
    async fn handle_server_request(&self, language_id: &str, request: JsonRpcMessage) {
        let method = request.method.as_deref().unwrap_or("");
        let request_id = request.id.clone();

        lsp_info!(
            "LSP-SERVER-REQUEST",
            "Received request from server: {} | ID: {:?}",
            method,
            request_id
        );

        match method {
            "workspace/applyEdit" => {
                // Parse the ApplyWorkspaceEditParams
                if let Some(params) = request.params {
                    match serde_json::from_value::<lsp_types::ApplyWorkspaceEditParams>(params) {
                        Ok(apply_params) => {
                            // Queue the workspace edit for the Editor to apply
                            // The Editor has access to buffers, we just queue the edits here
                            let edit = apply_params.edit;

                            lsp_info!(
                                "LSP-WORKSPACE",
                                "Queuing workspace edit with {} document changes",
                                edit.document_changes.as_ref().map(|changes| match changes {
                                    lsp_types::DocumentChanges::Edits(edits) => edits.len(),
                                    lsp_types::DocumentChanges::Operations(ops) => ops.len(),
                                }).unwrap_or_else(|| edit.changes.as_ref().map(|c| c.len()).unwrap_or(0))
                            );

                            // Send to channel for Editor to process
                            match self.workspace_edit_tx.send(edit).await {
                                Ok(_) => {
                                    // Send success response to LSP server
                                    let response = lsp_types::ApplyWorkspaceEditResponse {
                                        applied: true,
                                        failure_reason: None,
                                        failed_change: None,
                                    };

                                    if let Some(id) = request_id {
                                        if let Some(server) = self.servers.get(language_id) {
                                            match serde_json::to_value(response) {
                                                Ok(value) => {
                                                    let response_msg = JsonRpcMessage::response(id, value);
                                                    if let Err(e) = server.send_response(response_msg).await {
                                                        lsp_error!(
                                                            "LSP-SERVER-REQUEST",
                                                            "Failed to send workspace/applyEdit response: {}",
                                                            e
                                                        );
                                                    }
                                                }
                                                Err(e) => {
                                                    lsp_error!(
                                                        "LSP-SERVER-REQUEST",
                                                        "Failed to serialize workspace/applyEdit response: {}",
                                                        e
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    // Channel send failed
                                    lsp_error!(
                                        "LSP-SERVER-REQUEST",
                                        "Failed to queue workspace edit: {}",
                                        e
                                    );

                                    if let Some(id) = request_id {
                                        if let Some(server) = self.servers.get(language_id) {
                                            let error_response = protocol::ResponseError {
                                                code: -32603, // Internal error
                                                message: format!("Failed to queue edit: {}", e),
                                                data: None,
                                            };

                                            let response_msg =
                                                JsonRpcMessage::error_response(id, error_response);

                                            if let Err(e) = server.send_response(response_msg).await {
                                                lsp_error!(
                                                    "LSP-SERVER-REQUEST",
                                                    "Failed to send error response: {}",
                                                    e
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            lsp_error!(
                                "LSP-SERVER-REQUEST",
                                "Failed to parse workspace/applyEdit params: {}",
                                e
                            );

                            // Send error response for parse failure
                            if let Some(id) = request_id {
                                if let Some(server) = self.servers.get(language_id) {
                                    let error_response = protocol::ResponseError {
                                        code: -32700, // Parse error
                                        message: format!("Failed to parse parameters: {}", e),
                                        data: None,
                                    };

                                    let response_msg =
                                        JsonRpcMessage::error_response(id, error_response);

                                    if let Err(e) = server.send_response(response_msg).await {
                                        lsp_error!(
                                            "LSP-SERVER-REQUEST",
                                            "Failed to send error response: {}",
                                            e
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                lsp_warn!(
                    "LSP-SERVER-REQUEST",
                    "Unsupported server request: {}",
                    method
                );

                // Send "method not found" error response
                if let Some(id) = request_id {
                    if let Some(server) = self.servers.get(language_id) {
                        let error_response = protocol::ResponseError {
                            code: -32601, // Method not found
                            message: format!("Method not supported: {}", method),
                            data: None,
                        };

                        let response_msg = JsonRpcMessage::error_response(id, error_response);

                        if let Err(e) = server.send_response(response_msg).await {
                            lsp_error!(
                                "LSP-SERVER-REQUEST",
                                "Failed to send error response: {}",
                                e
                            );
                        }
                    }
                }
            }
        }
    }

    /// Handles incoming notifications and requests from language servers
    /// This should be called in a background task to process notifications
    pub async fn handle_notification(&self, language_id: &str, message: JsonRpcMessage) {
        // Check if this is a request from the server (needs a response)
        if message.is_request() {
            self.handle_server_request(language_id, message).await;
            return;
        }

        // Handle notifications (no response needed)
        if let Some(method) = &message.method {
            match method.as_str() {
                "textDocument/publishDiagnostics" => {
                    if let Some(params) = message.params {
                        // Clone params for error message before moving
                        let params_clone = params.clone();
                        match serde_json::from_value::<PublishDiagnosticsParams>(params) {
                            Ok(diag_params) => {
                                self.set_diagnostics(diag_params.uri, diag_params.diagnostics)
                                    .await;
                            }
                            Err(e) => {
                                // ERROR: Failed to parse publishDiagnostics - this is critical for user feedback
                                lsp_error!(
                                    &format!("LSP:{}", language_id),
                                    "Failed to parse publishDiagnostics notification: {}",
                                    e
                                );
                                // Show params preview for debugging
                                let params_str = format!("{:?}", params_clone);
                                let preview = if params_str.len() > 500 {
                                    format!("{}...", &params_str[..500])
                                } else {
                                    params_str
                                };
                                lsp_error!(
                                    &format!("LSP:{}", language_id),
                                    "Malformed diagnostics params: {}",
                                    preview
                                );
                            }
                        }
                    }
                }
                "window/showMessage" => {
                    // Only show messages if OVIM_LSP_DEBUG is set to avoid cluttering the terminal
                    if std::env::var("OVIM_LSP_DEBUG").is_ok() {
                        if let Some(params) = message.params {
                            if let Ok(msg_params) =
                                serde_json::from_value::<lsp_types::ShowMessageParams>(params)
                            {
                                // Format message with severity prefix
                                let prefix = match msg_params.typ {
                                    lsp_types::MessageType::ERROR => "LSP Error",
                                    lsp_types::MessageType::WARNING => "LSP Warning",
                                    lsp_types::MessageType::INFO => "LSP Info",
                                    lsp_types::MessageType::LOG => "LSP Log",
                                    _ => "LSP",
                                };
                                let type_str = match msg_params.typ {
                                    lsp_types::MessageType::ERROR => "ERROR",
                                    lsp_types::MessageType::WARNING => "WARN",
                                    lsp_types::MessageType::INFO => "INFO",
                                    lsp_types::MessageType::LOG => "LOG",
                                    _ => "UNKNOWN",
                                };
                                let log_level = match msg_params.typ {
                                    lsp_types::MessageType::ERROR => {
                                        crate::lsp::logger::LogLevel::Error
                                    }
                                    lsp_types::MessageType::WARNING => {
                                        crate::lsp::logger::LogLevel::Warning
                                    }
                                    lsp_types::MessageType::INFO => {
                                        crate::lsp::logger::LogLevel::Info
                                    }
                                    _ => crate::lsp::logger::LogLevel::Info,
                                };
                                crate::lsp::logger::log_message(
                                    log_level,
                                    &format!("{}:{}", language_id, prefix),
                                    &format!("{}: {}", type_str, msg_params.message),
                                );
                            }
                        }
                    }
                }
                "window/logMessage" => {
                    if let Some(params) = message.params {
                        if let Ok(log_params) =
                            serde_json::from_value::<lsp_types::LogMessageParams>(params)
                        {
                            // Only log if OVIM_LSP_DEBUG is set
                            let log_level = match log_params.typ {
                                lsp_types::MessageType::ERROR => {
                                    crate::lsp::logger::LogLevel::Error
                                }
                                lsp_types::MessageType::WARNING => {
                                    crate::lsp::logger::LogLevel::Warning
                                }
                                lsp_types::MessageType::INFO => crate::lsp::logger::LogLevel::Info,
                                _ => crate::lsp::logger::LogLevel::Debug,
                            };
                            let prefix = match log_params.typ {
                                lsp_types::MessageType::ERROR => "ERROR",
                                lsp_types::MessageType::WARNING => "WARN",
                                lsp_types::MessageType::INFO => "INFO",
                                lsp_types::MessageType::LOG => "LOG",
                                _ => "UNKNOWN",
                            };
                            crate::lsp::logger::log_message(
                                log_level,
                                &format!("LSP:{}:{}", language_id, prefix),
                                &log_params.message,
                            );
                        }
                    }
                }
                "$/progress" => {
                    // Progress notifications from LSP server (e.g., jdtls indexing)
                    // These provide real-time feedback about long-running operations
                    if let Some(params) = &message.params {
                        // Try to parse as ProgressParams
                        if let Ok(progress) =
                            serde_json::from_value::<lsp_types::ProgressParams>(params.clone())
                        {
                            // Extract meaningful message from progress
                            let message_opt = match &progress.value {
                                lsp_types::ProgressParamsValue::WorkDone(work_done) => {
                                    match work_done {
                                        lsp_types::WorkDoneProgress::Begin(begin) => {
                                            Some(format!("{}: {}", language_id, begin.title,))
                                        }
                                        lsp_types::WorkDoneProgress::Report(report) => {
                                            if let Some(msg) = &report.message {
                                                Some(format!("{}: {}", language_id, msg))
                                            } else {
                                                report.percentage.map(|percentage| {
                                                    format!("{}: {}%", language_id, percentage)
                                                })
                                            }
                                        }
                                        lsp_types::WorkDoneProgress::End(end) => {
                                            if let Some(msg) = &end.message {
                                                Some(format!("{}: {}", language_id, msg))
                                            } else {
                                                Some(format!("{}: Complete", language_id))
                                            }
                                        }
                                    }
                                }
                            };

                            // Store and log progress messages for UI display
                            if let Some(message) = message_opt {
                                lsp_info!("Progress", "{}", message);
                                // Store latest progress message (will be cleared on End)
                                let mut current_progress = self.current_progress.lock().await;
                                match &progress.value {
                                    lsp_types::ProgressParamsValue::WorkDone(
                                        lsp_types::WorkDoneProgress::End(_),
                                    ) => {
                                        current_progress.remove(&language_id.to_string());
                                    }
                                    _ => {
                                        current_progress.insert(language_id.to_string(), message);
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {
                    // Silently ignore unknown notifications
                    lsp_debug!(
                        &format!("LSP:{}", language_id),
                        "Unknown notification: {}",
                        method
                    );
                }
            }
        }
    }

    /// Processes pending notifications from language servers
    /// Should be called regularly from the main event loop
    pub async fn process_notifications(&self) -> usize {
        let mut rx = self.notification_rx.lock().await;
        let mut count = 0;

        // Process all pending notifications (non-blocking)
        while let Ok(notification) = rx.try_recv() {
            self.handle_notification(&notification.language_id, notification.message)
                .await;
            count += 1;
        }

        count
    }

    /// Processes pending flush requests from debounce timers
    /// Should be called regularly from the main event loop
    /// Returns the number of flush requests processed
    pub async fn process_flush_requests(&self) -> usize {
        let mut rx_opt = self.flush_rx.lock().await;
        let mut count = 0;
        if let Some(rx) = rx_opt.as_mut() {
            // Process all pending flush requests (non-blocking)
            while let Ok(uri) = rx.try_recv() {
                if let Err(e) = self.flush_pending_changes(&uri).await {
                    lsp_error!("Debounce", "Error flushing changes for {}: {}", uri.as_str(), e);
                }
                count += 1;
            }
        }
        count
    }

    /// Polls for pending workspace edits that need to be applied by the Editor
    /// Returns a Vec of workspace edits that should be applied (in order)
    /// This is called from the main event loop which has access to the Editor
    pub async fn poll_pending_workspace_edits(&self) -> Vec<lsp_types::WorkspaceEdit> {
        let mut rx = self.workspace_edit_rx.lock().await;
        let mut edits = Vec::new();

        // Drain all pending workspace edits (non-blocking)
        while let Ok(edit) = rx.try_recv() {
            edits.push(edit);
        }

        edits
    }

    /// Starts a background task to listen for notifications and requests from a language server
    pub async fn start_notification_listener(&self, language_id: String) {
        let server = self
            .servers
            .get(&language_id)
            .map(|entry| entry.value().clone());

        if let Some(server) = server {
            let tx = self.notification_tx.clone();
            let lang_id = language_id.clone();
            let dropped_counter = self.dropped_notifications.clone();

            tokio::spawn(async move {
                while let Some(msg) = server.receive().await {
                    // Handle both notifications (no id) and requests from server (has id)
                    if msg.is_notification() || msg.is_request() {
                        // Send to manager for processing
                        let notification = LspNotification {
                            language_id: lang_id.clone(),
                            message: msg,
                        };

                        // BUG FIX: Use try_send instead of send to avoid blocking
                        // If channel is full, drop the notification and increment counter
                        // This prevents deadlocks when the receiver is slow
                        match tx.try_send(notification) {
                            Ok(()) => {
                                // Successfully sent
                            }
                            Err(mpsc::error::TrySendError::Full(_)) => {
                                // Channel full - drop notification and track it
                                let count = dropped_counter.fetch_add(1, Ordering::Relaxed);
                                if count.is_multiple_of(100) {
                                    // Log every 100 dropped notifications to avoid spam
                                    lsp_error!(
                                        "Listener",
                                        "Notification channel full, dropped {} notifications so far",
                                        count + 1
                                    );
                                }
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => {
                                // Manager dropped, stop listening
                                lsp_error!("Listener", "Notification channel closed, stopping listener");
                                break;
                            }
                        }
                    }
                }
            });
        }
    }
}
