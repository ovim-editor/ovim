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
mod notifications;
mod protocol;
mod requests;
mod server;
mod supervisor;
mod types;
mod utils;

pub use logger::{get_log_path, init_lsp_logging};

pub use protocol::{JsonRpcMessage, RequestId};
pub use server::{LanguageServer, LanguageServerHealth};
pub use supervisor::{RestartPolicy, TaskSupervisor};
pub use types::{uri_from_file_path, uri_to_file_path, LspPosition, LspRange};
pub use utils::compute_simple_diff;

use anyhow::{anyhow, Result};
use dashmap::DashMap;
use lsp_types::{Diagnostic, Uri};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

/// Maximum document size in bytes (10MB)
/// Protects against OOM when opening/syncing large files
const MAX_DOCUMENT_SIZE: usize = 10 * 1024 * 1024;

/// Maximum LSP message size in bytes (50MB)
/// Prevents protocol buffer overflow and server OOM
/// (Reserved for future message size validation)
#[allow(dead_code)]
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
pub(crate) struct ChangeDebouncer {
    /// URI of the document being edited
    uri: Uri,

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
    fn new(uri: Uri, language_id: String, text: String, old_text: Option<String>) -> Self {
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
    diagnostics: Mutex<HashMap<Uri, Vec<Diagnostic>>>,

    /// Document versions for change tracking
    document_versions: Mutex<HashMap<Uri, i32>>,

    /// Channel for receiving notifications from language servers (bounded to prevent memory issues)
    notification_tx: mpsc::Sender<LspNotification>,
    notification_rx: Mutex<mpsc::Receiver<LspNotification>>,

    /// Pending changes being debounced per document
    /// Coalesces rapid changes to reduce LSP traffic by ~1000x
    change_debouncers: DashMap<Uri, Arc<Mutex<ChangeDebouncer>>>,

    /// Channel for debounce flush requests (URI to flush)
    flush_tx: mpsc::Sender<Uri>,
    flush_rx: Mutex<Option<mpsc::Receiver<Uri>>>,

    /// Flag indicating diagnostics have changed and cache needs update
    diagnostics_changed: AtomicBool,

    /// Current progress messages from LSP servers (language_id -> message)
    current_progress: Mutex<HashMap<String, String>>,

    /// Channel for workspace edits that need to be applied by the Editor
    /// These come from server-initiated workspace/applyEdit requests
    workspace_edit_tx: mpsc::Sender<lsp_types::WorkspaceEdit>,
    workspace_edit_rx: Mutex<mpsc::Receiver<lsp_types::WorkspaceEdit>>,

    /// BUG FIX: Counter for dropped notifications when channel is full
    /// Prevents blocking when notification receiver is slow
    dropped_notifications: Arc<AtomicU64>,
}

impl LspManager {
    /// Creates a new LSP manager
    pub fn new() -> Self {
        // Use bounded channel to prevent unbounded memory growth from notifications
        let (notification_tx, notification_rx) = mpsc::channel(1000);
        let (flush_tx, flush_rx) = mpsc::channel(100);
        let (workspace_edit_tx, workspace_edit_rx) = mpsc::channel(100);
        Self {
            servers: DashMap::new(),
            diagnostics: Mutex::new(HashMap::new()),
            document_versions: Mutex::new(HashMap::new()),
            notification_tx,
            notification_rx: Mutex::new(notification_rx),
            change_debouncers: DashMap::new(),
            flush_tx,
            flush_rx: Mutex::new(Some(flush_rx)),
            diagnostics_changed: AtomicBool::new(false),
            current_progress: Mutex::new(HashMap::new()),
            workspace_edit_tx,
            workspace_edit_rx: Mutex::new(workspace_edit_rx),
            dropped_notifications: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Checks if diagnostics have changed and resets the flag
    pub fn diagnostics_changed(&self) -> bool {
        self.diagnostics_changed.swap(false, Ordering::SeqCst)
    }

    /// Gets the number of dropped notifications (when channel was full)
    /// BUG FIX: Added to track notification backpressure
    pub fn get_dropped_notification_count(&self) -> u64 {
        self.dropped_notifications.load(Ordering::Relaxed)
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
        lsp_debug!(
            "LspManager",
            "start_server called for language={}",
            language
        );
        // Check if already running
        if self.servers.contains_key(language) {
            lsp_debug!("LspManager", "Server already running for {}", language);
            return Ok(()); // Already running
        }

        lsp_debug!("LspManager", "Spawning server: {} {:?}", command, args);
        // Spawn and initialize without holding the lock (this can take 10-60 seconds)
        let mut server = LanguageServer::spawn(language, command, args).await?;
        lsp_debug!("LspManager", "Server spawned successfully");

        let root_uri =
            uri_from_file_path(root_path).ok_or_else(|| anyhow::anyhow!("Invalid root path"))?;
        lsp_debug!("LspManager", "Root URI: {}", root_uri.as_str());

        lsp_debug!("LspManager", "Calling initialize...");
        server.initialize(root_uri).await?;
        lsp_debug!("LspManager", "Initialize completed successfully");

        // Insert into servers map
        // Double-check in case another task started the same server
        if let Some(mut existing) = self.servers.insert(language.to_string(), server) {
            // Another thread won the race - clean up the existing server
            if let Err(e) = existing.shutdown().await {
                lsp_warn!(
                    "LspManager",
                    "Failed to shut down redundant server for {}: {}",
                    language,
                    e
                );
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
    pub async fn get_diagnostics(&self, uri: &Uri) -> Vec<Diagnostic> {
        let diagnostics = self.diagnostics.lock().await;
        diagnostics.get(uri).cloned().unwrap_or_default()
    }

    /// Gets diagnostics for a specific line in a file
    pub async fn get_diagnostics_for_line(&self, uri: &Uri, line: u32) -> Vec<Diagnostic> {
        let diagnostics = self.diagnostics.lock().await;
        diagnostics
            .get(uri)
            .map(|diags| {
                diags
                    .iter()
                    .filter(|d| d.range.start.line <= line && d.range.end.line >= line)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Counts diagnostics by severity
    pub async fn count_diagnostics(&self, uri: &Uri) -> (usize, usize, usize, usize) {
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
    pub async fn set_diagnostics(&self, uri: Uri, diagnostics: Vec<Diagnostic>) {
        crate::metrics::LSP_DIAGNOSTICS_TOTAL.inc();
        let mut diags = self.diagnostics.lock().await;
        diags.insert(uri, diagnostics);
        self.diagnostics_changed.store(true, Ordering::SeqCst);
    }

    /// Gets health information for all language servers
    pub async fn health_check(&self) -> Vec<LanguageServerHealth> {
        // Collect servers while holding lock (minimal duration)
        // to avoid holding DashMap lock during async health_check() calls
        let servers: Vec<_> = self.servers.iter().map(|r| r.value().clone()).collect();

        // Lock is released after collection; now iterate without contention
        let mut health_infos = Vec::new();
        for server in servers {
            health_infos.push(server.health_check().await);
        }

        health_infos
    }

    /// Get list of active server language IDs (sync, for command execution)
    pub fn active_server_languages(&self) -> Vec<String> {
        self.servers.iter().map(|r| r.key().clone()).collect()
    }

    /// Get command for a language server (sync)
    pub fn server_command(&self, language: &str) -> Option<String> {
        self.servers.get(language).map(|s| s.command().to_string())
    }

    /// Gets the current version of a document
    pub async fn get_document_version(&self, uri: &Uri) -> i32 {
        let versions = self.document_versions.lock().await;
        versions.get(uri).copied().unwrap_or(0)
    }

    /// Increments the version of a document
    pub async fn increment_document_version(&self, uri: &Uri) -> i32 {
        let mut versions = self.document_versions.lock().await;
        let version = versions.entry(uri.clone()).or_insert(0);
        *version += 1;
        *version
    }

    /// Gets a reference to a language server
    pub async fn get_server(&self, language: &str) -> Option<LanguageServer> {
        self.servers
            .get(language)
            .map(|entry| entry.value().clone())
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
    async fn test_diagnostics_storage() {
        let manager = LspManager::new();
        let uri: Uri = "file:///test.rs".parse().unwrap();

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
        let uri: Uri = "file:///test.rs".parse().unwrap();

        // Initial version is 0
        assert_eq!(manager.get_document_version(&uri).await, 0);

        // Increment version
        let v1 = manager.increment_document_version(&uri).await;
        assert_eq!(v1, 1);

        let v2 = manager.increment_document_version(&uri).await;
        assert_eq!(v2, 2);

        assert_eq!(manager.get_document_version(&uri).await, 2);
    }
}
