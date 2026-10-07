//! Sending document notifications to the language servers: `didOpen`,
//! `didChange`, `didSave` and `didClose`.

use super::document_sync::DocumentSyncRequestAction;
use super::*;
use crate::lsp::uri_from_file_path;
use std::sync::Arc;
use std::time::Duration;

impl Editor {
    /// Sends buffered text changes to LSP if modified.
    ///
    /// Forwards the latest buffer content to LspManager on every tick where the
    /// buffer is dirty.  Debouncing is handled entirely by LspManager's
    /// `ChangeDebouncer` (single-owner, 150 ms).  The editor side no longer
    /// adds its own 150 ms gate — that was causing a redundant double-debounce
    /// (OV-00165).
    pub async fn send_lsp_changes_if_modified(&mut self) {
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            return;
        };

        let Some(file_path) = self.buffer().file_path().map(|p| p.to_string()) else {
            return;
        };

        let uri = match uri_from_file_path(&file_path) {
            Some(u) => u,
            None => return,
        };

        let state_key = file_path.clone();
        let manager_version = lsp.get_document_version(&uri).await;
        let sent_version = lsp.get_last_sent_version(&uri).await;
        let needs_reconcile = manager_version > 0
            && self
                .lsp
                .state
                .document_sync
                .get(&state_key)
                .is_some_and(|state| {
                    (sent_version > 0 && state.last_flushed_content.is_none())
                        || state
                            .target_lsp_version
                            .is_some_and(|target_version| sent_version >= target_version)
                });

        let mut content: Option<Arc<str>> = None;
        if needs_reconcile {
            content = Some(Arc::from(self.buffer().rope().to_string()));
            self.reconcile_document_sync_with_manager(
                &state_key,
                content.as_deref(),
                manager_version,
                sent_version,
            );
        } else if manager_version > 0 {
            self.lsp.state.current_file_lsp_version = manager_version;
            self.lsp.state.current_file_lsp_sent_version = sent_version;
        }

        // Check if we need to send — only guard is didOpen + modified
        let should_send = self
            .lsp
            .state
            .document_sync
            .get(&state_key)
            .is_some_and(|state| state.did_open_sent && state.is_modified());

        if should_send {
            // Snapshot current content once for queue/no-op checks and potential send.
            let content: Arc<str> =
                content.unwrap_or_else(|| Arc::from(self.buffer().rope().to_string()));

            let force_full_resend = {
                let state = self
                    .lsp
                    .state
                    .document_sync
                    .entry(state_key.clone())
                    .or_default();
                // A forced resend bypasses both no-op guards: they assume the
                // sync-state snapshots reflect what the server has, which is
                // exactly what a reload after an external write broke.
                if !state.force_full_resend {
                    if state.target_lsp_version.is_none()
                        && state.flushed_content() == Some(&*content)
                    {
                        state.buffer_modified = false;
                        state.last_queued_content = None;
                        return;
                    }

                    if state.queued_content() == Some(&*content) {
                        return;
                    }
                }
                state.force_full_resend
            };

            // Get language_id from file extension
            let language_id = match self.language_id_for_path(&file_path) {
                Some(id) => id,
                None => return,
            };

            // Get old content for incremental sync. Under a forced resend
            // there is no trustworthy baseline — send the full document.
            let old_content = if force_full_resend {
                None
            } else {
                self.lsp
                    .state
                    .document_sync
                    .get(&state_key)
                    .and_then(|state| state.last_flushed_content.clone())
            };

            // Send the didChange notification to all servers for this language
            if lsp
                .did_change_broadcast(uri.clone(), &language_id, content.clone(), old_content)
                .await
                .is_err()
            {
                return;
            }

            // Track the queued LSP document version (bumped immediately in did_change).
            let queued_version = lsp.get_document_version(&uri).await;
            self.lsp.state.current_file_lsp_version = queued_version;
            self.lsp.state.current_file_lsp_sent_version = lsp.get_last_sent_version(&uri).await;

            // Record the newest queued snapshot; manager reconciliation will only
            // promote it to flushed once last_sent catches up.
            let state = self.lsp.state.document_sync.entry(state_key).or_default();
            state.mark_change_queued(content, queued_version);
            state.force_full_resend = false;

            // Note: slot invalidation for inlay_hints and diagnostics happens
            // upstream in the canonical `mark_buffer_modified` hook (and in
            // `mark_document_flushed` for server-state-changed boundaries).
            // The explicit invalidate that used to live here was pure
            // duplication — every path that produced modified content
            // already bumped the generation counter.
        }
    }

    /// Sends didSave notification to LSP if needed. A refused send (server
    /// not draining stdin) is retried no sooner than `SAVE_RETRY_DELAY`.
    pub async fn send_lsp_save_if_needed(&mut self) {
        const SAVE_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

        let Some(file_path) = self.buffer().file_path().map(|p| p.to_string()) else {
            return;
        };

        let uri = match uri_from_file_path(&file_path) {
            Some(u) => u,
            None => return,
        };

        let state_key = file_path.to_string();
        let mut should_send = false;

        // Check if we should send save notification
        if let Some(state) = self.lsp.state.document_sync.get(&state_key) {
            if state.should_send_save() {
                should_send = true;
            }
        }

        if should_send {
            // Ensure didOpen/didChange state for this URI before sending didSave.
            self.ensure_lsp_document_synced().await;

            // Get buffer content BEFORE we update the state
            let content = self.buffer().rope().to_string();

            // Get language_id from file extension
            let language_id = match self.language_id_for_path(&file_path) {
                Some(id) => id,
                None => return,
            };

            let Some(ref lsp) = self.lsp.state.lsp_manager else {
                return;
            };

            // Send the didSave notification to all servers for this language
            match lsp
                .did_save_broadcast(uri, &language_id, Some(content))
                .await
            {
                Ok(()) => {
                    // Mark as sent AFTER successful send; this hook also
                    // invalidates inlay_hints + diagnostics because servers
                    // may re-analyze on save (e.g. rust-analyzer's cargo
                    // check pass) and emit fresh data for unchanged buffers.
                    self.on_lsp_save_sent(&state_key);
                }
                Err(e) => {
                    crate::lsp_warn!(
                        "LSP",
                        "didSave failed for {}: {} (retrying in {:?})",
                        file_path,
                        e,
                        SAVE_RETRY_DELAY
                    );
                    if let Some(state) = self.lsp.state.document_sync.get_mut(&state_key) {
                        state.defer_save_retry(SAVE_RETRY_DELAY);
                    }
                }
            }
        }
    }

    /// Makes sure every document the user has open is known to its language
    /// server: the current buffer plus every buffer visible in another window
    /// or tab. Covers buffers opened before the server finished starting,
    /// split/tab buffers, and buffers a restarted server has never seen.
    ///
    /// The current buffer's edits are streamed by `send_lsp_changes_if_modified`;
    /// this only adds the missing `didOpen` for it, and both open and change
    /// notifications for the other visible buffers.
    pub async fn sync_open_documents(&mut self) {
        const OPEN_RETRY_DELAY: Duration = Duration::from_secs(2);

        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            return;
        };
        if lsp.active_server_languages().is_empty() {
            return;
        }

        for index in 0..self.buffers.len() {
            let is_current = index == self.current_buffer_index;
            if !is_current && !self.buffer_is_open_in_ui(index) {
                // A hidden buffer the server already has open (its text was
                // changed by a workspace edit, an autoread...) must not leave
                // the server with a stale copy: later versioned edits are
                // checked against what the server last received (OV-00475).
                // Never `didOpen` a hidden buffer here.
                let opened_on_server = self.buffers[index]
                    .file_path()
                    .and_then(|path| self.lsp.state.document_sync.get(path))
                    .is_some_and(|state| state.did_open_sent);
                if !opened_on_server {
                    continue;
                }
            }
            let buffer = &self.buffers[index];
            if super::buffer_manager::is_scratch_buffer(buffer) {
                continue;
            }
            let Some(file_path) = buffer.file_path().map(str::to_string) else {
                continue;
            };
            let Some(uri) = uri_from_file_path(&file_path) else {
                continue;
            };
            let Some(language_id) = self.language_id_for_path(&file_path) else {
                continue;
            };
            if lsp.servers_for_document_uri(&language_id, &uri).is_empty() {
                continue;
            }

            // The manager forgets a document's version when its server
            // restarts; a "sent" document it no longer tracks was never
            // opened on the replacement, so start its sync over.
            if self
                .lsp
                .state
                .document_sync
                .get(&file_path)
                .is_some_and(|state| state.did_open_sent)
                && lsp.get_document_version(&uri).await == 0
            {
                self.lsp.state.document_sync.remove(&file_path);
                self.lsp.slots.inlay_hints.invalidate();
                self.lsp.slots.diagnostics.invalidate();
            }

            self.open_document_on_joined_servers(index, &file_path, &uri, &language_id)
                .await;

            let state = self.lsp.state.document_sync.get(&file_path);
            let opened = state.is_some_and(|state| state.did_open_sent);
            if is_current {
                let retry_ok = state.is_none_or(|state| {
                    state
                        .open_retry_after
                        .is_none_or(|at| std::time::Instant::now() >= at)
                });
                if !opened && retry_ok {
                    self.ensure_lsp_document_synced().await;
                    if !self
                        .lsp
                        .state
                        .document_sync
                        .get(&file_path)
                        .is_some_and(|state| state.did_open_sent)
                    {
                        self.lsp
                            .state
                            .document_sync
                            .entry(file_path)
                            .or_default()
                            .open_retry_after = Some(std::time::Instant::now() + OPEN_RETRY_DELAY);
                    }
                }
                continue;
            }

            if !opened {
                if state.is_some_and(|state| {
                    state
                        .open_retry_after
                        .is_some_and(|at| std::time::Instant::now() < at)
                }) {
                    continue;
                }
                let content: Arc<str> = Arc::from(self.buffers[index].rope().to_string());
                match lsp
                    .did_open_broadcast(uri.clone(), &language_id, 1, content.to_string())
                    .await
                {
                    Ok(()) => {
                        let flushed_version = lsp.get_last_sent_version(&uri).await;
                        self.mark_document_flushed(&file_path, content, flushed_version);
                        self.lsp.slots.diagnostics.invalidate();
                    }
                    Err(error) => {
                        crate::lsp_warn!("LSP", "didOpen failed for {}: {}", file_path, error);
                        self.lsp
                            .state
                            .document_sync
                            .entry(file_path)
                            .or_default()
                            .open_retry_after = Some(std::time::Instant::now() + OPEN_RETRY_DELAY);
                    }
                }
                continue;
            }

            // This runs every tick for every open buffer: an unchanged one
            // must cost nothing, so its text is copied only once it is known
            // to have been modified.
            let Some(state) = state.filter(|state| state.is_modified()) else {
                continue;
            };
            let old_content = state.last_flushed_content.clone();
            let content: Arc<str> = Arc::from(self.buffers[index].rope().to_string());
            if old_content.as_deref() == Some(&*content) {
                if let Some(state) = self.lsp.state.document_sync.get_mut(&file_path) {
                    state.buffer_modified = false;
                }
                continue;
            }
            if lsp
                .did_change_broadcast(uri.clone(), &language_id, content.clone(), old_content)
                .await
                .is_err()
            {
                continue;
            }
            let queued_version = lsp.get_document_version(&uri).await;
            if let Some(state) = self.lsp.state.document_sync.get_mut(&file_path) {
                state.mark_change_queued(content, queued_version);
            }
            if let Ok(Some((text, version))) = lsp
                .flush_pending_changes_broadcast(&uri, &language_id)
                .await
            {
                self.mark_document_flushed(&file_path, Arc::from(text), version);
            }
        }
    }

    /// Ensures the LSP server has the latest document content before making a request
    ///
    /// CRITICAL FIX: When we make a hover/goto request immediately after typing,
    /// the debounced didChange (150ms) might not have been sent yet. This causes
    /// LSP to return stale results. We flush pending changes here to ensure LSP
    /// has the latest content.
    pub async fn ensure_lsp_document_synced(&mut self) -> bool {
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            return false;
        };

        let Some(file_path) = self.buffer().file_path().map(|p| p.to_string()) else {
            return false;
        };

        let uri = match uri_from_file_path(&file_path) {
            Some(u) => u,
            None => return false,
        };

        let state_key = file_path.clone();

        // Get language_id from file extension
        let language_id = match self.language_id_for_path(&file_path) {
            Some(id) => id,
            None => return false,
        };

        // Get buffer content
        let content_str = self.buffer().rope().to_string();
        let manager_version = lsp.get_document_version(&uri).await;
        let sent_version = lsp.get_last_sent_version(&uri).await;
        self.reconcile_document_sync_with_manager(
            &state_key,
            Some(&content_str),
            manager_version,
            sent_version,
        );

        let content: Arc<str> = Arc::from(content_str);
        let plan = self.document_sync_request_plan(&state_key, &content);
        match plan.action {
            DocumentSyncRequestAction::Noop => false,
            DocumentSyncRequestAction::DidOpen => {
                match lsp
                    .did_open_broadcast(uri.clone(), &language_id, 1, content.to_string())
                    .await
                {
                    Ok(_) => {
                        let flushed_version = lsp.get_last_sent_version(&uri).await;
                        self.lsp.state.current_file_lsp_version =
                            lsp.get_document_version(&uri).await;
                        self.lsp.state.current_file_lsp_sent_version = flushed_version;
                        self.mark_document_flushed(&state_key, content, flushed_version);
                        // Server just got a fresh view of the document —
                        // invalidate derived slots so hints/diagnostics
                        // get re-requested against the newly-open doc.
                        self.lsp.slots.inlay_hints.invalidate();
                        self.lsp.slots.diagnostics.invalidate();
                    }
                    Err(e) => {
                        crate::lsp_warn!(
                            "LSP",
                            "didOpen failed for {}: {} (will retry)",
                            state_key,
                            e
                        );
                    }
                }
                true
            }
            DocumentSyncRequestAction::FlushQueued => {
                // Use actual flushed content to avoid desync with LSP server.
                let flushed = lsp
                    .flush_pending_changes_broadcast(&uri, &language_id)
                    .await
                    .ok()
                    .flatten();
                self.lsp.state.current_file_lsp_version = lsp.get_document_version(&uri).await;
                self.lsp.state.current_file_lsp_sent_version =
                    lsp.get_last_sent_version(&uri).await;
                if let Some((text, ver)) = flushed {
                    self.mark_document_flushed(&state_key, Arc::from(text), ver);
                }
                // On None/Err nothing new reached the server — recording the
                // CURRENT buffer as flushed here used to claim content the
                // server never received, silently swallowing the next resend
                // (OV-00326). Manager reconciliation promotes the queued
                // snapshot once last_sent actually catches up.
                true
            }
            DocumentSyncRequestAction::QueueChangeAndFlush => {
                if lsp
                    .did_change_broadcast(
                        uri.clone(),
                        &language_id,
                        content.clone(),
                        plan.old_content,
                    )
                    .await
                    .is_err()
                {
                    return true;
                }

                let queued_version = lsp.get_document_version(&uri).await;
                {
                    let state = self
                        .lsp
                        .state
                        .document_sync
                        .entry(state_key.clone())
                        .or_default();
                    state.mark_change_queued(content.clone(), queued_version);
                }

                // Use actual flushed content to avoid desync with LSP server.
                let flushed = lsp
                    .flush_pending_changes_broadcast(&uri, &language_id)
                    .await
                    .ok()
                    .flatten();
                self.lsp.state.current_file_lsp_version = lsp.get_document_version(&uri).await;
                self.lsp.state.current_file_lsp_sent_version =
                    lsp.get_last_sent_version(&uri).await;
                if let Some((text, ver)) = flushed {
                    self.mark_document_flushed(&state_key, Arc::from(text), ver);
                }
                // On None/Err: see FlushQueued above — never record content
                // the server did not receive as flushed (OV-00326).
                true
            }
        }
    }

    /// Queues a `didClose` for `path`, sent by the next
    /// [`send_lsp_close_if_needed`](Self::send_lsp_close_if_needed).
    pub(crate) fn queue_lsp_did_close(&mut self, path: impl Into<String>) {
        let path = path.into();
        if !self.lsp.state.pending_did_close.contains(&path) {
            self.lsp.state.pending_did_close.push(path);
        }
    }

    /// Sends `didClose` for every queued document.
    pub async fn send_lsp_close_if_needed(&mut self) {
        for path in std::mem::take(&mut self.lsp.state.pending_did_close) {
            self.send_did_close(&path).await;
        }
    }

    async fn send_did_close(&mut self, file_path: &str) {
        // Switching away from a buffer only hides it: like other editors' LSP
        // clients (attach per loaded buffer, not per visible window) the
        // document stays open on the server while its buffer is loaded. That
        // keeps its diagnostics alive for the Problems view and keeps the
        // server's picture of the project current. The document is closed when
        // the buffer goes away or its path changes (save-as, rename, delete).
        let still_loaded = self
            .buffers
            .iter()
            .any(|buffer| buffer.file_path() == Some(file_path));
        if still_loaded {
            return;
        }

        let Some(ref lsp) = self.lsp.state.lsp_manager else {
            return;
        };

        let Some(uri) = uri_from_file_path(file_path) else {
            return;
        };

        // Get language_id from file extension
        let Some(language_id) = self.language_id_for_path(file_path) else {
            return;
        };

        let _ = lsp.did_close_broadcast(uri, &language_id).await;
        self.lsp.state.document_sync.remove(file_path);
    }
}
