//! LSP diagnostics handling
//!
//! This module handles LSP diagnostics (errors, warnings, hints).
//! It provides diagnostic querying, caching, and display functionality.

use super::super::Editor;
use crate::editor::lsp_state::{
    DiagnosticRefreshTaskResult, PendingDiagnosticRefresh, PendingLspRequest,
};
use crate::lsp::uri_from_file_path;
use tokio::sync::oneshot::error::TryRecvError;

fn diagnostic_counts(diagnostics: &[lsp_types::Diagnostic]) -> (usize, usize, usize, usize) {
    let mut errors = 0;
    let mut warnings = 0;
    let mut info = 0;
    let mut hints = 0;
    for diagnostic in diagnostics {
        match diagnostic.severity {
            Some(lsp_types::DiagnosticSeverity::ERROR) => errors += 1,
            Some(lsp_types::DiagnosticSeverity::WARNING) => warnings += 1,
            Some(lsp_types::DiagnosticSeverity::INFORMATION) => info += 1,
            Some(lsp_types::DiagnosticSeverity::HINT) => hints += 1,
            None => warnings += 1,
            _ => {}
        }
    }
    (errors, warnings, info, hints)
}

impl Editor {
    /// Get current file diagnostics from LSP
    pub async fn get_current_file_diagnostics(&self) -> Option<Vec<lsp_types::Diagnostic>> {
        let lsp = self.lsp.state.lsp_manager.as_ref()?;
        let file_path = self.buffer().file_path()?;
        let uri = uri_from_file_path(file_path)?;
        let diagnostics = lsp.get_diagnostics(&uri).await;
        Some(diagnostics)
    }

    /// Query and cache diagnostics for the current file
    pub async fn update_diagnostics(&mut self) {
        if let Some(lsp) = &self.lsp.state.lsp_manager {
            if let Some(file_path) = self.buffer().file_path() {
                let file_path = file_path.to_string();
                if let Some(uri) = uri_from_file_path(&file_path) {
                    // OV-00161: Skip caching if there are unsent edits
                    let doc_version = lsp.get_document_version(&uri).await;
                    let last_sent = lsp.get_last_sent_version(&uri).await;
                    self.lsp.state.current_file_lsp_version = doc_version;
                    self.lsp.state.current_file_lsp_sent_version = last_sent;
                    if last_sent < doc_version {
                        self.lsp.state.diagnostics_refresh_requested = true;
                        return;
                    }

                    // Snapshot buffer version BEFORE async fetch — if the buffer
                    // changes during the fetch, the stamp won't match and the
                    // display-side staleness check will hide them.
                    let version_before = self.buffer().version();
                    let diagnostics = lsp.get_diagnostics(&uri).await;
                    // Compute count directly from fetched diagnostics (not from cached count)
                    let mut errors = 0;
                    let mut warnings = 0;
                    let mut info = 0;
                    let mut hints = 0;
                    for d in &diagnostics {
                        match d.severity {
                            Some(lsp_types::DiagnosticSeverity::ERROR) => errors += 1,
                            Some(lsp_types::DiagnosticSeverity::WARNING) => warnings += 1,
                            Some(lsp_types::DiagnosticSeverity::INFORMATION) => info += 1,
                            Some(lsp_types::DiagnosticSeverity::HINT) => hints += 1,
                            None => warnings += 1,
                            _ => {}
                        }
                    }
                    self.lsp.state.diagnostic_count = (errors, warnings, info, hints);
                    // Cache full diagnostic list with version provenance
                    self.lsp.state.current_file_diagnostics = diagnostics;
                    self.lsp.state.diagnostics_buffer_version = version_before;
                    self.lsp.state.diagnostics_file_path = Some(file_path);
                    self.lsp.state.diagnostics_lsp_version = doc_version;
                    return;
                }
            }
        }
        self.lsp.state.current_file_diagnostics.clear();
        self.lsp.state.diagnostics_file_path = None;
    }

    /// Get total diagnostic count (errors, warnings, info, hints) from cached diagnostics
    pub async fn get_diagnostic_count(&self) -> (usize, usize, usize, usize) {
        self.lsp.state.diagnostic_count
    }

    /// Spawn a background diagnostics refresh for the current file.
    pub fn spawn_diagnostic_cache_refresh(&mut self) {
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            self.lsp.state.current_file_diagnostics.clear();
            self.lsp.state.diagnostic_count = (0, 0, 0, 0);
            self.lsp.state.diagnostics_file_path = None;
            return;
        };

        let Some(file_path) = self.buffer().file_path().map(str::to_string) else {
            self.lsp.state.current_file_diagnostics.clear();
            self.lsp.state.diagnostic_count = (0, 0, 0, 0);
            self.lsp.state.diagnostics_file_path = None;
            return;
        };

        let Some(uri) = uri_from_file_path(&file_path) else {
            self.lsp.state.current_file_diagnostics.clear();
            self.lsp.state.diagnostic_count = (0, 0, 0, 0);
            self.lsp.state.diagnostics_file_path = None;
            return;
        };

        let buffer_version = self.buffer().version();
        if self
            .lsp.state
            .pending_diagnostic_refresh
            .as_ref()
            .is_some_and(|pending| {
                pending.file_path == file_path && pending.buffer_version == buffer_version
            })
        {
            return;
        }

        if let Some(pending) = self.lsp.state.pending_diagnostic_refresh.take() {
            pending.request.task.abort();
        }

        self.lsp.state.diagnostic_refresh_seq =
            self.lsp.state.diagnostic_refresh_seq.wrapping_add(1);
        let seq = self.lsp.state.diagnostic_refresh_seq;
        let file_path_for_task = file_path.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let doc_version = lsp.get_document_version(&uri).await;
            let last_sent = lsp.get_last_sent_version(&uri).await;
            let diagnostics = if last_sent < doc_version {
                Vec::new()
            } else {
                lsp.get_diagnostics(&uri).await
            };
            let task_result = DiagnosticRefreshTaskResult {
                file_path: file_path_for_task,
                buffer_version,
                lsp_version: doc_version,
                lsp_sent_version: last_sent,
                count: diagnostic_counts(&diagnostics),
                diagnostics,
                deferred: last_sent < doc_version,
            };

            let _ = tx.send(Ok(task_result.clone()));
            Ok(task_result)
        });

        self.lsp.state.pending_diagnostic_refresh = Some(PendingDiagnosticRefresh {
            seq,
            file_path,
            buffer_version,
            request: PendingLspRequest {
                task,
                receiver: rx,
                started: std::time::Instant::now(),
            },
        });
    }

    /// Poll background diagnostics refresh responses without blocking the UI tick.
    pub fn poll_pending_diagnostic_refresh_response(&mut self) -> bool {
        let Some(mut pending) = self.lsp.state.pending_diagnostic_refresh.take() else {
            return false;
        };

        match pending.request.receiver.try_recv() {
            Ok(Ok(result)) => {
                if pending.seq != self.lsp.state.diagnostic_refresh_seq {
                    return false;
                }

                self.lsp.state.current_file_lsp_version = result.lsp_version;
                self.lsp.state.current_file_lsp_sent_version = result.lsp_sent_version;

                if result.deferred {
                    self.lsp.state.diagnostics_refresh_requested = true;
                    return false;
                }

                if self.buffer().file_path() != Some(result.file_path.as_str())
                    || self.buffer().version() != result.buffer_version
                {
                    self.lsp.state.diagnostics_refresh_requested = true;
                    return false;
                }

                self.lsp.state.diagnostic_count = result.count;
                self.on_diagnostic_counts_changed(result.count.0, result.count.1);
                self.lsp.state.current_file_diagnostics = result.diagnostics;
                self.lsp.state.diagnostics_buffer_version = result.buffer_version;
                self.lsp.state.diagnostics_file_path = Some(result.file_path);
                self.lsp.state.diagnostics_lsp_version = result.lsp_version;
                self.record_diagnostic_query_duration(
                    pending.request.started.elapsed().as_micros() as u64,
                );
                true
            }
            Ok(Err(e)) => {
                crate::lsp_warn!("LSP", "Diagnostics refresh failed: {}", e);
                self.lsp.state.diagnostics_refresh_requested = true;
                false
            }
            Err(TryRecvError::Empty) => {
                self.lsp.state.pending_diagnostic_refresh = Some(pending);
                false
            }
            Err(TryRecvError::Closed) => false,
        }
    }

    /// Updates the cached diagnostic count (should be called when diagnostics change)
    pub async fn update_diagnostic_cache(&mut self) {
        let start = std::time::Instant::now();

        // Snapshot buffer version BEFORE async fetches — if the buffer changes
        // during the fetch, the stamp won't match and diagnostics are hidden.
        let version_before = self.buffer().version();

        // Query diagnostic count from LSP manager
        let count = if let Some(lsp) = &self.lsp.state.lsp_manager {
            if let Some(file_path) = self.buffer().file_path().map(str::to_string) {
                if let Some(uri) = crate::lsp::uri_from_file_path(&file_path) {
                    // OV-00161: Check for unsent edits.  If the document version
                    // has been bumped (in did_change) but the flush hasn't happened
                    // yet, the diagnostics in the DashMap are from an older content
                    // version.  Defer the cache update so we don't stamp stale
                    // diagnostics with the current buffer version.
                    let doc_version = lsp.get_document_version(&uri).await;
                    let last_sent = lsp.get_last_sent_version(&uri).await;
                    self.lsp.state.current_file_lsp_sent_version = last_sent;
                    if last_sent < doc_version {
                        crate::log_debug!(
                            "diagnostics",
                            "update_diagnostic_cache: deferring (unsent edits: last_sent={} doc_version={})",
                            last_sent,
                            doc_version
                        );
                        // Ensure we retry on the next tick
                        self.lsp.state.diagnostics_refresh_requested = true;
                        // Still update current_file_lsp_version so the rendering
                        // guard correctly hides stale cached diagnostics.
                        self.lsp.state.current_file_lsp_version = doc_version;
                        return;
                    }

                    let diagnostics = lsp.get_diagnostics(&uri).await;
                    let c = diagnostic_counts(&diagnostics);
                    crate::log_debug!(
                        "diagnostics",
                        "update_diagnostic_cache: uri={} count={:?} doc_version={}",
                        uri.as_str(),
                        c,
                        doc_version
                    );
                    // Store LSP version provenance alongside count
                    self.lsp.state.current_file_lsp_version = doc_version;
                    self.lsp.state.current_file_diagnostics = diagnostics;
                    self.lsp.state.diagnostics_buffer_version = version_before;
                    self.lsp.state.diagnostics_file_path = Some(file_path.clone());
                    self.lsp.state.diagnostics_lsp_version = doc_version;
                    c
                } else {
                    crate::log_debug!(
                        "diagnostics",
                        "update_diagnostic_cache: uri_from_file_path failed for {}",
                        file_path
                    );
                    (0, 0, 0, 0)
                }
            } else {
                crate::log_debug!("diagnostics", "update_diagnostic_cache: no file_path");
                (0, 0, 0, 0)
            }
        } else {
            (0, 0, 0, 0)
        };

        self.lsp.state.diagnostic_count = count;

        // Reset badge dismissal if counts changed
        self.on_diagnostic_counts_changed(count.0, count.1);

        if self.lsp.state.current_file_diagnostics.is_empty() {
            crate::log_debug!(
                "diagnostics",
                "update_diagnostic_cache: no diagnostics cached for current file (count was {:?})",
                count
            );
            self.lsp.state.current_file_diagnostics.clear();
            self.lsp.state.diagnostics_file_path = None;
        }

        let duration = start.elapsed().as_micros() as u64;
        self.record_diagnostic_query_duration(duration);
    }

    /// Returns true if the cached diagnostics are stale (buffer or LSP version mismatch).
    pub(crate) fn diagnostics_cache_stale(&self) -> bool {
        // File path mismatch: diagnostics were cached for a different file.
        let current_path = self.buffer().file_path();
        if self.lsp.state.diagnostics_file_path.as_deref() != current_path {
            return true;
        }

        // If this document has local edits not yet sent to LSP, any cached diagnostics
        // are potentially for older content and must be hidden.
        if let Some(file_path) = self.buffer().file_path() {
            if self
                .lsp.state
                .document_sync
                .get(file_path)
                .is_some_and(|state| state.is_modified())
            {
                return true;
            }
        }

        // Buffer version mismatch: buffer was edited since diagnostics were cached.
        if self.lsp.state.diagnostics_buffer_version != self.buffer().version() {
            return true;
        }
        // LSP version mismatch: a new document version was assigned (via did_change)
        // since diagnostics were cached — the server may not have processed it yet.
        if self.lsp.state.diagnostics_lsp_version != self.lsp.state.current_file_lsp_version {
            return true;
        }
        false
    }

    /// Get diagnostics for a specific line from cached diagnostics
    pub fn diagnostics_for_line(&self, line: usize) -> Vec<&lsp_types::Diagnostic> {
        if self.diagnostics_cache_stale() {
            return Vec::new();
        }
        let result: Vec<_> = self
            .lsp.state
            .current_file_diagnostics
            .iter()
            .filter(|d| d.range.start.line as usize == line)
            .collect();
        // Only log if there are cached diagnostics (to avoid spam)
        if !self.lsp.state.current_file_diagnostics.is_empty() && result.is_empty() && line < 10 {
            crate::log_debug!(
                "diagnostics",
                "diagnostics_for_line({}): 0 matches in {} cached diagnostics, first diag line={}",
                line,
                self.lsp.state.current_file_diagnostics.len(),
                self.lsp.state
                    .current_file_diagnostics
                    .first()
                    .map(|d| d.range.start.line)
                    .unwrap_or(0)
            );
        }
        result
    }

    /// Get the current diagnostic at the cursor position
    pub fn current_diagnostic(&self) -> Option<String> {
        if self.diagnostics_cache_stale() {
            return None;
        }
        let line = self.buffer().cursor().line();
        let diagnostics = &self.lsp.state.current_file_diagnostics;

        diagnostics
            .iter()
            .find(|d| d.range.start.line as usize == line)
            .map(|d| d.message.clone())
    }

    /// Get the total number of diagnostics
    pub fn diagnostic_count(&self) -> usize {
        if self.diagnostics_cache_stale() {
            return 0;
        }
        let diagnostics = &self.lsp.state.current_file_diagnostics;
        diagnostics.len()
    }

    /// Get all diagnostics for the current file
    pub fn all_diagnostics(&self) -> &[lsp_types::Diagnostic] {
        if self.diagnostics_cache_stale() {
            return &[];
        }
        &self.lsp.state.current_file_diagnostics
    }

    /// Show diagnostic at cursor in hover popup (like vim.diagnostic.open_float())
    pub fn show_diagnostic_at_cursor(&mut self) {
        use crate::mode::Mode;

        let line = self.buffer().cursor().line();
        let col = self.buffer().cursor().col();
        let diagnostics = self.diagnostics_for_line(line);

        if diagnostics.is_empty() {
            self.set_lsp_status("No diagnostics at cursor".to_string());
            return;
        }

        // Convert diagnostic positions from UTF-16 to char columns for comparison
        let line_text: String = {
            let rope = self.buffer().rope();
            if line < rope.len_lines() {
                rope.line(line).chars().take_while(|&c| c != '\n').collect()
            } else {
                String::new()
            }
        };

        // Pick diagnostic under cursor when possible; otherwise pick the nearest one
        // on the line by column distance (instead of arbitrary first entry).
        let diagnostic = diagnostics
            .iter()
            .min_by_key(|d| {
                let start = crate::lsp::utf16_to_char_col(&line_text, d.range.start.character);
                let end = crate::lsp::utf16_to_char_col(&line_text, d.range.end.character);
                if col >= start && col <= end {
                    0usize
                } else if col < start {
                    start - col
                } else {
                    col.saturating_sub(end)
                }
            })
            .unwrap();

        // Format severity with markdown for nice rendering
        let severity_label = match diagnostic.severity {
            Some(lsp_types::DiagnosticSeverity::ERROR) => "Error",
            Some(lsp_types::DiagnosticSeverity::WARNING) => "Warning",
            Some(lsp_types::DiagnosticSeverity::INFORMATION) => "Info",
            Some(lsp_types::DiagnosticSeverity::HINT) => "Hint",
            _ => "Diagnostic",
        };

        // Build markdown-formatted message
        // **Severity**: Message
        // Source: source (if available)
        let mut message = format!("**{}**: {}", severity_label, diagnostic.message);

        // Add source if available (e.g., "rustc", "clippy")
        if let Some(ref source) = diagnostic.source {
            message.push_str(&format!("\n\n`{}`", source));
        }

        // Add diagnostic code if available
        if let Some(ref code) = diagnostic.code {
            let code_str = match code {
                lsp_types::NumberOrString::Number(n) => n.to_string(),
                lsp_types::NumberOrString::String(s) => s.clone(),
            };
            message.push_str(&format!(" `{}`", code_str));
        }
        self.lsp.state.hover_info = Some(message);
        self.lsp.state.hover_position = Some((line, col));
        self.lsp.state.hover_content_type = crate::editor::lsp_state::HoverContentType::Diagnostic;
        self.set_mode(Mode::HoverPreview);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::lsp_state::{
        DiagnosticRefreshTaskResult, PendingDiagnosticRefresh, PendingLspRequest,
    };
    use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
    use tokio::sync::oneshot;

    #[tokio::test(flavor = "current_thread")]
    async fn poll_pending_diagnostic_refresh_response_applies_latest_result() {
        let mut editor = Editor::with_content("class Test {}\n");
        let file_path = "/tmp/Test.java".to_string();
        editor.set_file_path(file_path.clone());
        editor.lsp.state.diagnostic_refresh_seq = 1;

        let diagnostic = Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 5)),
            severity: Some(DiagnosticSeverity::WARNING),
            message: "Example warning".to_string(),
            ..Diagnostic::default()
        };

        let (tx, receiver) = oneshot::channel::<anyhow::Result<DiagnosticRefreshTaskResult>>();
        tx.send(Ok(DiagnosticRefreshTaskResult {
            file_path: file_path.clone(),
            buffer_version: editor.buffer().version(),
            lsp_version: 4,
            lsp_sent_version: 4,
            diagnostics: vec![diagnostic],
            count: (0, 1, 0, 0),
            deferred: false,
        }))
        .unwrap();

        editor.lsp.state.pending_diagnostic_refresh = Some(PendingDiagnosticRefresh {
            seq: 1,
            file_path: file_path.clone(),
            buffer_version: editor.buffer().version(),
            request: PendingLspRequest {
                task: tokio::spawn(async move {
                    Ok(DiagnosticRefreshTaskResult {
                        file_path,
                        buffer_version: 0,
                        lsp_version: 0,
                        lsp_sent_version: 0,
                        diagnostics: Vec::new(),
                        count: (0, 0, 0, 0),
                        deferred: false,
                    })
                }),
                receiver,
                started: std::time::Instant::now(),
            },
        });

        assert!(editor.poll_pending_diagnostic_refresh_response());
        assert_eq!(editor.lsp.state.diagnostic_count, (0, 1, 0, 0));
        assert_eq!(editor.lsp.state.current_file_diagnostics.len(), 1);
        assert_eq!(editor.lsp.state.current_file_lsp_version, 4);
        assert_eq!(editor.lsp.state.current_file_lsp_sent_version, 4);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn poll_pending_diagnostic_refresh_response_requeues_deferred_result() {
        let mut editor = Editor::with_content("class Test {}\n");
        let file_path = "/tmp/Test.java".to_string();
        editor.set_file_path(file_path.clone());
        editor.lsp.state.diagnostic_refresh_seq = 1;

        let (tx, receiver) = oneshot::channel::<anyhow::Result<DiagnosticRefreshTaskResult>>();
        tx.send(Ok(DiagnosticRefreshTaskResult {
            file_path: file_path.clone(),
            buffer_version: editor.buffer().version(),
            lsp_version: 5,
            lsp_sent_version: 4,
            diagnostics: Vec::new(),
            count: (0, 0, 0, 0),
            deferred: true,
        }))
        .unwrap();

        editor.lsp.state.pending_diagnostic_refresh = Some(PendingDiagnosticRefresh {
            seq: 1,
            file_path,
            buffer_version: editor.buffer().version(),
            request: PendingLspRequest {
                task: tokio::spawn(async {
                    Ok(DiagnosticRefreshTaskResult {
                        file_path: String::new(),
                        buffer_version: 0,
                        lsp_version: 0,
                        lsp_sent_version: 0,
                        diagnostics: Vec::new(),
                        count: (0, 0, 0, 0),
                        deferred: false,
                    })
                }),
                receiver,
                started: std::time::Instant::now(),
            },
        });

        assert!(!editor.poll_pending_diagnostic_refresh_response());
        assert!(editor.lsp.state.diagnostics_refresh_requested);
        assert!(editor.lsp.state.current_file_diagnostics.is_empty());
        assert_eq!(editor.lsp.state.current_file_lsp_version, 5);
        assert_eq!(editor.lsp.state.current_file_lsp_sent_version, 4);
    }
}
