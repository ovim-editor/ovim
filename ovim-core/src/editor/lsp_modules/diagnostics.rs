//! LSP diagnostics handling
//!
//! This module handles LSP diagnostics (errors, warnings, hints).
//! It provides diagnostic querying, caching, and display functionality.

use super::super::Editor;
use crate::lsp::uri_from_file_path;

impl Editor {
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
                    // OV-00161: Skip caching if there are unsent edits
                    let doc_version = lsp.get_document_version(&uri).await;
                    let last_sent = lsp.get_last_sent_version(&uri).await;
                    self.lsp_state.current_file_lsp_version = doc_version;
                    if last_sent < doc_version {
                        self.lsp_state.diagnostics_refresh_requested = true;
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
                    self.lsp_state.diagnostic_count = (errors, warnings, info, hints);
                    // Cache full diagnostic list with version provenance
                    self.lsp_state.current_file_diagnostics = diagnostics;
                    self.lsp_state.diagnostics_buffer_version = version_before;
                    self.lsp_state.diagnostics_lsp_version = doc_version;
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

        // Snapshot buffer version BEFORE async fetches — if the buffer changes
        // during the fetch, the stamp won't match and diagnostics are hidden.
        let version_before = self.buffer().version();

        // Query diagnostic count from LSP manager
        let count = if let Some(lsp) = &self.lsp_state.lsp_manager {
            if let Some(file_path) = self.buffer().file_path() {
                if let Some(uri) = crate::lsp::uri_from_file_path(file_path) {
                    // OV-00161: Check for unsent edits.  If the document version
                    // has been bumped (in did_change) but the flush hasn't happened
                    // yet, the diagnostics in the DashMap are from an older content
                    // version.  Defer the cache update so we don't stamp stale
                    // diagnostics with the current buffer version.
                    let doc_version = lsp.get_document_version(&uri).await;
                    let last_sent = lsp.get_last_sent_version(&uri).await;
                    if last_sent < doc_version {
                        crate::log_debug!(
                            "diagnostics",
                            "update_diagnostic_cache: deferring (unsent edits: last_sent={} doc_version={})",
                            last_sent,
                            doc_version
                        );
                        // Ensure we retry on the next tick
                        self.lsp_state.diagnostics_refresh_requested = true;
                        // Still update current_file_lsp_version so the rendering
                        // guard correctly hides stale cached diagnostics.
                        self.lsp_state.current_file_lsp_version = doc_version;
                        return;
                    }

                    let c = lsp.count_diagnostics(&uri).await;
                    crate::log_debug!(
                        "diagnostics",
                        "update_diagnostic_cache: uri={} count={:?} doc_version={}",
                        uri.as_str(),
                        c,
                        doc_version
                    );
                    // Store LSP version provenance alongside count
                    self.lsp_state.current_file_lsp_version = doc_version;
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

        self.lsp_state.diagnostic_count = count;

        // Reset badge dismissal if counts changed
        self.on_diagnostic_counts_changed(count.0, count.1);

        if let Some(diagnostics) = self.get_current_file_diagnostics().await {
            crate::log_debug!(
                "diagnostics",
                "update_diagnostic_cache: got {} diagnostics for current file (count was {:?})",
                diagnostics.len(),
                count
            );
            if !diagnostics.is_empty() {
                crate::log_debug!(
                    "diagnostics",
                    "first diag: line={} msg={:.50}",
                    diagnostics[0].range.start.line,
                    diagnostics[0].message
                );
            }
            self.lsp_state.current_file_diagnostics = diagnostics;
            self.lsp_state.diagnostics_buffer_version = version_before;
            self.lsp_state.diagnostics_lsp_version =
                self.lsp_state.current_file_lsp_version;
        } else {
            crate::log_debug!(
                "diagnostics",
                "update_diagnostic_cache: get_current_file_diagnostics returned None (count was {:?})",
                count
            );
            self.lsp_state.current_file_diagnostics.clear();
        }

        let duration = start.elapsed().as_micros() as u64;
        self.record_diagnostic_query_duration(duration);
    }

    /// Returns true if the cached diagnostics are stale (buffer or LSP version mismatch).
    pub(crate) fn diagnostics_cache_stale(&self) -> bool {
        // Buffer version mismatch: buffer was edited since diagnostics were cached.
        if self.lsp_state.diagnostics_buffer_version != self.buffer().version() {
            return true;
        }
        // LSP version mismatch: a new document version was assigned (via did_change)
        // since diagnostics were cached — the server may not have processed it yet.
        if self.lsp_state.diagnostics_lsp_version
            != self.lsp_state.current_file_lsp_version
        {
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
            .lsp_state
            .current_file_diagnostics
            .iter()
            .filter(|d| d.range.start.line as usize == line)
            .collect();
        // Only log if there are cached diagnostics (to avoid spam)
        if !self.lsp_state.current_file_diagnostics.is_empty() && result.is_empty() && line < 10 {
            crate::log_debug!(
                "diagnostics",
                "diagnostics_for_line({}): 0 matches in {} cached diagnostics, first diag line={}",
                line,
                self.lsp_state.current_file_diagnostics.len(),
                self.lsp_state
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
        let diagnostics = &self.lsp_state.current_file_diagnostics;

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
        let diagnostics = &self.lsp_state.current_file_diagnostics;
        diagnostics.len()
    }

    /// Get all diagnostics for the current file
    pub fn all_diagnostics(&self) -> &[lsp_types::Diagnostic] {
        if self.diagnostics_cache_stale() {
            return &[];
        }
        &self.lsp_state.current_file_diagnostics
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

        // Find diagnostic covering cursor column, or first on line
        let diagnostic = diagnostics
            .iter()
            .find(|d| {
                let start = crate::lsp::utf16_to_char_col(&line_text, d.range.start.character);
                let end = crate::lsp::utf16_to_char_col(&line_text, d.range.end.character);
                col >= start && col <= end
            })
            .or_else(|| diagnostics.first())
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
        self.lsp_state.hover_info = Some(message);
        self.lsp_state.hover_position = Some((line, col));
        self.lsp_state.hover_content_type = crate::editor::lsp_state::HoverContentType::Diagnostic;
        self.set_mode(Mode::HoverPreview);
    }
}
