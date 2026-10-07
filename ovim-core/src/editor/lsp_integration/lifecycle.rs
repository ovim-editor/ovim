//! LSP initialization, install consent, the active-server registry, and
//! crash-recovery supervision.

use super::*;
use crate::lsp::{uri_from_file_path, LspManager};
use std::collections::HashMap;
use std::sync::Arc;

impl Editor {
    /// Enables LSP support
    pub fn enable_lsp(&mut self) {
        self.lsp.state.lsp_manager = Some(Arc::new(LspManager::new()));
    }

    /// Gets a reference to the LSP manager
    pub fn lsp_manager(&self) -> Option<Arc<LspManager>> {
        self.lsp.state.lsp_manager.clone()
    }

    /// Close the LSP for the current file
    pub async fn close_current_file_lsp(&mut self) {
        let Some(ref lsp) = self.lsp.state.lsp_manager else {
            return;
        };

        let Some(file_path) = self.buffer().file_path().map(|p| p.to_string()) else {
            return;
        };

        let uri = match uri_from_file_path(&file_path) {
            Some(u) => u,
            None => return,
        };

        // Get language_id from file extension
        let language_id = match self.language_id_for_path(&file_path) {
            Some(id) => id,
            None => return,
        };

        // Send LSP close notification
        let file_path_string = file_path.to_string();
        let _ = lsp.did_close_broadcast(uri, &language_id).await;
        self.lsp.state.document_sync.remove(&file_path_string);
    }

    /// Check if LSP initialization is needed for the current file
    pub fn needs_lsp_init(&self) -> Option<String> {
        if self.lsp.state.needs_lsp_init {
            self.buffer().file_path().map(|s| s.to_string())
        } else {
            None
        }
    }

    /// Clear the LSP initialization flag after init is complete
    pub fn clear_lsp_init_flag(&mut self) {
        self.lsp.state.needs_lsp_init = false;
    }

    /// Request LSP initialization for the current file
    pub fn request_lsp_init(&mut self) {
        self.lsp.state.needs_lsp_init = true;
    }

    /// Check if there's a pending LSP install awaiting user consent
    pub fn has_pending_lsp_install(&self) -> bool {
        self.lsp.pending_install.is_some()
    }

    /// Get a summary of the pending LSP install for display
    pub fn pending_lsp_install_summary(&self) -> Option<(String, String, String)> {
        self.lsp.pending_install.as_ref().map(|p| {
            (
                p.language_name.clone(),
                p.server_command.clone(),
                p.method_description.clone(),
            )
        })
    }

    /// Resolve the pending LSP install consent dialog
    pub fn resolve_pending_lsp_install(&mut self, consent: super::LspInstallConsent) {
        let pending = self.lsp.pending_install.take();
        match consent {
            super::LspInstallConsent::Yes => {
                if let Some(p) = &pending {
                    self.set_lsp_status(format!("LSP: Installing {}...", p.server_command));
                }
                // The actual install is triggered by the event loop checking
                // lsp_install_approved. Store the approved info.
                self.lsp.approved_install = pending;
            }
            super::LspInstallConsent::Always => {
                self.options.lsp_auto_install = super::AutoInstallMode::Auto;
                if let Some(p) = &pending {
                    self.set_lsp_status(format!("LSP: Installing {}...", p.server_command));
                }
                self.lsp.approved_install = pending;
            }
            super::LspInstallConsent::No => {
                if let Some(p) = &pending {
                    self.set_lsp_status(format!(
                        "LSP: {} skipped. Use :set autoinstall=prompt to re-enable.",
                        p.server_command
                    ));
                }
            }
        }
    }

    /// Take the approved LSP install info (consumed by the event loop)
    pub fn take_approved_lsp_install(&mut self) -> Option<super::PendingLspInstall> {
        self.lsp.approved_install.take()
    }

    /// Register a new LSP server
    pub fn register_lsp_server(&mut self, language_id: String, server_name: String) {
        let status = format!("LSP: {server_name} ready");
        self.lsp
            .state
            .active_lsp_servers
            .insert(language_id, server_name);
        self.set_lsp_status(status);
    }

    /// Unregister an LSP server
    pub fn unregister_lsp_server(&mut self, language_id: &str) {
        self.lsp.state.active_lsp_servers.remove(language_id);
        if self.lsp.state.active_lsp_servers.is_empty() {
            let status_was_visible = self.status_message() == self.lsp_status();
            self.lsp.state.status.clear();
            if status_was_visible {
                self.clear_status_message();
            }
        }
    }

    /// Clear file-scoped LSP state while retaining the global server registry.
    pub(crate) fn clear_lsp_state(&mut self) {
        let lsp_status_was_visible =
            !self.lsp.state.status.is_empty() && self.status_message() == self.lsp_status();
        self.lsp.state.status.clear();
        if lsp_status_was_visible {
            self.clear_status_message();
        }

        self.lsp.state.diagnostic_count = (0, 0, 0, 0);
        self.lsp.state.blame_mouse_hover = false;
        self.lsp.state.hover_info = None;
        self.lsp.state.signature_help = None;
        self.lsp.state.hover_scroll = 0;
        self.lsp.state.hover_h_scroll = 0;
        self.lsp.state.hover_position = None;
        self.lsp.state.available_code_actions.clear();
        self.lsp.state.available_completions.clear();
        self.lsp.state.available_references.clear();
        self.lsp.state.available_workspace_symbols.clear();
        self.lsp.state.available_call_hierarchy.clear();
        self.lsp.state.available_type_hierarchy.clear();
        self.lsp.state.hierarchy = None;
        self.lsp.state.active_lsp_result_type = None;
        self.lsp.state.inlay_hints.clear();
        // Drop the cached diagnostic vector too — it belongs to the file we're
        // leaving. The decorations get wiped below regardless, but leaving the
        // raw vector populated is an inconsistency with how inlay_hints is
        // treated. (OV-00269)
        self.lsp.state.clear_current_file_diagnostics();
        self.lsp.slots.inlay_hints.cancel_and_invalidate();
        self.lsp.intents.clear();
        self.lsp.slots.cancel_all();
        self.lsp.state.hover_cache = None;
        self.completion_menu.hide();
        // Reset LSP version tracking (new file has its own version space)
        self.lsp.state.current_file_lsp_version = 0;
        self.lsp.state.current_file_lsp_sent_version = 0;
        self.lsp.state.diagnostics_file_path = None;
        self.decorations.clear();
    }

    /// Get active LSP servers map
    pub fn active_lsp_servers(&self) -> &HashMap<String, String> {
        &self.lsp.state.active_lsp_servers
    }

    /// Running server for the current buffer's language, if one is active.
    pub fn current_lsp_server_name(&self) -> Option<&str> {
        let language_id = self
            .buffer()
            .file_path()
            .and_then(|path| self.language_id_for_path(path))?;
        self.lsp
            .state
            .active_lsp_servers
            .get(&language_id)
            .map(String::as_str)
    }

    /// Runs crash recovery for language servers and surfaces its
    /// announcements ("crashed, restarting in 1s", "restarted", ...).
    pub async fn supervise_lsp_servers(&mut self) {
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            return;
        };
        lsp.supervise_servers().await;
        let events = lsp.take_lifecycle_events();
        if events.is_empty() {
            return;
        }
        for event in events {
            self.set_lsp_status(event);
        }
        self.mark_dirty();
    }
}
