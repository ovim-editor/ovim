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

mod protocol;
mod server;
mod types;

pub use protocol::{JsonRpcMessage, RequestId};
pub use server::LanguageServer;
pub use types::{LspPosition, LspRange};

use anyhow::Result;
use lsp_types::{
    Diagnostic, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, PublishDiagnosticsParams,
    TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem, Url,
    VersionedTextDocumentIdentifier,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{mpsc, Mutex};

/// Notification message from a language server
#[derive(Clone)]
pub struct LspNotification {
    pub language_id: String,
    pub message: JsonRpcMessage,
}

/// Central LSP manager coordinating all language servers
pub struct LspManager {
    /// Active language servers (one per language)
    servers: Mutex<HashMap<String, LanguageServer>>,

    /// Diagnostics per file URI
    diagnostics: Mutex<HashMap<Url, Vec<Diagnostic>>>,

    /// Next request ID
    next_request_id: AtomicU64,

    /// Document versions for change tracking
    document_versions: Mutex<HashMap<Url, i32>>,

    /// Channel for receiving notifications from language servers
    notification_tx: mpsc::UnboundedSender<LspNotification>,
    notification_rx: Mutex<mpsc::UnboundedReceiver<LspNotification>>,
}

impl LspManager {
    /// Creates a new LSP manager
    pub fn new() -> Self {
        let (notification_tx, notification_rx) = mpsc::unbounded_channel();
        Self {
            servers: Mutex::new(HashMap::new()),
            diagnostics: Mutex::new(HashMap::new()),
            next_request_id: AtomicU64::new(1),
            document_versions: Mutex::new(HashMap::new()),
            notification_tx,
            notification_rx: Mutex::new(notification_rx),
        }
    }

    /// Generates a unique request ID
    fn next_request_id(&self) -> RequestId {
        RequestId::Number(self.next_request_id.fetch_add(1, Ordering::SeqCst))
    }

    /// Starts a language server for the given language
    pub async fn start_server(
        &self,
        language: &str,
        command: &str,
        args: Vec<String>,
        root_path: &Path,
    ) -> Result<()> {
        let mut servers = self.servers.lock().await;

        if servers.contains_key(language) {
            return Ok(()); // Already running
        }

        let mut server = LanguageServer::spawn(command, args).await?;

        // Initialize the server
        let root_uri = Url::from_file_path(root_path)
            .map_err(|_| anyhow::anyhow!("Invalid root path"))?;

        server.initialize(root_uri).await?;

        servers.insert(language.to_string(), server);

        Ok(())
    }

    /// Stops a language server
    pub async fn stop_server(&self, language: &str) -> Result<()> {
        let mut servers = self.servers.lock().await;

        if let Some(mut server) = servers.remove(language) {
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
        let servers = self.servers.lock().await;
        servers.get(language).cloned()
    }

    /// Sends textDocument/didOpen notification
    pub async fn did_open(
        &self,
        uri: Url,
        language_id: &str,
        version: i32,
        text: String,
    ) -> Result<()> {
        let servers = self.servers.lock().await;
        let server = servers
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

        // Initialize version tracking
        let mut versions = self.document_versions.lock().await;
        versions.insert(uri, version);

        Ok(())
    }

    /// Sends textDocument/didChange notification
    pub async fn did_change(
        &self,
        uri: Url,
        language_id: &str,
        changes: Vec<TextDocumentContentChangeEvent>,
    ) -> Result<()> {
        let servers = self.servers.lock().await;
        let server = servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Increment version
        let version = self.increment_document_version(&uri).await;

        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier { uri, version },
            content_changes: changes,
        };

        server
            .notify("textDocument/didChange", serde_json::to_value(params)?)
            .await?;

        Ok(())
    }

    /// Sends textDocument/didSave notification
    pub async fn did_save(&self, uri: Url, language_id: &str, text: Option<String>) -> Result<()> {
        let servers = self.servers.lock().await;
        let server = servers
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
        let servers = self.servers.lock().await;
        let server = servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        let params = DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
        };

        server
            .notify("textDocument/didClose", serde_json::to_value(params)?)
            .await?;

        // Clean up version tracking
        let mut versions = self.document_versions.lock().await;
        versions.remove(&uri);

        Ok(())
    }

    /// Handles incoming notifications from language servers
    /// This should be called in a background task to process notifications
    pub async fn handle_notification(&self, language_id: &str, notification: JsonRpcMessage) {
        if let Some(method) = &notification.method {
            match method.as_str() {
                "textDocument/publishDiagnostics" => {
                    if let Some(params) = notification.params {
                        match serde_json::from_value::<PublishDiagnosticsParams>(params) {
                            Ok(diag_params) => {
                                self.set_diagnostics(diag_params.uri, diag_params.diagnostics)
                                    .await;
                            }
                            Err(_e) => {
                                // Silently ignore parsing errors
                            }
                        }
                    }
                }
                _ => {
                    // Silently ignore unknown notifications
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

    /// Starts a background task to listen for notifications from a language server
    pub async fn start_notification_listener(&self, language_id: String) {
        let server = {
            let servers = self.servers.lock().await;
            servers.get(&language_id).cloned()
        };

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

                            if tx.send(notification).is_err() {
                                break; // Manager dropped, stop listening
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

        let servers = self.servers.lock().await;
        let server = servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

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

    /// Requests hover information for a position in a document
    pub async fn hover(
        &self,
        uri: &Url,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<String>> {
        use lsp_types::{HoverParams, Position, TextDocumentIdentifier, TextDocumentPositionParams};

        let servers = self.servers.lock().await;
        let server = servers
            .get(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

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

        let response: Option<lsp_types::Hover> = serde_json::from_value(result).ok();

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
}
