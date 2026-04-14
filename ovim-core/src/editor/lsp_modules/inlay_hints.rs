use crate::editor::Editor;
use lsp_types::Position;

impl Editor {
    /// Returns true when inlay hints need refreshing and the debounce
    /// window has elapsed. Delegates to `TrackedSlot::needs_refresh()`.
    pub fn inlay_hints_refresh_needed(&self) -> bool {
        if self.lsp.state.lsp_manager.is_none() {
            return false;
        }
        // Ensure we have a valid file with LSP support
        let Some(file_path) = self.buffer().file_path() else {
            return false;
        };
        if crate::syntax::LanguageRegistry::get_lsp_language_id(file_path).is_none() {
            return false;
        }
        self.lsp.slots.inlay_hints.needs_refresh()
    }

    /// Spawn a background inlay hint refresh for the current file.
    pub fn request_inlay_hints_refresh(&mut self) {
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            return;
        };

        let Some(file_path) = self.buffer().file_path().map(|p| p.to_string()) else {
            return;
        };

        let Some(uri) = crate::lsp::uri_from_file_path(&file_path) else {
            return;
        };

        let Some(language_id) =
            crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path)
        else {
            return;
        };
        let buffer_version = self.buffer().version();
        let start_line = 0;
        let end_line = self.buffer().line_count();
        let lsp_sent_version = self.lsp.state.current_file_lsp_sent_version;

        // Document sync is handled by send_lsp_changes_if_modified() earlier
        // in the tick. The task only makes the LSP request — no debouncer
        // interaction, avoiding races with the main event loop.

        let file_path_for_task = file_path.clone();
        let language_id = language_id.to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let range = lsp_types::Range {
                start: Position {
                    line: start_line as u32,
                    character: 0,
                },
                end: Position {
                    line: end_line as u32,
                    character: 0,
                },
            };

            let result = lsp
                .inlay_hints(&uri, range, &language_id)
                .await
                .map(|hints| crate::editor::lsp_slot::InlayHintResult {
                    request_key: crate::editor::lsp_state::InlayHintRequestKey {
                        file_path: file_path_for_task,
                        start_line,
                        end_line,
                        lsp_version: lsp_sent_version,
                    },
                    buffer_version,
                    synced_content: None,
                    synced_lsp_version: None,
                    hints,
                });

            let _ = tx.send(result);
        });

        // Slot::fire() cancels any previously in-flight inlay hint request.
        self.lsp
            .slots
            .inlay_hints
            .fire(task, rx, buffer_version as u64);
    }

    /// Get inlay hints for a specific line (0-indexed).
    pub fn inlay_hints_for_line(&self, line: usize) -> Vec<&lsp_types::InlayHint> {
        self.lsp
            .state
            .inlay_hints
            .iter()
            .filter(|h| h.position.line as usize == line)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_editor_does_not_need_refresh_without_lsp() {
        let editor = Editor::with_content("class Test {}\n");
        // No LSP enabled — should not request refresh.
        assert!(!editor.inlay_hints_refresh_needed());
    }

    #[test]
    fn invalidation_triggers_refresh() {
        let mut editor = Editor::with_content("class Test {}\n");
        editor.enable_lsp();
        editor.set_file_path("/tmp/Test.java".to_string());

        // No invalidation yet — not stale.
        assert!(!editor.lsp.slots.inlay_hints.is_stale());

        // Invalidate — now stale and needs refresh.
        editor.lsp.slots.inlay_hints.invalidate();
        assert!(editor.lsp.slots.inlay_hints.is_stale());
        assert!(editor.inlay_hints_refresh_needed());
    }

    #[test]
    fn scroll_does_not_trigger_refresh() {
        let mut editor = Editor::with_content("class Test {}\nline2\nline3\n");
        editor.enable_lsp();
        editor.set_file_path("/tmp/Test.java".to_string());

        // Not stale — scrolling shouldn't change that.
        assert!(!editor.lsp.slots.inlay_hints.is_stale());
        editor.viewport.scroll_offset = 5;
        assert!(
            !editor.inlay_hints_refresh_needed(),
            "scroll should not invalidate file-scoped hints"
        );
    }

    #[test]
    fn debounce_suppresses_rapid_refreshes() {
        let mut editor = Editor::with_content("class Test {}\n");
        editor.enable_lsp();
        editor.set_file_path("/tmp/Test.java".to_string());

        // First invalidation — needs_refresh returns true (no prior fire).
        editor.lsp.slots.inlay_hints.invalidate();
        assert!(editor.inlay_hints_refresh_needed());

        // Simulate fire (marks generation as covered + records timestamp).
        // Use TrackedSlot's internal state directly for pure state testing.
        editor.lsp.slots.inlay_hints.fired_at = editor.lsp.slots.inlay_hints.generation;
        editor.lsp.slots.inlay_hints.last_fired = Some(std::time::Instant::now());

        // Immediately invalidate again — stale, but within debounce window.
        editor.lsp.slots.inlay_hints.invalidate();
        assert!(editor.lsp.slots.inlay_hints.is_stale());
        assert!(
            !editor.lsp.slots.inlay_hints.needs_refresh(),
            "debounce should suppress immediate re-request"
        );
    }

    #[test]
    fn no_lsp_language_support_means_no_refresh() {
        let mut editor = Editor::with_content("hello world\n");
        editor.enable_lsp();
        // .txt has no LSP language ID — should not request refresh.
        editor.set_file_path("/tmp/test.txt".to_string());
        editor.lsp.slots.inlay_hints.invalidate();
        assert!(!editor.inlay_hints_refresh_needed());
    }
}
