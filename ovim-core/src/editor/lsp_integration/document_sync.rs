//! Per-document sync bookkeeping: what the server has opened, queued and
//! flushed, and how that is reconciled with the LSP manager's versions.

use super::*;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DocumentSyncRequestAction {
    Noop,
    DidOpen,
    QueueChangeAndFlush,
    FlushQueued,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DocumentSyncRequestPlan {
    pub(super) action: DocumentSyncRequestAction,
    pub(super) old_content: Option<Arc<str>>,
}

impl Editor {
    /// Marks a document as having sent didOpen notification
    /// Used by LSP pre-warming to prevent duplicate didOpen
    pub fn mark_document_opened(&mut self, file_path: &str) {
        let state = self
            .lsp
            .state
            .document_sync
            .entry(file_path.to_string())
            .or_default();
        state.did_open_sent = true;

        // Server-state-changed boundary: document is now open on the server
        // without any local buffer mutation. Invalidate derived slots so the
        // next event-loop tick requests fresh hints/diagnostics.
        self.lsp.slots.inlay_hints.invalidate();
        self.lsp.slots.diagnostics.invalidate();
    }

    /// Marks a document as opened and synced (didOpen sent with this exact content).
    pub fn mark_document_opened_with_content(&mut self, file_path: &str, content: String) {
        self.mark_document_flushed(file_path, Arc::from(content), 1);

        // Server-state-changed boundary: document is now open on the server
        // without any local buffer mutation. Invalidate derived slots so the
        // next event-loop tick requests fresh hints/diagnostics.
        self.lsp.slots.inlay_hints.invalidate();
        self.lsp.slots.diagnostics.invalidate();
    }

    pub(super) fn document_sync_request_plan(
        &self,
        file_path: &str,
        current_content: &str,
    ) -> DocumentSyncRequestPlan {
        let Some(state) = self.lsp.state.document_sync.get(file_path) else {
            return DocumentSyncRequestPlan {
                action: DocumentSyncRequestAction::DidOpen,
                old_content: None,
            };
        };

        if !state.did_open_sent {
            return DocumentSyncRequestPlan {
                action: DocumentSyncRequestAction::DidOpen,
                old_content: None,
            };
        }

        let queued_current = state.queued_content() == Some(current_content);
        if state.is_modified() && queued_current {
            return DocumentSyncRequestPlan {
                action: DocumentSyncRequestAction::FlushQueued,
                old_content: None,
            };
        }

        let flushed_current = state.flushed_content() == Some(current_content);
        if state.is_modified() || !flushed_current {
            return DocumentSyncRequestPlan {
                action: DocumentSyncRequestAction::QueueChangeAndFlush,
                old_content: state.last_flushed_content.clone(),
            };
        }

        DocumentSyncRequestPlan {
            action: DocumentSyncRequestAction::Noop,
            old_content: None,
        }
    }

    pub(super) fn mark_document_flushed(
        &mut self,
        file_path: &str,
        content: Arc<str>,
        flushed_version: i32,
    ) {
        let current_content = self
            .buffer()
            .file_path()
            .filter(|path| *path == file_path)
            .map(|_| self.buffer().rope().to_string());
        let state = self
            .lsp
            .state
            .document_sync
            .entry(file_path.to_string())
            .or_default();
        state.did_open_sent = true;
        state.mark_change_flushed(content, flushed_version, current_content.as_deref());

        // Intentionally no slot invalidation here. `mark_document_flushed`
        // has dual semantics — it's called both for "server just received
        // new content" (pre-warm, ensure_lsp_document_synced DidOpen) AND
        // for "we just recorded that an in-flight LSP result came back
        // with flushed content" (poll response success). The first case
        // needs invalidation, the second does not — putting it here would
        // re-mark a just-completed request as stale, forcing an immediate
        // re-fire of the same query. Invalidation for the first case
        // lives in `mark_document_opened` / `mark_document_opened_with_content`
        // (pre-warm) and next to `did_open_broadcast` in
        // `ensure_lsp_document_synced` (on-demand DidOpen).
    }

    fn document_sync_state_mut(&mut self) -> Option<&mut lsp_state::DocumentSyncState> {
        let file_path = self.buffer().file_path()?.to_string();
        Some(self.lsp.state.document_sync.entry(file_path).or_default())
    }

    pub(super) fn reconcile_document_sync_with_manager(
        &mut self,
        file_path: &str,
        current_content: Option<&str>,
        manager_version: i32,
        sent_version: i32,
    ) {
        if manager_version <= 0 {
            return;
        }

        self.lsp.state.current_file_lsp_version = manager_version;
        self.lsp.state.current_file_lsp_sent_version = sent_version;

        let state = self
            .lsp
            .state
            .document_sync
            .entry(file_path.to_string())
            .or_default();

        if sent_version > 0 && !state.did_open_sent {
            state.did_open_sent = true;
        }

        if sent_version > 0 && state.last_flushed_content.is_none() && !state.force_full_resend {
            let seeded_content = state
                .last_queued_content
                .clone()
                .or_else(|| current_content.map(Arc::from));
            if let Some(content) = seeded_content {
                state.mark_change_flushed(content, sent_version, current_content);
            }
        }

        if state
            .target_lsp_version
            .is_some_and(|target_version| sent_version >= target_version)
        {
            let flushed_content = state
                .last_queued_content
                .clone()
                .or_else(|| current_content.map(Arc::from));
            if let Some(content) = flushed_content {
                state.mark_change_flushed(content, sent_version, current_content);
            }
        }
    }

    pub async fn refresh_current_lsp_sync_versions(&mut self) {
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            self.lsp.state.current_file_lsp_version = 0;
            self.lsp.state.current_file_lsp_sent_version = 0;
            return;
        };

        let Some(file_path) = self.buffer().file_path().map(str::to_string) else {
            self.lsp.state.current_file_lsp_version = 0;
            self.lsp.state.current_file_lsp_sent_version = 0;
            return;
        };

        let Some(uri) = crate::lsp::uri_from_file_path(&file_path) else {
            self.lsp.state.current_file_lsp_version = 0;
            self.lsp.state.current_file_lsp_sent_version = 0;
            return;
        };

        let manager_version = lsp.get_document_version(&uri).await;
        let sent_version = lsp.get_last_sent_version(&uri).await;
        let needs_content = self
            .lsp
            .state
            .document_sync
            .get(&file_path)
            .is_some_and(|state| {
                (sent_version > 0 && state.last_flushed_content.is_none())
                    || state
                        .target_lsp_version
                        .is_some_and(|target_version| sent_version >= target_version)
            });
        let current_content = needs_content.then(|| self.buffer().rope().to_string());
        self.reconcile_document_sync_with_manager(
            &file_path,
            current_content.as_deref(),
            manager_version,
            sent_version,
        );
    }

    /// Mark buffer as modified (for LSP didChange tracking).
    ///
    /// This is the **canonical "buffer just mutated" hook**. Every path that
    /// touches buffer content (operators, insert-mode keystrokes, undo/redo,
    /// completion accept, substitute, workspace-edit apply, dot-repeat, etc.)
    /// routes through here. That makes it the natural home for invalidating
    /// derived LSP state that becomes stale whenever the buffer changes —
    /// specifically, inlay hints and diagnostics.
    ///
    /// Hoisting the invalidation here (rather than inside `send_lsp_changes_if_modified`)
    /// fixes a class of silent staleness bugs: when buffer content equals
    /// the last-flushed content (e.g. type-then-backspace, or any undo that
    /// returns the document to a flushed state), the send path takes an
    /// early return and the slot invalidation downstream was never reached.
    /// Here the invalidation is unconditional on mutation, independent of
    /// document-sync state. `TrackedSlot`'s debounce absorbs tight loops.
    pub fn mark_buffer_modified(&mut self) {
        if let Some(state) = self.document_sync_state_mut() {
            state.mark_modified();
        }
        self.lsp.slots.inlay_hints.invalidate();
        self.lsp.slots.diagnostics.invalidate();
    }

    pub fn mark_buffer_modified_force_send(&mut self) {
        if let Some(state) = self.document_sync_state_mut() {
            state.mark_modified();
            // Clear flushed content so the next sync sends a full document
            // update rather than an incremental diff.  This is critical after
            // `:e!` (reload from disk): if a prior desync corrupted
            // last_flushed_content, incremental diffs against it would produce
            // further incorrect updates.
            state.last_flushed_content = None;
            // Clearing last_flushed_content alone is not enough: the manager
            // reconcile treats "None" as "unknown, assume in sync" and re-seeds
            // it with the CURRENT buffer content, after which the no-op guard
            // in send_lsp_changes_if_modified compares equal and silently
            // drops the update — the server keeps analyzing the pre-reload
            // text and its stale diagnostics never clear. (OV-00324)
            state.force_full_resend = true;
        }
        self.lsp.slots.inlay_hints.invalidate();
        self.lsp.slots.diagnostics.invalidate();
    }

    /// Canonical hook called after a successful `didSave` broadcast.
    ///
    /// This is a server-state-changed boundary that does NOT ride through
    /// `mark_buffer_modified` (the buffer didn't mutate), so it needs its
    /// own explicit invalidation of the slot generations. Without this,
    /// servers that re-analyze on save (e.g. rust-analyzer's cargo check)
    /// produce diagnostics/hints that never get re-requested.
    pub fn on_lsp_save_sent(&mut self, file_path: &str) {
        let state = self
            .lsp
            .state
            .document_sync
            .entry(file_path.to_string())
            .or_default();
        state.mark_save_sent();
        self.lsp.slots.inlay_hints.invalidate();
        self.lsp.slots.diagnostics.invalidate();
    }

    /// Handle LSP/diagnostics state when current buffer path changes (e.g. :w newfile).
    pub fn handle_file_path_transition_after_save(
        &mut self,
        old_path: Option<String>,
        new_path: Option<String>,
    ) {
        if old_path == new_path {
            return;
        }

        if let Some(old) = old_path {
            self.lsp.state.document_sync.remove(&old);
            self.queue_lsp_did_close(old);
        }
        if let Some(newp) = &new_path {
            // The target path may already be open on the server (e.g.
            // `:w other.rs` onto a file that was previously edited): the
            // manager's didOpen claim survives, so reconcile would seed the
            // fresh sync entry with "server has the current buffer" while
            // the server actually holds the target file's OLD text. Start
            // the new entry in force_full_resend so the first sync pushes
            // the full buffer instead of assuming it's in sync (OV-00334).
            let state = self
                .lsp
                .state
                .document_sync
                .entry(newp.clone())
                .or_default();
            *state = Default::default();
            state.buffer_modified = true;
            state.force_full_resend = true;
        }

        self.lsp.state.needs_lsp_init = true;
        self.clear_and_refresh_diagnostics();
    }

    pub fn lsp_document_sync_exists(&self) -> bool {
        let Some(file_path) = self.buffer().file_path() else {
            return false;
        };
        self.lsp.state.document_sync.contains_key(file_path)
    }

    pub fn lsp_document_is_modified(&self) -> Option<bool> {
        let file_path = self.buffer().file_path()?;
        self.lsp
            .state
            .document_sync
            .get(file_path)
            .map(|s| s.is_modified())
    }

    /// Mark buffer as saved (for LSP didSave tracking)
    pub fn mark_buffer_saved(&mut self) {
        if let Some(state) = self.document_sync_state_mut() {
            state.mark_saved();
        }
    }
}
