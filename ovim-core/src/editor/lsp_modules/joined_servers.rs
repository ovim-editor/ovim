//! Opening documents on servers that joined their group late.

use super::super::Editor;
use lsp_types::Uri;
use std::time::{Duration, Instant};

/// How long to wait before offering a document to a server again after the
/// server refused it.
const OPEN_RETRY_DELAY: Duration = Duration::from_secs(2);

impl Editor {
    /// A document the group's first server opened is not known to a server
    /// that joined afterwards (a companion such as Tailwind) or that
    /// restarted while another server kept the document. Opens the buffer at
    /// `index` on exactly those servers; the others are left alone.
    pub(in crate::editor) async fn open_document_on_joined_servers(
        &mut self,
        index: usize,
        file_path: &str,
        uri: &Uri,
        language_id: &str,
    ) {
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            return;
        };
        let state = self.lsp.state.document_sync.get(file_path);
        let opened = state.is_some_and(|state| state.did_open_sent);
        let retry_ok =
            state.is_none_or(|state| state.open_retry_after.is_none_or(|at| Instant::now() >= at));
        if !opened || !retry_ok || !lsp.document_needs_open(language_id, uri) {
            return;
        }

        let content = self.buffers[index].rope().to_string();
        match lsp
            .did_open_broadcast(uri.clone(), language_id, 1, content)
            .await
        {
            Ok(()) => {
                self.lsp.slots.diagnostics.invalidate();
                self.lsp.slots.inlay_hints.invalidate();
            }
            Err(error) => {
                crate::lsp_warn!("LSP", "didOpen failed for {}: {}", file_path, error);
                if let Some(state) = self.lsp.state.document_sync.get_mut(file_path) {
                    state.open_retry_after = Some(Instant::now() + OPEN_RETRY_DELAY);
                }
            }
        }
    }
}
