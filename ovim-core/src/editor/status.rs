//! Status message, toasts, the diagnostic badge and diagnostic navigation.

use super::{Editor, Toast, ToastRequest};
use crate::unicode::GraphemeCol;

impl Editor {
    /// Gets cached diagnostic count (sync, suitable for UI rendering)
    pub fn cached_diagnostic_count(&self) -> (usize, usize, usize, usize) {
        if self.diagnostics_cache_stale() {
            return (0, 0, 0, 0);
        }
        self.lsp.state.diagnostic_count
    }

    /// Whether the diagnostic badge has been dismissed by double-Escape
    pub fn diagnostic_badge_dismissed(&self) -> bool {
        self.ui_panels.diagnostic_badge_dismissed
    }

    /// Dismiss the diagnostic badge (called on double-Escape)
    pub fn dismiss_diagnostic_badge(&mut self) {
        self.ui_panels.diagnostic_badge_dismissed = true;
    }

    /// Called when diagnostic counts change to potentially un-dismiss the badge
    pub fn on_diagnostic_counts_changed(&mut self, errors: usize, warnings: usize) {
        let new_count = (errors, warnings);
        if new_count != self.ui_panels.diagnostic_badge_last_count {
            self.ui_panels.diagnostic_badge_last_count = new_count;
            self.ui_panels.diagnostic_badge_dismissed = false;
        }
    }

    /// Set the latest user-facing editor status message.
    pub fn set_status_message(&mut self, message: impl Into<String>) {
        self.ui_panels.status_message = message.into();
        self.mark_dirty();
    }

    /// Return the latest user-facing editor status message.
    pub fn status_message(&self) -> &str {
        &self.ui_panels.status_message
    }

    /// Clear the current user-facing editor status message.
    pub fn clear_status_message(&mut self) {
        self.set_status_message(String::new());
    }

    /// Push a toast notification into the top-right toast center.
    pub fn push_toast(&mut self, request: ToastRequest) -> u64 {
        let id = self.ui_panels.toast_center.push(request);
        self.mark_dirty();
        id
    }

    /// Returns true if there is at least one visible toast.
    pub fn has_visible_toasts(&self) -> bool {
        self.ui_panels.toast_center.has_visible()
    }

    /// Returns visible toasts ordered newest-first.
    pub fn visible_toasts_newest_first(&self, max: usize) -> Vec<Toast> {
        self.ui_panels.toast_center.visible_toasts_newest_first(max)
    }

    /// Dismiss the newest visible toast, if any.
    pub fn dismiss_latest_toast(&mut self) -> bool {
        let dismissed = self.ui_panels.toast_center.dismiss_latest_visible();
        if dismissed {
            self.mark_dirty();
        }
        dismissed
    }

    /// Returns true if either diagnostics badge or toast overlay has visible content.
    pub fn has_top_right_overlay(&self) -> bool {
        let (errors, warnings, _, _) = self.cached_diagnostic_count();
        let diagnostic_visible = !self.diagnostic_badge_dismissed() && (errors > 0 || warnings > 0);
        diagnostic_visible || self.has_visible_toasts()
    }

    /// Dismiss one top-right overlay item (newest toast first, then diagnostic badge).
    pub fn dismiss_top_right_overlay(&mut self) -> bool {
        if self.dismiss_latest_toast() {
            return true;
        }

        let (errors, warnings, _, _) = self.cached_diagnostic_count();
        let diagnostic_visible = !self.diagnostic_badge_dismissed() && (errors > 0 || warnings > 0);
        if diagnostic_visible {
            self.dismiss_diagnostic_badge();
            return true;
        }

        false
    }

    /// Get last escape time for double-Escape detection
    pub fn last_escape_time(&self) -> Option<std::time::Instant> {
        self.ui_panels.last_escape_time
    }

    /// Set last escape time
    pub fn set_last_escape_time(&mut self, time: std::time::Instant) {
        self.ui_panels.last_escape_time = Some(time);
    }

    /// Clear last escape time
    pub fn clear_last_escape_time(&mut self) {
        self.ui_panels.last_escape_time = None;
    }

    /// Jump to next diagnostic (]d).
    pub fn goto_next_diagnostic(&mut self) {
        self.goto_diagnostic(true, false);
    }

    /// Jump to previous diagnostic ([d).
    pub fn goto_prev_diagnostic(&mut self) {
        self.goto_diagnostic(false, false);
    }

    /// Navigate error diagnostics only (]D / [D).
    pub fn goto_error_diagnostic(&mut self, forward: bool) {
        self.goto_diagnostic(forward, true);
    }

    fn goto_diagnostic(&mut self, forward: bool, errors_only: bool) {
        let line = self.buffer().cursor().line();
        let current = (
            line,
            self.col_to_utf16(line, self.buffer().cursor().col().0),
        );
        let positions = self
            .lsp
            .state
            .current_file_diagnostics
            .iter()
            .filter(|diagnostic| {
                !errors_only
                    || diagnostic
                        .severity
                        .unwrap_or(lsp_types::DiagnosticSeverity::ERROR)
                        == lsp_types::DiagnosticSeverity::ERROR
            })
            .map(|diagnostic| {
                (
                    diagnostic.range.start.line as usize,
                    diagnostic.range.start.character,
                )
            });
        let target = if forward {
            positions
                .clone()
                .filter(|position| *position > current)
                .min()
                .or_else(|| positions.min())
        } else {
            positions
                .clone()
                .filter(|position| *position < current)
                .max()
                .or_else(|| positions.max())
        };
        if let Some((line, character)) = target {
            let col = self.utf16_to_grapheme_col(line, character);
            self.buffer_mut()
                .cursor_mut()
                .set_position(line, GraphemeCol(col));
        }
    }
}

impl Editor {
    /// Inject diagnostics for testing diagnostic navigation.
    /// Sets diagnostics for the current file (test helper).
    pub fn set_test_diagnostics(&mut self, diagnostics: Vec<lsp_types::Diagnostic>) {
        self.lsp.state.set_current_file_diagnostics(diagnostics);
        self.lsp.state.diagnostics_file_path = self.buffer().file_path().map(|p| p.to_string());
        // Anchor against the current rope so tests exercise the same
        // edit-log projection path as the real refresh. (OV-00328)
        let rope = self.buffer().rope().clone();
        let version = self.buffer().version() as u64;
        self.lsp
            .state
            .anchor_current_file_diagnostics(&rope, version);
    }
}
