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
    ///
    /// Async because we flush pending didChange notifications to the LSP
    /// server first, mirroring the hover/goto pattern. Without this flush
    /// the server may answer against a stale `lsp_sent_version` and the
    /// reply gets dropped by the version-mismatch guard in
    /// `poll_pending_inlay_hint_response`, producing visual stalls.
    pub async fn request_inlay_hints_refresh(&mut self) {
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            return;
        };

        let Some(file_path) = self.buffer().file_path().map(|p| p.to_string()) else {
            return;
        };

        let Some(uri) = crate::lsp::uri_from_file_path(&file_path) else {
            return;
        };

        let Some(language_id) = crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path)
        else {
            return;
        };

        // Flush any queued didChange notifications so the `lsp_sent_version`
        // we capture below matches what the server will actually answer
        // about. See ensure_lsp_document_synced() for the same pattern
        // used by hover / goto / completion.
        self.ensure_lsp_document_synced().await;

        let buffer_version = self.buffer().version();
        let start_line = 0;
        let end_line = self.buffer().line_count();
        let lsp_sent_version = self.lsp.state.current_file_lsp_sent_version;

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

    // -------------------------------------------------------------------
    // Sprint 1 — Canonical buffer-mutation hook invalidates slots.
    //
    // These tests pin the three "LSP view just went stale" signals and
    // the bonus correctness cliff. Before the fix, each of these paths
    // left slots clean even though the server's view of the world had
    // drifted from ours (or vice versa), causing inlay hints and
    // diagnostics to silently fail to refresh.
    // -------------------------------------------------------------------

    /// Signal 1 (pre-warm): after we send didOpen to the LSP server for
    /// the first time, the server now has a document we need hints and
    /// diagnostics for — without any buffer edit having occurred. The
    /// "document opened" hook must invalidate both slots.
    #[test]
    fn did_open_hook_invalidates_slots() {
        let mut editor = Editor::with_content("class Test {}\n");
        editor.enable_lsp();
        editor.set_file_path("/tmp/Test.java".to_string());
        assert!(!editor.lsp.slots.inlay_hints.is_stale());
        assert!(!editor.lsp.slots.diagnostics.is_stale());

        // Simulate the pre-warm didOpen completion path
        // (lsp_init/mod.rs:248-ish and lsp_integration.rs:1595-ish both
        // call this after a successful did_open_broadcast).
        editor.mark_document_opened_with_content("/tmp/Test.java", "class Test {}\n".to_string());

        assert!(
            editor.lsp.slots.inlay_hints.is_stale(),
            "didOpen should invalidate inlay_hints — server has a new document"
        );
        assert!(
            editor.lsp.slots.diagnostics.is_stale(),
            "didOpen should invalidate diagnostics — server has a new document"
        );
    }

    /// Signal 2 (save): after sending didSave, the server may re-analyze
    /// the document and emit new diagnostics / hints even though the
    /// buffer itself didn't change. The "save sent" hook must invalidate.
    #[test]
    fn did_save_hook_invalidates_slots() {
        let mut editor = Editor::with_content("class Test {}\n");
        editor.enable_lsp();
        editor.set_file_path("/tmp/Test.java".to_string());
        assert!(!editor.lsp.slots.inlay_hints.is_stale());
        assert!(!editor.lsp.slots.diagnostics.is_stale());

        // Simulate the didSave broadcast success path. This mirrors what
        // send_lsp_save_if_needed does after a successful broadcast:
        // the slot invalidate must happen regardless of buffer state.
        editor.on_lsp_save_sent("/tmp/Test.java");

        assert!(
            editor.lsp.slots.inlay_hints.is_stale(),
            "didSave should invalidate inlay_hints — server may re-analyze on save"
        );
        assert!(
            editor.lsp.slots.diagnostics.is_stale(),
            "didSave should invalidate diagnostics — server may re-analyze on save"
        );
    }

    /// Signal 3 (undo, content-equals-flushed cliff): when the user types
    /// a char and undoes it, buffer content equals the last-flushed
    /// content, so send_lsp_changes_if_modified takes the early-return
    /// path and never invalidates. The canonical mark_buffer_modified
    /// hook must invalidate so we don't miss this case.
    #[test]
    fn undo_invalidates_slots_even_when_content_equals_flushed() {
        let mut editor = Editor::with_content("class Test {}\n");
        editor.enable_lsp();
        editor.set_file_path("/tmp/Test.java".to_string());

        // Simulate the "already flushed" state: last_flushed_content
        // matches current buffer content, no target version pending.
        {
            let state = editor
                .lsp
                .state
                .document_sync
                .entry("/tmp/Test.java".to_string())
                .or_default();
            state.did_open_sent = true;
            state.last_flushed_content = Some(std::sync::Arc::from("class Test {}\n"));
            state.target_lsp_version = None;
            state.buffer_modified = false;
        }

        // Type a char to create an undo-able edit.
        editor.record_operation(
            |buf| {
                buf.insert_text_at(0, crate::unicode::CharCol::ZERO, "x");
            },
            None,
        );

        // Clear staleness introduced by the insertion so we can
        // isolate the undo-path signal.
        editor.lsp.slots.inlay_hints.fired_at = editor.lsp.slots.inlay_hints.generation;
        editor.lsp.slots.inlay_hints.last_fired = Some(std::time::Instant::now());
        editor.lsp.slots.diagnostics.fired_at = editor.lsp.slots.diagnostics.generation;
        editor.lsp.slots.diagnostics.last_fired = Some(std::time::Instant::now());
        assert!(!editor.lsp.slots.inlay_hints.is_stale());
        assert!(!editor.lsp.slots.diagnostics.is_stale());

        // Undo: buffer content is now back to what was flushed.
        editor.undo();

        assert!(
            editor.lsp.slots.inlay_hints.is_stale(),
            "undo must invalidate inlay_hints even when content == last_flushed"
        );
        assert!(
            editor.lsp.slots.diagnostics.is_stale(),
            "undo must invalidate diagnostics even when content == last_flushed"
        );
    }

    /// Bonus cliff: typing a char and immediately backspacing it leaves
    /// buffer content == last-flushed. Under the old design the early
    /// return in send_lsp_changes_if_modified skipped invalidation. With
    /// the canonical hook in mark_buffer_modified, the TrackedSlot's
    /// debounce absorbs the extra invalidations but the final state
    /// must still be stale.
    #[test]
    fn type_then_backspace_still_invalidates_slots() {
        let mut editor = Editor::with_content("hello\n");
        editor.enable_lsp();
        editor.set_file_path("/tmp/test.rs".to_string());

        // Pretend we've already flushed the current content to the server.
        {
            let state = editor
                .lsp
                .state
                .document_sync
                .entry("/tmp/test.rs".to_string())
                .or_default();
            state.did_open_sent = true;
            state.last_flushed_content = Some(std::sync::Arc::from("hello\n"));
            state.target_lsp_version = None;
            state.buffer_modified = false;
        }

        // Reset slots to a clean baseline.
        editor.lsp.slots.inlay_hints.fired_at = editor.lsp.slots.inlay_hints.generation;
        editor.lsp.slots.inlay_hints.last_fired = Some(std::time::Instant::now());
        editor.lsp.slots.diagnostics.fired_at = editor.lsp.slots.diagnostics.generation;
        editor.lsp.slots.diagnostics.last_fired = Some(std::time::Instant::now());
        assert!(!editor.lsp.slots.inlay_hints.is_stale());
        assert!(!editor.lsp.slots.diagnostics.is_stale());

        // Type a char ("x"), then delete it — content returns to "hello\n".
        editor.record_operation(
            |buf| {
                buf.insert_text_at(0, crate::unicode::CharCol::ZERO, "x");
            },
            None,
        );
        editor.record_operation(
            |buf| {
                buf.delete_range(
                    0,
                    crate::unicode::CharCol::ZERO,
                    0,
                    crate::unicode::CharCol(1),
                );
            },
            None,
        );

        // Content now matches last_flushed_content — the early-return
        // path would have skipped invalidation. But both slots should
        // still be stale because mark_buffer_modified fired twice.
        assert_eq!(editor.buffer().rope().to_string(), "hello\n");
        assert!(
            editor.lsp.slots.inlay_hints.is_stale(),
            "inlay_hints must be stale after type+backspace cycle"
        );
        assert!(
            editor.lsp.slots.diagnostics.is_stale(),
            "diagnostics must be stale after type+backspace cycle"
        );
    }
}
