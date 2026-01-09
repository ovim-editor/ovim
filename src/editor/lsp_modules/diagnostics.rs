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
}
