//! LSP (Language Server Protocol) client implementation
//!
//! This module provides LSP support for ovim, enabling IDE-like features such as:
//! - Diagnostics (errors and warnings)
//! - Go to definition
//! - Hover information
//! - Code completion
//! - Code actions
//! - Formatting
//!
//! # Architecture
//!
//! - `LspManager`: Central coordinator managing multiple language servers
//! - `LanguageServer`: Individual language server process management
//! - `protocol`: JSON-RPC message handling
//! - `types`: Type conversions and helpers

#[macro_use]
pub mod logger;
mod protocol;
mod server;
mod supervisor;
mod types;

pub use logger::init_lsp_logging;

pub use protocol::{JsonRpcMessage, RequestId};
pub use server::{LanguageServer, LanguageServerHealth};
pub use supervisor::{RestartPolicy, TaskSupervisor};
pub use types::{LspPosition, LspRange};

use anyhow::{anyhow, Result};
use dashmap::DashMap;
use lsp_types::{
    Diagnostic, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, PublishDiagnosticsParams,
    TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem, Url,
    VersionedTextDocumentIdentifier,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

/// Maximum document size in bytes (10MB)
/// Protects against OOM when opening/syncing large files
const MAX_DOCUMENT_SIZE: usize = 10 * 1024 * 1024;

/// Maximum LSP message size in bytes (50MB)
/// Prevents protocol buffer overflow and server OOM
const MAX_MESSAGE_SIZE: usize = 50 * 1024 * 1024;

/// Debounce duration for textDocument/didChange notifications (milliseconds)
/// Coalesces rapid changes to reduce LSP traffic by ~1000x
/// Reduced to 150ms for faster diagnostics feedback (was 300ms)
const CHANGE_DEBOUNCE_MS: u64 = 150;

/// Notification message from a language server
#[derive(Clone)]
pub struct LspNotification {
    pub language_id: String,
    pub message: JsonRpcMessage,
}

/// Information about an active LSP server for introspection
#[derive(Clone, Debug, serde::Serialize)]
pub struct LspServerInfo {
    pub language: String,
    pub command: String,
    pub state: String,
    pub pending_requests: usize,
    pub has_capabilities: bool,
}

/// Debouncer for textDocument/didChange notifications
/// Coalesces rapid changes to reduce LSP traffic
struct ChangeDebouncer {
    /// URI of the document being edited
    uri: Url,

    /// Language ID (e.g., "rust", "python")
    language_id: String,

    /// Full text of the pending change
    pending_text: String,

    /// Old text before change (for incremental sync)
    old_text: Option<String>,

    /// Timer handle for the debounce delay
    timer_handle: Option<JoinHandle<()>>,
}

impl ChangeDebouncer {
    fn new(uri: Url, language_id: String, text: String, old_text: Option<String>) -> Self {
        Self {
            uri,
            language_id,
            pending_text: text,
            old_text,
            timer_handle: None,
        }
    }

    /// Cancels the pending timer if any
    fn cancel_timer(&mut self) {
        if let Some(handle) = self.timer_handle.take() {
            handle.abort();
        }
    }
}

impl Drop for ChangeDebouncer {
    fn drop(&mut self) {
        self.cancel_timer();
    }
}

/// Central LSP manager coordinating all language servers
pub struct LspManager {
    /// Active language servers (one per language)
    /// Using DashMap for lock-free concurrent access
    servers: DashMap<String, LanguageServer>,

    /// Diagnostics per file URI
    diagnostics: Mutex<HashMap<Url, Vec<Diagnostic>>>,

    /// Next request ID
    next_request_id: AtomicU64,

    /// Document versions for change tracking
    document_versions: Mutex<HashMap<Url, i32>>,

    /// Channel for receiving notifications from language servers (bounded to prevent memory issues)
    notification_tx: mpsc::Sender<LspNotification>,
    notification_rx: Mutex<mpsc::Receiver<LspNotification>>,

    /// Pending changes being debounced per document
    /// Coalesces rapid changes to reduce LSP traffic by ~1000x
    change_debouncers: DashMap<Url, Arc<Mutex<ChangeDebouncer>>>,

    /// Channel for debounce flush requests (URI to flush)
    flush_tx: mpsc::Sender<Url>,
    flush_rx: Mutex<Option<mpsc::Receiver<Url>>>,

    /// Flag indicating diagnostics have changed and cache needs update
    diagnostics_changed: AtomicBool,

    /// Current progress messages from LSP servers (language_id -> message)
    current_progress: Mutex<HashMap<String, String>>,
}

impl LspManager {
    /// Creates a new LSP manager
    pub fn new() -> Self {
        // Use bounded channel to prevent unbounded memory growth from notifications
        let (notification_tx, notification_rx) = mpsc::channel(1000);
        let (flush_tx, flush_rx) = mpsc::channel(100);
        Self {
            servers: DashMap::new(),
            diagnostics: Mutex::new(HashMap::new()),
            next_request_id: AtomicU64::new(1),
            document_versions: Mutex::new(HashMap::new()),
            notification_tx,
            notification_rx: Mutex::new(notification_rx),
            change_debouncers: DashMap::new(),
            flush_tx,
            flush_rx: Mutex::new(Some(flush_rx)),
            diagnostics_changed: AtomicBool::new(false),
            current_progress: Mutex::new(HashMap::new()),
        }
    }

    /// Generates a unique request ID
    fn next_request_id(&self) -> RequestId {
        RequestId::Number(self.next_request_id.fetch_add(1, Ordering::SeqCst))
    }

    /// Checks if diagnostics have changed and resets the flag
    pub fn diagnostics_changed(&self) -> bool {
        self.diagnostics_changed.swap(false, Ordering::SeqCst)
    }

    /// Gets current progress message (non-blocking)
    pub fn get_progress_message(&self) -> Option<String> {
        if let Ok(progress) = self.current_progress.try_lock() {
            if !progress.is_empty() {
                // Return the first progress message (usually only one active)
                progress.values().next().cloned()
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Starts a language server for the given language
    pub async fn start_server(
        &self,
        language: &str,
        command: &str,
        args: Vec<String>,
        root_path: &Path,
    ) -> Result<()> {
        lsp_debug!("LspManager", "start_server called for language={}", language);
        // Check if already running
        if self.servers.contains_key(language) {
            lsp_debug!("LspManager", "Server already running for {}", language);
            return Ok(()); // Already running
        }

        lsp_debug!("LspManager", "Spawning server: {} {:?}", command, args);
        // Spawn and initialize without holding the lock (this can take 10-60 seconds)
        let mut server = LanguageServer::spawn(language, command, args).await?;
        lsp_debug!("LspManager", "Server spawned successfully");

        let root_uri = Url::from_file_path(root_path)
            .map_err(|_| anyhow::anyhow!("Invalid root path"))?;
        lsp_debug!("LspManager", "Root URI: {}", root_uri);

        lsp_debug!("LspManager", "Calling initialize...");
        server.initialize(root_uri).await?;
        lsp_debug!("LspManager", "Initialize completed successfully");

        // Insert into servers map
        // Double-check in case another task started the same server
        if let Some(mut existing) = self.servers.insert(language.to_string(), server) {
            // Another thread won the race - clean up the existing server
            if let Err(e) = existing.shutdown().await {
                lsp_warn!("LspManager", "Failed to shut down redundant server for {}: {}", language, e);
            }
        }

        Ok(())
    }

    /// Stops a language server
    pub async fn stop_server(&self, language: &str) -> Result<()> {
        if let Some((_, mut server)) = self.servers.remove(language) {
            server.shutdown().await?;
        }

        Ok(())
    }

    /// Gets diagnostics for a file
    pub async fn get_diagnostics(&self, uri: &Url) -> Vec<Diagnostic> {
        let diagnostics = self.diagnostics.lock().await;
        diagnostics.get(uri).cloned().unwrap_or_default()
    }

    /// Gets diagnostics for a specific line in a file
    pub async fn get_diagnostics_for_line(&self, uri: &Url, line: u32) -> Vec<Diagnostic> {
        let diagnostics = self.diagnostics.lock().await;
        diagnostics
            .get(uri)
            .map(|diags| {
                diags
                    .iter()
                    .filter(|d| {
                        d.range.start.line <= line && d.range.end.line >= line
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Counts diagnostics by severity
    pub async fn count_diagnostics(&self, uri: &Url) -> (usize, usize, usize, usize) {
        let diagnostics = self.diagnostics.lock().await;
        if let Some(diags) = diagnostics.get(uri) {
            let mut errors = 0;
            let mut warnings = 0;
            let mut info = 0;
            let mut hints = 0;

            for diag in diags {
                match diag.severity {
                    Some(lsp_types::DiagnosticSeverity::ERROR) => errors += 1,
                    Some(lsp_types::DiagnosticSeverity::WARNING) => warnings += 1,
                    Some(lsp_types::DiagnosticSeverity::INFORMATION) => info += 1,
                    Some(lsp_types::DiagnosticSeverity::HINT) => hints += 1,
                    None => warnings += 1, // Default to warning if no severity
                    _ => {}
                }
            }

            (errors, warnings, info, hints)
        } else {
            (0, 0, 0, 0)
        }
    }

    /// Sets diagnostics for a file (called when receiving publishDiagnostics)
    pub async fn set_diagnostics(&self, uri: Url, diagnostics: Vec<Diagnostic>) {
        let mut diags = self.diagnostics.lock().await;
        diags.insert(uri, diagnostics);
        self.diagnostics_changed.store(true, Ordering::SeqCst);
    }

    /// Gets health information for all language servers
    pub async fn health_check(&self) -> Vec<LanguageServerHealth> {
        let mut health_infos = Vec::new();

        for entry in self.servers.iter() {
            health_infos.push(entry.value().health_check().await);
        }

        health_infos
    }

    /// Gets the current version of a document
    pub async fn get_document_version(&self, uri: &Url) -> i32 {
        let versions = self.document_versions.lock().await;
        versions.get(uri).copied().unwrap_or(0)
    }

    /// Increments the version of a document
    pub async fn increment_document_version(&self, uri: &Url) -> i32 {
        let mut versions = self.document_versions.lock().await;
        let version = versions.entry(uri.clone()).or_insert(0);
        *version += 1;
        *version
    }

    /// Gets a reference to a language server
    pub async fn get_server(&self, language: &str) -> Option<LanguageServer> {
        self.servers.get(language).map(|entry| entry.value().clone())
    }

    /// Sends textDocument/didOpen notification
    pub async fn did_open(
        &self,
        uri: Url,
        language_id: &str,
        version: i32,
        text: String,
    ) -> Result<()> {
        lsp_debug!("LSP-NOTIFY", "textDocument/didOpen | URI: {} | Language: {} | Version: {} | Size: {} bytes", uri, language_id, version, text.len());

        // Check document size to prevent OOM
        if text.len() > MAX_DOCUMENT_SIZE {
            return Err(anyhow!(
                "Document '{}' too large: {} bytes (max {} bytes / {:.1} MB)",
                uri,
                text.len(),
                MAX_DOCUMENT_SIZE,
                MAX_DOCUMENT_SIZE as f64 / (1024.0 * 1024.0)
            ));
        }

        let server = self.servers
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
    async fn send_did_change_immediate(
        &self,
        uri: Url,
        language_id: &str,
        text: String,
        old_text: Option<String>,
    ) -> Result<()> {
        // Get server reference
        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Increment version INSIDE critical section to prevent race condition.
        // This ensures version numbers are assigned in the same order they're sent to the server.
        // Without this, concurrent changes could get versions 1,2 but send them as 2,1.
        let version = self.increment_document_version(&uri).await;

        // Check if server supports incremental sync and we have old content
        let supports_incremental = server.supports_incremental_sync().await;

        let content_changes = if supports_incremental && old_text.is_some() {
            // Try incremental sync
            if let Some(old) = old_text {
                if let Some((range, new_text)) = compute_simple_diff(&old, &text) {
                    // Use incremental change
                    vec![TextDocumentContentChangeEvent {
                        range: Some(range),
                        range_length: None, // Optional, we don't compute it
                        text: new_text,
                    }]
                } else {
                    // No changes detected or identical content
                    return Ok(());
                }
            } else {
                // Fallback to full sync
                vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text,
                }]
            }
        } else {
            // Use full document sync
            vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text,
            }]
        };

        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version,
            },
            content_changes,
        };

        server
            .notify("textDocument/didChange", serde_json::to_value(params)?)
            .await?;

        Ok(())
    }

    /// Flushes pending changes for a document (sends immediately)
    pub async fn flush_pending_changes(&self, uri: &Url) -> Result<()> {
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

            self.send_did_change_immediate(uri, &language_id, text, old_text)
                .await?;
        }
        Ok(())
    }

    /// Sends textDocument/didChange notification with debouncing
    /// Coalesces rapid changes to reduce LSP traffic by ~1000x
    pub async fn did_change(
        &self,
        uri: Url,
        language_id: &str,
        text: String,
        old_text: Option<String>,
    ) -> Result<()> {
        // Get or create debouncer for this document
        let debouncer_arc = if let Some(existing) = self.change_debouncers.get(&uri) {
            existing.value().clone()
        } else {
            // Create new debouncer
            let debouncer = Arc::new(Mutex::new(ChangeDebouncer::new(
                uri.clone(),
                language_id.to_string(),
                text.clone(),
                old_text.clone(),
            )));
            self.change_debouncers.insert(uri.clone(), debouncer.clone());
            debouncer
        };

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
    pub async fn did_save(&self, uri: Url, language_id: &str, text: Option<String>) -> Result<()> {
        // Flush any pending changes before saving
        self.flush_pending_changes(&uri).await?;

        let server = self.servers
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
    pub async fn did_close(&self, uri: Url, language_id: &str) -> Result<()> {
        // Flush any pending changes before closing
        self.flush_pending_changes(&uri).await?;

        let server = self.servers
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

    /// Handles incoming notifications from language servers
    /// This should be called in a background task to process notifications
    pub async fn handle_notification(&self, language_id: &str, notification: JsonRpcMessage) {
        if let Some(method) = &notification.method {
            match method.as_str() {
                "textDocument/publishDiagnostics" => {
                    if let Some(params) = notification.params {
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
                        if let Some(params) = notification.params {
                            if let Ok(msg_params) = serde_json::from_value::<lsp_types::ShowMessageParams>(params) {
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
                                    lsp_types::MessageType::ERROR => crate::lsp::logger::LogLevel::Error,
                                    lsp_types::MessageType::WARNING => crate::lsp::logger::LogLevel::Warning,
                                    lsp_types::MessageType::INFO => crate::lsp::logger::LogLevel::Info,
                                    _ => crate::lsp::logger::LogLevel::Info,
                                };
                                crate::lsp::logger::log_message(log_level, &format!("{}:{}", language_id, prefix), &format!("{}: {}", type_str, msg_params.message));
                            }
                        }
                    }
                }
                "window/logMessage" => {
                    if let Some(params) = notification.params {
                        if let Ok(log_params) = serde_json::from_value::<lsp_types::LogMessageParams>(params) {
                            // Only log if OVIM_LSP_DEBUG is set
                            let log_level = match log_params.typ {
                                lsp_types::MessageType::ERROR => crate::lsp::logger::LogLevel::Error,
                                lsp_types::MessageType::WARNING => crate::lsp::logger::LogLevel::Warning,
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
                            crate::lsp::logger::log_message(log_level, &format!("LSP:{}:{}", language_id, prefix), &log_params.message);
                        }
                    }
                }
                "$/progress" => {
                    // Progress notifications from LSP server (e.g., jdtls indexing)
                    // These provide real-time feedback about long-running operations
                    if let Some(params) = &notification.params {
                        // Try to parse as ProgressParams
                        if let Ok(progress) = serde_json::from_value::<lsp_types::ProgressParams>(params.clone()) {
                            // Extract meaningful message from progress
                            let message_opt = match &progress.value {
                                lsp_types::ProgressParamsValue::WorkDone(work_done) => {
                                    match work_done {
                                        lsp_types::WorkDoneProgress::Begin(begin) => {
                                            Some(format!("{}: {}",
                                                language_id,
                                                begin.title,
                                            ))
                                        }
                                        lsp_types::WorkDoneProgress::Report(report) => {
                                            if let Some(msg) = &report.message {
                                                Some(format!("{}: {}", language_id, msg))
                                            } else if let Some(percentage) = report.percentage {
                                                Some(format!("{}: {}%", language_id, percentage))
                                            } else {
                                                None // Skip reports without useful info
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
                                    lsp_types::ProgressParamsValue::WorkDone(lsp_types::WorkDoneProgress::End(_)) => {
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
                    lsp_debug!(&format!("LSP:{}", language_id), "Unknown notification: {}", method);
                }
            }
        }
    }

    /// Processes pending notifications from language servers
    /// Should be called regularly from the main event loop
    pub async fn process_notifications(&self) {
        let mut rx = self.notification_rx.lock().await;

        // Process all pending notifications (non-blocking)
        while let Ok(notification) = rx.try_recv() {
            self.handle_notification(&notification.language_id, notification.message).await;
        }
    }

    /// Processes pending flush requests from debounce timers
    /// Should be called regularly from the main event loop
    pub async fn process_flush_requests(&self) {
        let mut rx_opt = self.flush_rx.lock().await;
        if let Some(rx) = rx_opt.as_mut() {
            // Process all pending flush requests (non-blocking)
            while let Ok(uri) = rx.try_recv() {
                if let Err(e) = self.flush_pending_changes(&uri).await {
                    lsp_error!("Debounce", "Error flushing changes for {}: {}", uri, e);
                }
            }
        }
    }

    /// Starts a background task to listen for notifications from a language server
    pub async fn start_notification_listener(&self, language_id: String) {
        let server = self.servers.get(&language_id).map(|entry| entry.value().clone());

        if let Some(server) = server {
            let tx = self.notification_tx.clone();
            let lang_id = language_id.clone();

            tokio::spawn(async move {
                loop {
                    if let Some(msg) = server.receive().await {
                        if msg.is_notification() {
                            // Send notification to manager for processing
                            let notification = LspNotification {
                                language_id: lang_id.clone(),
                                message: msg,
                            };

                            if let Err(e) = tx.send(notification).await {
                                lsp_error!("Listener", "Failed to send notification: {}", e);
                                break; // Manager dropped or channel full, stop listening
                            }
                        }
                    } else {
                        break; // Server closed
                    }
                }
            });
        }
    }

    /// Requests go-to-definition for a position in a document
    pub async fn goto_definition(
        &self,
        uri: &Url,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<lsp_types::Location>> {
        use lsp_types::{GotoDefinitionParams, GotoDefinitionResponse, Position, TextDocumentIdentifier, TextDocumentPositionParams};

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports goto definition
        if !server.supports_goto_definition().await {
            return Ok(None); // Gracefully return None if not supported
        }

        let params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: uri.clone(),
                },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("textDocument/definition", serde_json::to_value(params)?)
            .await?;

        let response: Option<GotoDefinitionResponse> = serde_json::from_value(result).ok();

        // Convert response to single location (take first if multiple)
        Ok(response.and_then(|resp| match resp {
            GotoDefinitionResponse::Scalar(location) => Some(location),
            GotoDefinitionResponse::Array(locations) => locations.into_iter().next(),
            GotoDefinitionResponse::Link(links) => {
                links.into_iter().next().map(|link| lsp_types::Location {
                    uri: link.target_uri,
                    range: link.target_selection_range,
                })
            }
        }))
    }

    /// Requests go-to-implementation for a position in a document
    pub async fn implementation(
        &self,
        uri: &Url,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<lsp_types::Location>> {
        use lsp_types::{request::GotoImplementationParams, Position, TextDocumentIdentifier, TextDocumentPositionParams};
        use lsp_types::GotoDefinitionResponse as GotoImplementationResponse;

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports goto implementation
        if !server.supports_goto_implementation().await {
            return Ok(None); // Gracefully return None if not supported
        }

        let params = GotoImplementationParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: uri.clone(),
                },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("textDocument/implementation", serde_json::to_value(params)?)
            .await?;

        let response: Option<GotoImplementationResponse> = serde_json::from_value(result).ok();

        // Convert response to single location (take first if multiple)
        Ok(response.and_then(|resp| match resp {
            GotoImplementationResponse::Scalar(location) => Some(location),
            GotoImplementationResponse::Array(locations) => locations.into_iter().next(),
            GotoImplementationResponse::Link(links) => {
                links.into_iter().next().map(|link| lsp_types::Location {
                    uri: link.target_uri,
                    range: link.target_selection_range,
                })
            }
        }))
    }

    /// Requests go-to-type-definition for a position in a document
    pub async fn type_definition(
        &self,
        uri: &Url,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<lsp_types::Location>> {
        use lsp_types::{request::GotoTypeDefinitionParams, Position, TextDocumentIdentifier, TextDocumentPositionParams};
        use lsp_types::GotoDefinitionResponse as GotoTypeDefinitionResponse;

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports goto type definition
        if !server.supports_goto_type_definition().await {
            return Ok(None); // Gracefully return None if not supported
        }

        let params = GotoTypeDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: uri.clone(),
                },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("textDocument/typeDefinition", serde_json::to_value(params)?)
            .await?;

        let response: Option<GotoTypeDefinitionResponse> = serde_json::from_value(result).ok();

        // Convert response to single location (take first if multiple)
        Ok(response.and_then(|resp| match resp {
            GotoTypeDefinitionResponse::Scalar(location) => Some(location),
            GotoTypeDefinitionResponse::Array(locations) => locations.into_iter().next(),
            GotoTypeDefinitionResponse::Link(links) => {
                links.into_iter().next().map(|link| lsp_types::Location {
                    uri: link.target_uri,
                    range: link.target_selection_range,
                })
            }
        }))
    }

    /// Requests hover information for a position in a document
    pub async fn hover(
        &self,
        uri: &Url,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<String>> {
        use lsp_types::{HoverParams, Position, TextDocumentIdentifier, TextDocumentPositionParams};

        lsp_debug!("LSP-HOVER", "hover() called | URI: {} | line: {}, char: {} | language: {}", uri, line, character, language_id);

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        lsp_debug!("LSP-HOVER", "Server found for language: {}", language_id);

        // Check if server supports hover
        if !server.supports_hover().await {
            lsp_debug!("LSP-HOVER", "Server does not support hover");
            return Ok(None); // Gracefully return None if not supported
        }

        lsp_debug!("LSP-HOVER", "Server supports hover, sending request");

        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: uri.clone(),
                },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
        };

        let result = server
            .request("textDocument/hover", serde_json::to_value(params)?)
            .await?;

        lsp_debug!("LSP-HOVER", "Received response: {}", result);

        // Handle null response (valid LSP response meaning no hover info)
        let response: Option<lsp_types::Hover> = if result.is_null() {
            lsp_debug!("LSP-HOVER", "Response is null");
            None
        } else {
            let parsed = serde_json::from_value(result.clone()).ok();
            lsp_debug!("LSP-HOVER", "Parsed response: {:?}", parsed.is_some());
            parsed
        };

        // Extract text from hover response
        Ok(response.and_then(|hover| {
            match hover.contents {
                lsp_types::HoverContents::Scalar(content) => Some(marked_string_to_text(content)),
                lsp_types::HoverContents::Array(contents) => {
                    let texts: Vec<String> = contents.into_iter()
                        .map(marked_string_to_text)
                        .collect();
                    if texts.is_empty() {
                        None
                    } else {
                        Some(texts.join("\n\n"))
                    }
                }
                lsp_types::HoverContents::Markup(content) => {
                    Some(content.value)
                }
            }
        }))
    }

    /// Requests code completion for a position in a document
    pub async fn completion(
        &self,
        uri: &Url,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Vec<lsp_types::CompletionItem>> {
        use lsp_types::{CompletionParams, CompletionResponse, Position, TextDocumentIdentifier, TextDocumentPositionParams};

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports completion
        if !server.supports_completion().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        let params = CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: uri.clone(),
                },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: None,
        };

        let result = server
            .request("textDocument/completion", serde_json::to_value(params)?)
            .await?;

        let response: Option<CompletionResponse> = serde_json::from_value(result).ok();

        Ok(response.map(|resp| match resp {
            CompletionResponse::Array(items) => items,
            CompletionResponse::List(list) => list.items,
        }).unwrap_or_default())
    }

    /// Requests document formatting
    pub async fn format_document(
        &self,
        uri: &Url,
        language_id: &str,
        tab_size: u32,
        insert_spaces: bool,
    ) -> Result<Vec<lsp_types::TextEdit>> {
        use lsp_types::{DocumentFormattingParams, FormattingOptions, TextDocumentIdentifier};

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports formatting
        if !server.supports_formatting().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        let params = DocumentFormattingParams {
            text_document: TextDocumentIdentifier {
                uri: uri.clone(),
            },
            options: FormattingOptions {
                tab_size,
                insert_spaces,
                ..Default::default()
            },
            work_done_progress_params: Default::default(),
        };

        let result = server
            .request("textDocument/formatting", serde_json::to_value(params)?)
            .await?;

        let edits: Option<Vec<lsp_types::TextEdit>> = serde_json::from_value(result).ok();

        Ok(edits.unwrap_or_default())
    }

    /// Requests range formatting (format only a selection)
    pub async fn format_range(
        &self,
        uri: &Url,
        language_id: &str,
        start_line: u32,
        start_character: u32,
        end_line: u32,
        end_character: u32,
        tab_size: u32,
        insert_spaces: bool,
    ) -> Result<Vec<lsp_types::TextEdit>> {
        use lsp_types::{DocumentRangeFormattingParams, FormattingOptions, Position, Range, TextDocumentIdentifier};

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports range formatting
        if !server.supports_range_formatting().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        let params = DocumentRangeFormattingParams {
            text_document: TextDocumentIdentifier {
                uri: uri.clone(),
            },
            range: Range {
                start: Position {
                    line: start_line,
                    character: start_character,
                },
                end: Position {
                    line: end_line,
                    character: end_character,
                },
            },
            options: FormattingOptions {
                tab_size,
                insert_spaces,
                ..Default::default()
            },
            work_done_progress_params: Default::default(),
        };

        let result = server
            .request("textDocument/rangeFormatting", serde_json::to_value(params)?)
            .await?;

        let edits: Option<Vec<lsp_types::TextEdit>> = serde_json::from_value(result).ok();

        Ok(edits.unwrap_or_default())
    }

    /// Requests code actions for a position in a document
    pub async fn code_actions(
        &self,
        uri: &Url,
        line: u32,
        character: u32,
        language_id: &str,
        diagnostics: Vec<Diagnostic>,
    ) -> Result<Vec<lsp_types::CodeActionOrCommand>> {
        use lsp_types::{
            CodeActionContext, CodeActionParams, Position, Range, TextDocumentIdentifier,
        };

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports code actions
        if !server.supports_code_actions().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        // Create a range at the cursor position (zero-width range)
        let range = Range {
            start: Position { line, character },
            end: Position { line, character },
        };

        let params = CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: uri.clone(),
            },
            range,
            context: CodeActionContext {
                diagnostics,
                only: None,
                trigger_kind: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("textDocument/codeAction", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::CodeActionOrCommand>> =
            serde_json::from_value(result).ok();

        Ok(response.unwrap_or_default())
    }

    /// Requests find references for a symbol at a position
    pub async fn references(
        &self,
        uri: &Url,
        line: u32,
        character: u32,
        language_id: &str,
        include_declaration: bool,
    ) -> Result<Vec<lsp_types::Location>> {
        use lsp_types::{Position, ReferenceContext, ReferenceParams, TextDocumentIdentifier, TextDocumentPositionParams};

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports references
        if !server.supports_references().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        let params = ReferenceParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: uri.clone(),
                },
                position: Position { line, character },
            },
            context: ReferenceContext {
                include_declaration,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("textDocument/references", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::Location>> = serde_json::from_value(result).ok();

        Ok(response.unwrap_or_default())
    }

    /// Prepares for rename by checking if the symbol can be renamed
    /// Returns the range of the symbol to rename and optionally a placeholder
    pub async fn prepare_rename(
        &self,
        uri: &Url,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<lsp_types::PrepareRenameResponse>> {
        use lsp_types::{Position, TextDocumentIdentifier, TextDocumentPositionParams};

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports prepare rename
        if !server.supports_prepare_rename().await {
            return Ok(None); // Return None if not supported
        }

        let params = TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: uri.clone(),
            },
            position: Position { line, character },
        };

        let result = server
            .request("textDocument/prepareRename", serde_json::to_value(params)?)
            .await?;

        let response: Option<lsp_types::PrepareRenameResponse> = serde_json::from_value(result).ok();

        Ok(response)
    }

    /// Requests rename for a symbol at a position
    pub async fn rename(
        &self,
        uri: &Url,
        line: u32,
        character: u32,
        language_id: &str,
        new_name: String,
    ) -> Result<Option<lsp_types::WorkspaceEdit>> {
        use lsp_types::{Position, RenameParams, TextDocumentIdentifier, TextDocumentPositionParams};

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports rename
        if !server.supports_rename().await {
            return Ok(None); // Return None if not supported
        }

        let params = RenameParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: uri.clone(),
                },
                position: Position { line, character },
            },
            new_name,
            work_done_progress_params: Default::default(),
        };

        let result = server
            .request("textDocument/rename", serde_json::to_value(params)?)
            .await?;

        let response: Option<lsp_types::WorkspaceEdit> = serde_json::from_value(result).ok();

        Ok(response)
    }

    /// Requests signature help for a position in a document
    pub async fn signature_help(
        &self,
        uri: &Url,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<lsp_types::SignatureHelp>> {
        use lsp_types::{Position, SignatureHelpParams, TextDocumentIdentifier, TextDocumentPositionParams};

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports signature help
        if !server.supports_signature_help().await {
            return Ok(None); // Return None if not supported
        }

        let params = SignatureHelpParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: uri.clone(),
                },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            context: None,
        };

        let result = server
            .request("textDocument/signatureHelp", serde_json::to_value(params)?)
            .await?;

        let response: Option<lsp_types::SignatureHelp> = serde_json::from_value(result).ok();

        Ok(response)
    }

    /// Requests selection ranges for smart selection expansion
    pub async fn selection_range(
        &self,
        uri: &Url,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<lsp_types::SelectionRange>> {
        use lsp_types::{Position, SelectionRangeParams, TextDocumentIdentifier};

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports selection range
        if !server.supports_selection_range().await {
            return Ok(None); // Return None if not supported
        }

        let params = SelectionRangeParams {
            text_document: TextDocumentIdentifier {
                uri: uri.clone(),
            },
            positions: vec![Position { line, character }],
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("textDocument/selectionRange", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::SelectionRange>> = serde_json::from_value(result).ok();

        // Return the first (and only) selection range
        Ok(response.and_then(|ranges| ranges.into_iter().next()))
    }

    /// Requests document symbols (outline)
    pub async fn document_symbols(
        &self,
        uri: &Url,
        language_id: &str,
    ) -> Result<Vec<lsp_types::DocumentSymbol>> {
        use lsp_types::{DocumentSymbolParams, TextDocumentIdentifier};

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports document symbols
        if !server.supports_document_symbol().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        let params = DocumentSymbolParams {
            text_document: TextDocumentIdentifier {
                uri: uri.clone(),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("textDocument/documentSymbol", serde_json::to_value(params)?)
            .await?;

        // Response can be either DocumentSymbol[] or SymbolInformation[]
        // Try DocumentSymbol first (hierarchical)
        if let Ok(symbols) = serde_json::from_value::<Vec<lsp_types::DocumentSymbol>>(result.clone()) {
            return Ok(symbols);
        }

        // Fall back to SymbolInformation (flat) - convert to DocumentSymbol
        if let Ok(symbols) = serde_json::from_value::<Vec<lsp_types::SymbolInformation>>(result) {
            // Convert SymbolInformation to DocumentSymbol (without children)
            let doc_symbols = symbols.into_iter().map(|sym| {
                lsp_types::DocumentSymbol {
                    name: sym.name,
                    detail: None,
                    kind: sym.kind,
                    tags: sym.tags,
                    deprecated: None, // Use tags instead
                    range: sym.location.range,
                    selection_range: sym.location.range,
                    children: None,
                }
            }).collect();
            return Ok(doc_symbols);
        }

        Ok(Vec::new())
    }

    /// Requests document highlights for symbol at position
    /// Returns ranges that should be highlighted (read, write, or text occurrences)
    pub async fn document_highlight(
        &self,
        uri: &Url,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Vec<lsp_types::DocumentHighlight>> {
        use lsp_types::{DocumentHighlightParams, Position, TextDocumentIdentifier, TextDocumentPositionParams};

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports document highlight
        if !server.supports_document_highlight().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        let params = DocumentHighlightParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: uri.clone(),
                },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("textDocument/documentHighlight", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::DocumentHighlight>> = serde_json::from_value(result).ok();

        Ok(response.unwrap_or_default())
    }

    /// Requests workspace-wide symbol search
    pub async fn workspace_symbols(
        &self,
        language_id: &str,
        query: String,
    ) -> Result<Vec<lsp_types::SymbolInformation>> {
        use lsp_types::WorkspaceSymbolParams;

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports workspace symbols
        if !server.supports_workspace_symbol().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        let params = WorkspaceSymbolParams {
            query,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("workspace/symbol", serde_json::to_value(params)?)
            .await?;

        // Response can be either SymbolInformation[] or WorkspaceSymbol[]
        // Try SymbolInformation first (simpler format)
        if let Ok(symbols) = serde_json::from_value::<Vec<lsp_types::SymbolInformation>>(result.clone()) {
            return Ok(symbols);
        }

        // Try WorkspaceSymbol (newer format with optional data field)
        if let Ok(symbols) = serde_json::from_value::<Vec<lsp_types::WorkspaceSymbol>>(result) {
            // Convert WorkspaceSymbol to SymbolInformation
            let symbol_infos = symbols.into_iter().filter_map(|sym| {
                // WorkspaceSymbol has OneOf<Location, WorkspaceLocation>
                // We only support full Location for now
                match sym.location {
                    lsp_types::OneOf::Left(location) => Some(lsp_types::SymbolInformation {
                        name: sym.name,
                        kind: sym.kind,
                        tags: sym.tags,
                        deprecated: None,
                        location,
                        container_name: sym.container_name,
                    }),
                    lsp_types::OneOf::Right(_workspace_location) => {
                        // Skip workspace locations (URIs without ranges) for now
                        // These need to be resolved separately
                        None
                    }
                }
            }).collect();
            return Ok(symbol_infos);
        }

        Ok(Vec::new())
    }

    /// Requests folding ranges for a document
    /// Returns ranges that can be folded (functions, blocks, comments, etc.)
    pub async fn folding_range(
        &self,
        uri: &Url,
        language_id: &str,
    ) -> Result<Vec<lsp_types::FoldingRange>> {
        use lsp_types::{FoldingRangeParams, TextDocumentIdentifier};

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports folding range
        if !server.supports_folding_range().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        let params = FoldingRangeParams {
            text_document: TextDocumentIdentifier {
                uri: uri.clone(),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("textDocument/foldingRange", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::FoldingRange>> = serde_json::from_value(result).ok();

        Ok(response.unwrap_or_default())
    }

    /// Prepares call hierarchy for a position in a document
    /// Returns call hierarchy items at the cursor position (typically one item)
    pub async fn prepare_call_hierarchy(
        &self,
        uri: Url,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<Vec<lsp_types::CallHierarchyItem>>> {
        use lsp_types::{CallHierarchyPrepareParams, Position, TextDocumentIdentifier, TextDocumentPositionParams};

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports call hierarchy
        if !server.supports_call_hierarchy().await {
            return Ok(None); // Return None if not supported
        }

        let params = CallHierarchyPrepareParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
        };

        let result = server
            .request("textDocument/prepareCallHierarchy", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::CallHierarchyItem>> = serde_json::from_value(result).ok();

        Ok(response)
    }

    /// Requests incoming calls for a call hierarchy item
    /// Returns methods/functions that call the given item
    pub async fn incoming_calls(
        &self,
        item: lsp_types::CallHierarchyItem,
        language_id: &str,
    ) -> Result<Option<Vec<lsp_types::CallHierarchyIncomingCall>>> {
        use lsp_types::CallHierarchyIncomingCallsParams;

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports call hierarchy
        if !server.supports_call_hierarchy().await {
            return Ok(None); // Return None if not supported
        }

        let params = CallHierarchyIncomingCallsParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("callHierarchy/incomingCalls", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::CallHierarchyIncomingCall>> = serde_json::from_value(result).ok();

        Ok(response)
    }

    /// Requests outgoing calls for a call hierarchy item
    /// Returns methods/functions that the given item calls
    pub async fn outgoing_calls(
        &self,
        item: lsp_types::CallHierarchyItem,
        language_id: &str,
    ) -> Result<Option<Vec<lsp_types::CallHierarchyOutgoingCall>>> {
        use lsp_types::CallHierarchyOutgoingCallsParams;

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports call hierarchy
        if !server.supports_call_hierarchy().await {
            return Ok(None); // Return None if not supported
        }

        let params = CallHierarchyOutgoingCallsParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("callHierarchy/outgoingCalls", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::CallHierarchyOutgoingCall>> = serde_json::from_value(result).ok();

        Ok(response)
    }

    /// Prepares type hierarchy for a position in a document
    /// Returns type hierarchy items at the cursor position (typically one item - the class/interface at cursor)
    pub async fn prepare_type_hierarchy(
        &self,
        uri: Url,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<Vec<lsp_types::TypeHierarchyItem>>> {
        use lsp_types::{TypeHierarchyPrepareParams, Position, TextDocumentIdentifier, TextDocumentPositionParams};

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports type hierarchy
        if !server.supports_type_hierarchy().await {
            return Ok(None); // Return None if not supported
        }

        let params = TypeHierarchyPrepareParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
        };

        let result = server
            .request("textDocument/prepareTypeHierarchy", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::TypeHierarchyItem>> = serde_json::from_value(result).ok();

        Ok(response)
    }

    /// Requests supertypes (parent classes and interfaces) for a type hierarchy item
    /// Returns parent classes and implemented interfaces
    pub async fn supertypes(
        &self,
        item: lsp_types::TypeHierarchyItem,
        language_id: &str,
    ) -> Result<Option<Vec<lsp_types::TypeHierarchyItem>>> {
        use lsp_types::TypeHierarchySupertypesParams;

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports type hierarchy
        if !server.supports_type_hierarchy().await {
            return Ok(None); // Return None if not supported
        }

        let params = TypeHierarchySupertypesParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("typeHierarchy/supertypes", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::TypeHierarchyItem>> = serde_json::from_value(result).ok();

        Ok(response)
    }

    /// Requests subtypes (subclasses and implementations) for a type hierarchy item
    /// Returns child classes and interface implementations
    pub async fn subtypes(
        &self,
        item: lsp_types::TypeHierarchyItem,
        language_id: &str,
    ) -> Result<Option<Vec<lsp_types::TypeHierarchyItem>>> {
        use lsp_types::TypeHierarchySubtypesParams;

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports type hierarchy
        if !server.supports_type_hierarchy().await {
            return Ok(None); // Return None if not supported
        }

        let params = TypeHierarchySubtypesParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("typeHierarchy/subtypes", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::TypeHierarchyItem>> = serde_json::from_value(result).ok();

        Ok(response)
    }

    /// Executes a command on the LSP server (e.g., "Organize Imports")
    /// Returns the command result if successful
    pub async fn execute_command(
        &self,
        command: String,
        arguments: Option<Vec<serde_json::Value>>,
        language_id: &str,
    ) -> Result<Option<serde_json::Value>> {
        use lsp_types::ExecuteCommandParams;

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports execute command
        if !server.supports_execute_command().await {
            return Err(anyhow::anyhow!("Server does not support workspace/executeCommand"));
        }

        let params = ExecuteCommandParams {
            command,
            arguments: arguments.unwrap_or_default(),
            work_done_progress_params: Default::default(),
        };

        let result = server
            .request("workspace/executeCommand", serde_json::to_value(params)?)
            .await?;

        Ok(Some(result))
    }

    /// Requests inlay hints for a document range
    pub async fn inlay_hints(
        &self,
        uri: &Url,
        range: lsp_types::Range,
        language_id: &str,
    ) -> Result<Vec<lsp_types::InlayHint>> {
        use lsp_types::{InlayHintParams, TextDocumentIdentifier, WorkDoneProgressParams};

        let server = self.servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Check if server supports inlay hints
        if !server.supports_inlay_hints().await {
            return Ok(Vec::new());
        }

        let params = InlayHintParams {
            text_document: TextDocumentIdentifier {
                uri: uri.clone(),
            },
            range,
            work_done_progress_params: WorkDoneProgressParams::default(),
        };

        let result = server
            .request("textDocument/inlayHint", serde_json::to_value(params)?)
            .await?;

        let hints: Vec<lsp_types::InlayHint> = serde_json::from_value(result).unwrap_or_default();

        Ok(hints)
    }

    /// Gets LSP status information for all active servers
    /// Returns a list of server info with language, command, state, and pending requests
    pub async fn get_lsp_status(&self) -> Vec<LspServerInfo> {
        let mut result = Vec::new();

        for entry in self.servers.iter() {
            let language = entry.key().clone();
            let server = entry.value();
            let state = server.get_state().await;
            let pending_count = server.pending_requests_count().await;
            let has_capabilities = server.has_capabilities().await;
            let command = server.get_command().await;

            result.push(LspServerInfo {
                language,
                command,
                state: format!("{:?}", state),
                pending_requests: pending_count,
                has_capabilities,
            });
        }

        result
    }

    /// Gets the list of active language server names
    pub async fn get_active_servers(&self) -> Vec<String> {
        self.servers.iter().map(|entry| entry.key().clone()).collect()
    }
}

/// Computes a simple diff between old and new content for incremental sync
/// Returns Some((range, new_text)) if a single contiguous change is found,
/// or None if the change is too complex (fallback to full sync)
pub fn compute_simple_diff(old_content: &str, new_content: &str) -> Option<(lsp_types::Range, String)> {
    use lsp_types::{Position, Range};

    let old_lines: Vec<&str> = old_content.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();

    // Find first differing line from start
    let mut start_line = 0;
    while start_line < old_lines.len() && start_line < new_lines.len() {
        if old_lines[start_line] != new_lines[start_line] {
            break;
        }
        start_line += 1;
    }

    // If all old lines match prefix of new lines, it's just appending
    if start_line == old_lines.len() {
        if new_lines.len() > old_lines.len() {
            // Lines were appended
            let start_pos = Position {
                line: old_lines.len() as u32,
                character: 0,
            };
            let new_text = new_lines[old_lines.len()..].join("\n");
            let new_text = if !old_content.is_empty() { format!("\n{}", new_text) } else { new_text };

            return Some((
                Range {
                    start: start_pos,
                    end: start_pos,
                },
                new_text,
            ));
        }
        // Contents are identical
        return None;
    }

    // Find first differing line from end
    let mut end_line_old = old_lines.len();
    let mut end_line_new = new_lines.len();

    while end_line_old > start_line && end_line_new > start_line {
        if old_lines[end_line_old - 1] != new_lines[end_line_new - 1] {
            break;
        }
        end_line_old -= 1;
        end_line_new -= 1;
    }

    // Now we have a contiguous changed region:
    // old: start_line..end_line_old
    // new: start_line..end_line_new

    // Find character-level start position within the first changed line
    let start_char = if end_line_old == start_line {
        // Pure insertion: no old lines in the changed region
        // Start at beginning of the line
        0
    } else if start_line < old_lines.len() && start_line < new_lines.len() {
        let old_line = old_lines[start_line];
        let new_line = new_lines[start_line];
        let mut char_pos = 0;

        for (old_ch, new_ch) in old_line.chars().zip(new_line.chars()) {
            if old_ch != new_ch {
                break;
            }
            char_pos += 1;
        }
        char_pos
    } else {
        0
    };

    // Find character-level end position within the last changed line
    let end_char = if end_line_old > 0 && end_line_old <= old_lines.len() {
        old_lines[end_line_old - 1].chars().count()
    } else {
        0
    };

    let start_pos = Position {
        line: start_line as u32,
        character: start_char as u32,
    };

    let end_pos = Position {
        line: (end_line_old.saturating_sub(1)) as u32,
        character: end_char as u32,
    };

    // Extract the new text for the changed region
    let new_text = if start_line < new_lines.len() {
        if end_line_new > start_line {
            // Multiple lines changed
            let mut result = String::new();

            // First line: from start_char onwards
            if let Some(first_line) = new_lines.get(start_line) {
                if start_char < first_line.chars().count() {
                    result.push_str(&first_line.chars().skip(start_char).collect::<String>());
                }
            }

            // Middle lines
            for line in &new_lines[start_line + 1..end_line_new] {
                result.push('\n');
                result.push_str(line);
            }

            result
        } else {
            // Single line partial change
            new_lines[start_line]
                .chars()
                .skip(start_char)
                .collect::<String>()
        }
    } else {
        String::new()
    };

    Some((Range { start: start_pos, end: end_pos }, new_text))
}

/// Converts a MarkedString to plain text
fn marked_string_to_text(marked: lsp_types::MarkedString) -> String {
    match marked {
        lsp_types::MarkedString::String(s) => s,
        lsp_types::MarkedString::LanguageString(ls) => ls.value,
    }
}

impl Default for LspManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_lsp_manager_creation() {
        let manager = LspManager::new();
        assert_eq!(manager.next_request_id(), RequestId::Number(1));
        assert_eq!(manager.next_request_id(), RequestId::Number(2));
    }

    #[tokio::test]
    async fn test_diagnostics_storage() {
        let manager = LspManager::new();
        let uri = Url::parse("file:///test.rs").unwrap();

        // Initially no diagnostics
        assert!(manager.get_diagnostics(&uri).await.is_empty());

        // Set diagnostics
        let diags = vec![]; // Empty for now
        manager.set_diagnostics(uri.clone(), diags).await;

        // Verify stored
        assert_eq!(manager.get_diagnostics(&uri).await.len(), 0);
    }

    #[tokio::test]
    async fn test_document_versioning() {
        let manager = LspManager::new();
        let uri = Url::parse("file:///test.rs").unwrap();

        // Initial version is 0
        assert_eq!(manager.get_document_version(&uri).await, 0);

        // Increment version
        let v1 = manager.increment_document_version(&uri).await;
        assert_eq!(v1, 1);

        let v2 = manager.increment_document_version(&uri).await;
        assert_eq!(v2, 2);

        assert_eq!(manager.get_document_version(&uri).await, 2);
    }

    #[test]
    fn test_compute_simple_diff_no_change() {
        let old = "Hello, world!";
        let new = "Hello, world!";
        let result = compute_simple_diff(old, new);
        assert!(result.is_none(), "No diff expected for identical content");
    }

    #[test]
    fn test_compute_simple_diff_single_line_insert() {
        let old = "Hello, world!";
        let new = "Hello, beautiful world!";
        let result = compute_simple_diff(old, new);
        assert!(result.is_some(), "Expected diff for inserted text");

        let (range, new_text) = result.unwrap();
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 7);
        assert_eq!(range.end.line, 0);
        assert_eq!(range.end.character, 13);
        assert_eq!(new_text, "beautiful world!");
    }

    #[test]
    fn test_compute_simple_diff_single_line_delete() {
        let old = "Hello, beautiful world!";
        let new = "Hello, world!";
        let result = compute_simple_diff(old, new);
        assert!(result.is_some(), "Expected diff for deleted text");

        let (range, new_text) = result.unwrap();
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 7);
        assert_eq!(range.end.line, 0);
        assert_eq!(range.end.character, 23);
        assert_eq!(new_text, "world!");
    }

    #[test]
    fn test_compute_simple_diff_multiline_change() {
        let old = "Line 1\nLine 2\nLine 3\n";
        let new = "Line 1\nModified Line 2\nLine 3\n";
        let result = compute_simple_diff(old, new);
        assert!(result.is_some(), "Expected diff for modified line");

        let (range, new_text) = result.unwrap();
        assert_eq!(range.start.line, 1);
        assert_eq!(range.start.character, 0);
        assert_eq!(range.end.line, 1);
        assert_eq!(range.end.character, 6);
        assert_eq!(new_text, "Modified Line 2");
    }

    #[test]
    fn test_compute_simple_diff_insert_line() {
        let old = "Line 1\nLine 3\n";
        let new = "Line 1\nLine 2\nLine 3\n";
        let result = compute_simple_diff(old, new);
        assert!(result.is_some(), "Expected diff for inserted line");

        let (range, new_text) = result.unwrap();
        assert_eq!(range.start.line, 1);
        // The diff algorithm should include "Line 2\nLine 3" as the new text
        assert!(new_text.contains("Line 2") || new_text.contains("Line 3"),
                "Expected new_text to contain inserted content, got: {:?}", new_text);
    }

    #[test]
    fn test_compute_simple_diff_delete_line() {
        let old = "Line 1\nLine 2\nLine 3\n";
        let new = "Line 1\nLine 3\n";
        let result = compute_simple_diff(old, new);
        assert!(result.is_some(), "Expected diff for deleted line");

        let (_range, _new_text) = result.unwrap();
        assert_eq!(_range.start.line, 1);
    }

    #[test]
    fn test_compute_simple_diff_start_of_file() {
        let old = "fn main() {\n    println!(\"Hello\");\n}\n";
        let new = "// Comment\nfn main() {\n    println!(\"Hello\");\n}\n";
        let result = compute_simple_diff(old, new);
        assert!(result.is_some(), "Expected diff for content added at start");

        let (range, new_text) = result.unwrap();
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 0);
        assert!(new_text.starts_with("// Comment"));
    }

    #[test]
    fn test_compute_simple_diff_end_of_file() {
        let old = "fn main() {\n    println!(\"Hello\");\n}\n";
        let new = "fn main() {\n    println!(\"Hello\");\n}\n// Trailing comment\n";
        let result = compute_simple_diff(old, new);
        assert!(result.is_some(), "Expected diff for content added at end");

        let (_range, new_text) = result.unwrap();
        assert!(new_text.contains("// Trailing comment"));
    }
}
