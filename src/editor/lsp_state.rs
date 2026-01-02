use crate::lsp::LspManager;
use std::collections::HashMap;
use std::sync::Arc;

/// Per-document synchronisation state, keyed by canonical file path
#[derive(Debug, Clone, Default)]
pub struct DocumentSyncState {
    pub buffer_modified: bool,
    pub buffer_saved: bool,
    pub last_synced_content: Option<String>,
    /// Track whether we've sent didOpen for this document
    pub did_open_sent: bool,
    /// Last time we sent didChange (for debouncing)
    pub last_change_sent: Option<std::time::Instant>,
}

impl DocumentSyncState {
    pub fn mark_modified(&mut self) {
        self.buffer_modified = true;
    }

    pub fn mark_saved(&mut self) {
        self.buffer_saved = true;
    }

    pub fn is_modified(&self) -> bool {
        self.buffer_modified
    }

    pub fn should_send_change(&self) -> bool {
        // Debounce: only send if 150ms have passed since last send
        match self.last_change_sent {
            None => true,
            Some(last) => last.elapsed().as_millis() >= 150,
        }
    }

    pub fn should_send_save(&self) -> bool {
        self.buffer_saved
    }

    pub fn mark_change_sent(&mut self) {
        self.buffer_modified = false;
        self.last_change_sent = Some(std::time::Instant::now());
    }

    pub fn mark_save_sent(&mut self) {
        self.buffer_saved = false;
    }
}

/// LSP-related state for the editor
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LspResultType {
    References,
    DocumentSymbols,
    WorkspaceSymbols,
    CallHierarchy,
    TypeHierarchy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LspAction {
    GoToDefinition,
    GoToImplementation,
    GoToType,
    ShowHover,
    Completion,
    FormatDocument,
    CodeActions,
    TypeHierarchy,
    CallHierarchyIncoming,
    CallHierarchyOutgoing,
    FindReferences,
    DocumentSymbols,
    WorkspaceSymbols,
    OrganizeImports,
    Rename(String), // New name for the symbol
    SemanticTokens, // Request semantic tokens for highlighting
}

/// Container for all LSP-related state in the editor
pub struct LspState {
    /// LSP manager (optional, only if LSP is enabled)
    pub lsp_manager: Option<Arc<LspManager>>,
    /// Cached diagnostic count (errors, warnings, info, hints) for status line display
    pub diagnostic_count: (usize, usize, usize, usize),
    /// Pending LSP action to execute in async context
    pub pending_lsp_action: Option<LspAction>,
    /// Retry count for pending LSP action (max 1 retry, then give up)
    pub lsp_action_retry_count: u8,
    /// Hover information to display (from LSP)
    pub hover_info: Option<String>,
    /// Scroll offset for hover window (line number)
    pub hover_scroll: usize,
    /// Cursor position when hover was triggered (line, col) - for positioning popup
    pub hover_position: Option<(usize, usize)>,
    /// Per-document sync state (tracked by canonical file path)
    pub document_sync: HashMap<String, DocumentSyncState>,
    /// LSP status message (errors, warnings, or info)
    pub lsp_status: String,
    /// Active LSP servers (language_id -> server_name)
    pub active_lsp_servers: HashMap<String, String>,
    /// Flag to indicate LSP needs initialization for current file
    pub needs_lsp_init: bool,
    /// File path that needs didClose notification (set when switching files)
    pub pending_did_close_file: Option<String>,
    /// Available code actions at current cursor position
    pub available_code_actions: Vec<lsp_types::CodeActionOrCommand>,
    /// Available completion items at current cursor position
    pub available_completions: Vec<lsp_types::CompletionItem>,
    /// Available LSP references at current cursor position
    pub available_references: Vec<lsp_types::Location>,
    /// Available document symbols for current file
    pub available_document_symbols: Vec<lsp_types::DocumentSymbol>,
    /// Available workspace symbols
    pub available_workspace_symbols: Vec<lsp_types::SymbolInformation>,
    /// Available call hierarchy items (incoming or outgoing)
    pub available_call_hierarchy: Vec<(String, lsp_types::Location)>,
    /// Available type hierarchy items (supertypes and subtypes)
    pub available_type_hierarchy: Vec<(String, lsp_types::Location)>,
    /// Inlay hints for the visible region
    pub inlay_hints: Vec<lsp_types::InlayHint>,
    /// Currently active LSP result type (for picker navigation)
    pub active_lsp_result_type: Option<LspResultType>,
    /// Cached diagnostics for current file (for inline display)
    pub current_file_diagnostics: Vec<lsp_types::Diagnostic>,
}

impl LspState {
    /// Creates a new LspState with default values
    pub fn new() -> Self {
        Self {
            lsp_manager: None,
            diagnostic_count: (0, 0, 0, 0),
            pending_lsp_action: None,
            lsp_action_retry_count: 0,
            hover_info: None,
            hover_scroll: 0,
            hover_position: None,
            document_sync: HashMap::new(),
            lsp_status: String::new(),
            active_lsp_servers: HashMap::new(),
            needs_lsp_init: false,
            pending_did_close_file: None,
            available_code_actions: Vec::new(),
            available_completions: Vec::new(),
            available_references: Vec::new(),
            available_document_symbols: Vec::new(),
            available_workspace_symbols: Vec::new(),
            available_call_hierarchy: Vec::new(),
            available_type_hierarchy: Vec::new(),
            inlay_hints: Vec::new(),
            active_lsp_result_type: None,
            current_file_diagnostics: Vec::new(),
        }
    }
}

impl Default for LspState {
    fn default() -> Self {
        Self::new()
    }
}
