use crate::lsp::LspManager;
use std::collections::HashMap;
use std::sync::Arc;

/// Per-document synchronisation state, keyed by canonical file path
#[derive(Debug, Clone, Default)]
pub struct DocumentSyncState {
    pub buffer_modified: bool,
    pub buffer_saved: bool,
    pub last_synced_content: Option<String>,
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
}

/// Container for all LSP-related state in the editor
pub struct LspState {
    /// LSP manager (optional, only if LSP is enabled)
    pub lsp_manager: Option<Arc<LspManager>>,
    /// Cached diagnostic count (errors, warnings, info, hints) for status line display
    pub diagnostic_count: (usize, usize, usize, usize),
    /// Pending LSP action to execute in async context
    pub pending_lsp_action: Option<LspAction>,
    /// Hover information to display (from LSP)
    pub hover_info: Option<String>,
    /// Scroll offset for hover window (line number)
    pub hover_scroll: usize,
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
}

impl LspState {
    /// Creates a new LspState with default values
    pub fn new() -> Self {
        Self {
            lsp_manager: None,
            diagnostic_count: (0, 0, 0, 0),
            pending_lsp_action: None,
            hover_info: None,
            hover_scroll: 0,
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
        }
    }
}

impl Default for LspState {
    fn default() -> Self {
        Self::new()
    }
}
