//! LSP request intents: the flags the editor sets and the dispatcher that
//! turns them into requests on the next tick.

use super::*;

impl Editor {
    /// Get a reference to the pending LSP intents.
    pub fn pending_intents(&self) -> &crate::editor::lsp_state::LspIntents {
        &self.lsp.intents
    }

    // -------------------------------------------------------------------------
    // LSP Action Requests (set per-feature intent flags)
    // -------------------------------------------------------------------------

    /// Request document format
    pub fn request_format_document(&mut self) {
        self.lsp.intents.format_document = true;
    }

    /// Request code actions at current cursor position
    pub fn request_code_actions(&mut self) {
        self.lsp.intents.code_actions = true;
    }

    /// Request call hierarchy (incoming calls) at current cursor position
    pub fn request_call_hierarchy_incoming(&mut self) {
        self.lsp.intents.call_hierarchy_incoming = true;
    }

    /// Request call hierarchy (outgoing calls) at current cursor position
    pub fn request_call_hierarchy_outgoing(&mut self) {
        self.lsp.intents.call_hierarchy_outgoing = true;
    }

    /// Request type hierarchy at current cursor position
    pub fn request_type_hierarchy(&mut self) {
        self.lsp.intents.type_hierarchy = true;
    }

    /// Request organize imports for the current document
    pub fn request_organize_imports(&mut self) {
        self.lsp.intents.organize_imports = true;
    }

    /// Request find references at current cursor position
    pub fn request_find_references(&mut self) {
        self.lsp.intents.find_references = true;
    }

    /// Request workspace symbols
    pub fn request_workspace_symbols(&mut self) {
        self.open_workspace_symbol_picker();
    }

    /// Request rename at current cursor position
    pub fn request_rename(&mut self, new_name: String) {
        self.lsp.intents.rename = Some(new_name);
    }

    /// Request semantic tokens for the current document
    pub fn request_semantic_tokens(&mut self) {
        self.lsp.intents.semantic_tokens = true;
    }

    // -------------------------------------------------------------------------
    // LSP Action Processing (process pending actions from event loop)
    // -------------------------------------------------------------------------

    /// Process pending actions that require asynchronous frontend services.
    /// Called from the event loop after synchronous input handling.
    ///
    /// Each intent is checked independently — multiple intents can fire in the
    /// same tick (unlike the old single-slot `pending_lsp_action` which lost
    /// actions when two were queued in the same frame). Each `_impl()` method
    /// fires into its own `Slot<T>` and returns immediately.
    pub async fn dispatch_pending_intents(&mut self) {
        self.dispatch_pending_browser_start().await;
        if std::mem::take(&mut self.lsp.intents.goto_definition) {
            let _ = self.goto_definition_impl().await;
        }
        if std::mem::take(&mut self.lsp.intents.goto_definition_new_tab) {
            let _ = self.goto_definition_new_tab_impl().await;
        }
        if std::mem::take(&mut self.lsp.intents.goto_implementation) {
            let _ = self.goto_implementation_impl().await;
        }
        if std::mem::take(&mut self.lsp.intents.goto_implementation_new_tab) {
            let _ = self.goto_implementation_new_tab_impl().await;
        }
        if std::mem::take(&mut self.lsp.intents.goto_type) {
            let _ = self.goto_type_impl().await;
        }
        if std::mem::take(&mut self.lsp.intents.hover) {
            let _ = self.hover_impl().await;
        }
        if std::mem::take(&mut self.lsp.intents.folding_ranges) {
            self.request_folding_ranges().await;
        }
        self.maintain_folds().await;
        if std::mem::take(&mut self.lsp.intents.signature_help) {
            let _ = self.signature_help_impl().await;
        }
        self.promote_due_completion();
        if let Some(intent) = self.lsp.intents.completion.take() {
            let _ = self.completion_impl(intent).await;
        }
        self.request_completion_resolve().await;
        self.run_pending_completion_command().await;
        if std::mem::take(&mut self.lsp.intents.format_document) {
            let _ = self.format_document_impl().await;
        }
        if std::mem::take(&mut self.lsp.intents.code_actions) {
            let _ = self.code_actions_impl().await;
        }
        if std::mem::take(&mut self.lsp.intents.type_hierarchy) {
            let _ = self.type_hierarchy_impl().await;
        }
        if std::mem::take(&mut self.lsp.intents.call_hierarchy_incoming) {
            let _ = self.call_hierarchy_impl(true).await;
        }
        if std::mem::take(&mut self.lsp.intents.call_hierarchy_outgoing) {
            let _ = self.call_hierarchy_impl(false).await;
        }
        if std::mem::take(&mut self.lsp.intents.find_references) {
            let _ = self.find_references_impl().await;
        }
        // The live symbol picker asks again whenever its query changes.
        if let Some(query) = self
            .picker_mut()
            .and_then(|picker| picker.take_symbol_query())
        {
            self.lsp.intents.workspace_symbols = Some(query);
        }
        if let Some(query) = self.lsp.intents.workspace_symbols.take() {
            let _ = self.workspace_symbols_impl(query).await;
        }
        if std::mem::take(&mut self.lsp.intents.organize_imports) {
            let _ = self.organize_imports_impl().await;
        }
        if let Some(new_name) = self.lsp.intents.rename.take() {
            let _ = self.rename_impl(new_name).await;
        }
        if std::mem::take(&mut self.lsp.intents.semantic_tokens) {
            let _ = self.semantic_tokens_impl().await;
        }
    }
}
