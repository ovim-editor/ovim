//! LSP diagnostics handling
//!
//! This module handles LSP diagnostics (errors, warnings, hints).
//! It provides diagnostic querying, caching, and display functionality.

use super::super::Editor;
use crate::lsp::uri_from_file_path;

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

    /// Get total diagnostic count (errors, warnings, info, hints) from cached diagnostics
    pub async fn get_diagnostic_count(&self) -> (usize, usize, usize, usize) {
        self.lsp.state.diagnostic_count
    }

    /// Spawn a background diagnostics refresh for the current file.
    ///
    /// INVARIANT: Callers must sync pending edits to the LSP server
    /// (`send_lsp_changes_if_modified`) before calling this.  Use
    /// `sync_lsp_and_refresh_diagnostics()` which enforces this ordering.
    pub fn spawn_diagnostic_cache_refresh(&mut self) {
        // Catch ordering violations in dev builds.  If the document sync
        // state is still dirty, diagnostics will be fetched against stale
        // server state — the exact bug we fixed by colocating sync + refresh.
        if let Some(file_path) = self.buffer().file_path() {
            if let Some(sync_state) = self.lsp.state.document_sync.get(file_path) {
                debug_assert!(
                    !sync_state.is_modified(),
                    "spawn_diagnostic_cache_refresh called while document sync is dirty \
                     for {} — send_lsp_changes_if_modified must run first",
                    file_path
                );
            }
        }

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

        // If diagnostics are already in flight, Slot::fire() will cancel the
        // old request and start a fresh one. This is correct: if new diagnostics
        // arrived via publishDiagnostics while a fetch was in progress, the
        // in-flight fetch has stale data and should be replaced.

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
            let task_result = crate::editor::lsp_slot::DiagnosticResult {
                file_path: file_path_for_task,
                buffer_version,
                lsp_version: doc_version,
                lsp_sent_version: last_sent,
                count: diagnostic_counts(&diagnostics),
                diagnostics,
                deferred: last_sent < doc_version,
            };

            let _ = tx.send(Ok(task_result));
        });

        self.lsp
            .slots
            .diagnostics
            .fire(task, rx, buffer_version as u64);
    }

    /// Poll background diagnostics refresh responses without blocking the UI tick.
    pub fn poll_pending_diagnostic_refresh_response(&mut self) -> bool {
        let timeout = std::time::Duration::from_secs(15);
        let Some(result) = self.lsp.slots.diagnostics.poll_with_timeout(timeout) else {
            return false;
        };

        match result {
            Ok(result) => {
                self.lsp.state.current_file_lsp_version = result.lsp_version;
                self.lsp.state.current_file_lsp_sent_version = result.lsp_sent_version;

                if result.deferred {
                    self.lsp.slots.diagnostics.invalidate();
                    return false;
                }

                // Wrong file — ignore entirely.
                if self.buffer().file_path() != Some(result.file_path.as_str()) {
                    self.lsp.slots.diagnostics.invalidate();
                    return false;
                }

                // Always store and display the latest diagnostics — they're the
                // best data we have.  Showing slightly stale positions during
                // editing is better UX than hiding all feedback for 150ms+.
                // If the buffer changed since spawn, also request a fresh set.
                self.lsp.state.diagnostic_count = result.count;
                self.on_diagnostic_counts_changed(result.count.0, result.count.1);
                self.lsp.state.current_file_diagnostics = result.diagnostics;
                self.lsp.state.diagnostics_file_path = Some(result.file_path);

                // Build unified decorations from the new diagnostics.
                let rope = self.buffer().rope().clone();
                let diag_decs = crate::editor::decoration::decorations_from_diagnostics(
                    &self.lsp.state.current_file_diagnostics,
                    &rope,
                );
                self.decorations.replace_source(
                    crate::editor::decoration::DecorationSource::Diagnostic,
                    diag_decs,
                    &rope,
                );

                if self.buffer().version() != result.buffer_version {
                    // Buffer was edited during the fetch — request a fresh set
                    // for the current content.  The stale diagnostics stay visible
                    // until the refresh completes (better than blank).
                    self.lsp.slots.diagnostics.invalidate();
                }

                true
            }
            Err(e) => {
                crate::lsp_warn!("LSP", "Diagnostics refresh failed: {}", e);
                self.lsp.slots.diagnostics.invalidate();
                false
            }
        }
    }

    /// Returns true if the cached diagnostics are for a different file.
    ///
    /// Content edits do NOT make diagnostics stale — showing slightly
    /// out-of-date diagnostics (possibly at wrong positions) is better UX
    /// than hiding all feedback for 150ms+ on every keystroke.  Fresh
    /// diagnostics replace stale ones atomically when the LSP responds.
    pub(crate) fn diagnostics_cache_stale(&self) -> bool {
        self.lsp.state.diagnostics_file_path.as_deref() != self.buffer().file_path()
    }

    /// Get diagnostics for a specific line from cached diagnostics
    pub fn diagnostics_for_line(&self, line: usize) -> Vec<&lsp_types::Diagnostic> {
        if self.diagnostics_cache_stale() {
            return Vec::new();
        }
        let result: Vec<_> = self
            .lsp
            .state
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
                self.lsp
                    .state
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
        let col = self.buffer().cursor().col().0;
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
    use crate::editor::lsp_slot::DiagnosticResult;
    use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
    use tokio::sync::oneshot;

    /// Helper: fire a pre-built `DiagnosticResult` into the diagnostics slot so
    /// that `poll_pending_diagnostic_refresh_response` can pick it up immediately.
    fn fire_diagnostic_result(editor: &mut Editor, result: DiagnosticResult) {
        let buffer_version = result.buffer_version as u64;
        let (tx, rx) = oneshot::channel::<anyhow::Result<DiagnosticResult>>();
        tx.send(Ok(result)).unwrap();
        let task = tokio::spawn(async {});
        editor.lsp.slots.diagnostics.fire(task, rx, buffer_version);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn poll_pending_diagnostic_refresh_response_applies_latest_result() {
        let mut editor = Editor::with_content("class Test {}\n");
        let file_path = "/tmp/Test.java".to_string();
        editor.set_file_path(file_path.clone());

        let diagnostic = Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 5)),
            severity: Some(DiagnosticSeverity::WARNING),
            message: "Example warning".to_string(),
            ..Diagnostic::default()
        };

        let bv = editor.buffer().version();
        fire_diagnostic_result(
            &mut editor,
            DiagnosticResult {
                file_path,
                buffer_version: bv,
                lsp_version: 4,
                lsp_sent_version: 4,
                diagnostics: vec![diagnostic],
                count: (0, 1, 0, 0),
                deferred: false,
            },
        );

        assert!(editor.poll_pending_diagnostic_refresh_response());
        assert_eq!(editor.lsp.state.diagnostic_count, (0, 1, 0, 0));
        assert_eq!(editor.lsp.state.current_file_diagnostics.len(), 1);
        assert_eq!(editor.lsp.state.current_file_lsp_version, 4);
        assert_eq!(editor.lsp.state.current_file_lsp_sent_version, 4);
    }

    /// When the buffer is edited between spawning a diagnostic refresh and
    /// receiving the result, diagnostics should still be stored and displayed
    /// (stale data is better than no data), and a re-request should be scheduled.
    #[tokio::test(flavor = "current_thread")]
    async fn poll_keeps_diagnostics_when_buffer_edited_during_fetch() {
        use crate::editor::decoration::{
            Decoration, DecorationPlacement, DecorationSource, DecorationStyle,
        };

        let mut editor = Editor::with_content("let x = 1;\n");
        let file_path = "/tmp/test.rs".to_string();
        editor.set_file_path(file_path.clone());

        let initial_version = editor.buffer().version();

        // Simulate a diagnostic decoration already present from a prior refresh.
        let rope = editor.buffer().rope().clone();
        editor.decorations.replace_source(
            DecorationSource::Diagnostic,
            vec![Decoration {
                placement: DecorationPlacement::EndOfLine { char_offset: 0 },
                source: DecorationSource::Diagnostic,
                text: "old error".to_string(),
                display_width: 9,
                style: DecorationStyle::new(crate::color::Color::Red),
                priority: 0,
            }],
            &rope,
        );
        assert_eq!(editor.decorations.for_line(0).len(), 1);

        // Build a result that was spawned at the initial buffer version.
        let diagnostic = Diagnostic {
            range: Range::new(Position::new(0, 4), Position::new(0, 5)),
            severity: Some(DiagnosticSeverity::ERROR),
            message: "unused variable".to_string(),
            ..Diagnostic::default()
        };

        fire_diagnostic_result(
            &mut editor,
            DiagnosticResult {
                file_path,
                buffer_version: initial_version,
                lsp_version: 2,
                lsp_sent_version: 2,
                diagnostics: vec![diagnostic],
                count: (1, 0, 0, 0),
                deferred: false,
            },
        );

        // Simulate a buffer edit AFTER the refresh was spawned.
        editor
            .buffer_mut()
            .insert_text_at(0, crate::unicode::CharCol::ZERO, "// ");

        assert_ne!(
            editor.buffer().version(),
            initial_version,
            "buffer version should have changed after edit"
        );

        // Poll should succeed (result ready) and detect the version mismatch.
        let changed = editor.poll_pending_diagnostic_refresh_response();
        assert!(changed);

        // File path still matches, so diagnostics are NOT stale (show-until-replaced).
        assert!(!editor.diagnostics_cache_stale());
        // A refresh should be requested for the current buffer content.
        assert!(editor.lsp.slots.diagnostics.is_stale());

        // Diagnostic decorations should PERSIST (stale data is better than blank).
        assert!(
            !editor.decorations.for_line(0).is_empty(),
            "diagnostic decorations should persist when buffer was edited during fetch"
        );
    }

    /// Verify that when the buffer hasn't changed, decorations ARE applied and
    /// diagnostics are marked valid.
    #[tokio::test(flavor = "current_thread")]
    async fn poll_applies_decorations_when_buffer_unchanged() {
        let mut editor = Editor::with_content("let x = 1;\n");
        let file_path = "/tmp/test.rs".to_string();
        editor.set_file_path(file_path.clone());

        let buffer_version = editor.buffer().version();

        let diagnostic = Diagnostic {
            range: Range::new(Position::new(0, 4), Position::new(0, 5)),
            severity: Some(DiagnosticSeverity::ERROR),
            message: "unused variable".to_string(),
            ..Diagnostic::default()
        };

        fire_diagnostic_result(
            &mut editor,
            DiagnosticResult {
                file_path,
                buffer_version,
                lsp_version: 2,
                lsp_sent_version: 2,
                diagnostics: vec![diagnostic],
                count: (1, 0, 0, 0),
                deferred: false,
            },
        );

        // No buffer edits — poll should apply decorations.
        let changed = editor.poll_pending_diagnostic_refresh_response();
        assert!(changed);
        assert!(!editor.diagnostics_cache_stale());

        // Decorations should be present.
        assert!(
            !editor.decorations.for_line(0).is_empty(),
            "diagnostic decorations should be applied when buffer is unchanged"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn poll_pending_diagnostic_refresh_response_requeues_deferred_result() {
        let mut editor = Editor::with_content("class Test {}\n");
        let file_path = "/tmp/Test.java".to_string();
        editor.set_file_path(file_path.clone());

        let bv = editor.buffer().version();
        fire_diagnostic_result(
            &mut editor,
            DiagnosticResult {
                file_path,
                buffer_version: bv,
                lsp_version: 5,
                lsp_sent_version: 4,
                diagnostics: Vec::new(),
                count: (0, 0, 0, 0),
                deferred: true,
            },
        );

        assert!(!editor.poll_pending_diagnostic_refresh_response());
        assert!(editor.lsp.slots.diagnostics.is_stale());
        assert!(editor.lsp.state.current_file_diagnostics.is_empty());
        assert_eq!(editor.lsp.state.current_file_lsp_version, 5);
        assert_eq!(editor.lsp.state.current_file_lsp_sent_version, 4);
    }
}
