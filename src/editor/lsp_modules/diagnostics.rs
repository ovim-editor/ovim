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

    /// Get all diagnostics for the current file
    pub fn all_diagnostics(&self) -> &[lsp_types::Diagnostic] {
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

        // Find diagnostic covering cursor column, or first on line
        let diagnostic = diagnostics
            .iter()
            .find(|d| {
                let start = d.range.start.character as usize;
                let end = d.range.end.character as usize;
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
