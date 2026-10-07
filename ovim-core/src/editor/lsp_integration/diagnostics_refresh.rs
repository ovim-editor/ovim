//! Keeping the diagnostics slot in step with document sync.

use super::*;

impl Editor {
    pub fn request_diagnostics_refresh(&mut self) {
        self.lsp.slots.diagnostics.invalidate();
    }

    /// Sync pending edits/saves to the LSP server, then poll and refresh
    /// diagnostics.  Colocating these operations enforces the invariant that
    /// the server always has the latest content before we check for fresh
    /// diagnostics — preventing the one-tick-behind staleness bug where
    /// diagnostics were fetched before `didChange` was sent.
    ///
    /// Returns `true` if diagnostics changed and the UI should redraw.
    pub async fn sync_lsp_and_refresh_diagnostics(&mut self) -> bool {
        // Step 1: Push pending content to the server.
        self.supervise_lsp_servers().await;
        self.process_server_messages().await;
        // Before streaming edits: a document a (re)started server has never
        // seen must be opened first, never sent a bare didChange.
        self.sync_open_documents().await;
        self.process_workspace_file_events().await;
        self.process_pending_file_rename().await;
        self.send_lsp_changes_if_modified().await;
        self.send_lsp_save_if_needed().await;

        // Step 2: Now that the server is up-to-date, process diagnostics.
        let Some(lsp_manager) = self.lsp.state.lsp_manager.clone() else {
            return false;
        };

        self.refresh_current_lsp_sync_versions().await;

        // Apply publications held during the post-edit settle window. Deferring
        // rather than discarding ensures an empty update can clear old errors.
        lsp_manager.apply_deferred_diagnostics().await;

        // Transfer cross-thread signal to the TrackedSlot's generation counter.
        // diagnostics_changed() is consumed-on-read (AtomicBool swap), but
        // invalidate() is a monotonic counter that can never be lost.
        if lsp_manager.diagnostics_changed() {
            self.lsp.slots.diagnostics.invalidate();
        }

        // Poll completed results, then fire a new request if stale.
        let changed = self.poll_pending_diagnostic_refresh_response();
        let document_sync_dirty = self
            .buffer()
            .file_path()
            .and_then(|path| self.lsp.state.document_sync.get(path))
            .is_some_and(|state| state.is_modified());
        if self.lsp.slots.diagnostics.needs_refresh() && !document_sync_dirty {
            self.spawn_diagnostic_cache_refresh();
        }
        changed
    }

    /// Invalidate cached diagnostics and request a fresh pull from the LSP server.
    pub fn clear_and_refresh_diagnostics(&mut self) {
        self.lsp.state.diagnostics_file_path = None;
        self.lsp.slots.diagnostics.invalidate();
    }

    /// Returns true if diagnostics need refreshing.
    /// Note: unlike the old consumed-on-read flag, this is non-destructive —
    /// calling it multiple times returns the same result until fire() is called.
    /// Tests that previously used this as a "consume and check" should use
    /// `lsp.slots.diagnostics.is_stale()` directly for clarity.
    pub fn take_diagnostics_refresh_request(&mut self) -> bool {
        self.lsp.slots.diagnostics.is_stale()
    }
}
