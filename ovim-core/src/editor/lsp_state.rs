use crate::lsp::LspManager;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// Content type for hover window - distinguishes LSP hover from diagnostic popups
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HoverContentType {
    #[default]
    LspHover,
    Diagnostic,
    BlameInfo,
    AiReasoning,
}

/// Per-document synchronisation state, keyed by canonical file path.
///
/// Debouncing is handled entirely by `LspManager::ChangeDebouncer` (single
/// owner, 150 ms).  The editor side just tracks "dirty" / "sent" so it
/// forwards content to the debouncer on the next tick.
#[derive(Debug, Clone, Default)]
pub struct DocumentSyncState {
    pub buffer_modified: bool,
    pub buffer_saved: bool,
    pub last_synced_content: Option<String>,
    /// Track whether we've sent didOpen for this document
    pub did_open_sent: bool,
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

    pub fn should_send_save(&self) -> bool {
        self.buffer_saved
    }

    pub fn mark_change_sent(&mut self, synced_content: String) {
        self.buffer_modified = false;
        self.last_synced_content = Some(synced_content);
    }

    pub fn mark_save_sent(&mut self) {
        self.buffer_saved = false;
    }
}

/// Fingerprint of the most recent viewport-scoped inlay hint request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlayHintRequestKey {
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub buffer_version: usize,
    pub lsp_version: i32,
}

/// Cache for LSP hover results to avoid redundant requests
#[derive(Debug, Clone)]
pub struct HoverCache {
    pub file_path: String,
    pub line: usize,
    pub col: usize,
    pub buffer_version: usize,
    pub hover_text: String,
    pub cached_at: std::time::Instant,
}

impl HoverCache {
    const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(60);

    pub fn is_valid(
        &self,
        file_path: &str,
        line: usize,
        col: usize,
        buffer_version: usize,
    ) -> bool {
        self.file_path == file_path
            && self.line == line
            && self.col == col
            && self.buffer_version == buffer_version
            && self.cached_at.elapsed() < Self::MAX_AGE
    }

    pub fn new(
        file_path: String,
        line: usize,
        col: usize,
        buffer_version: usize,
        hover_text: String,
    ) -> Self {
        Self {
            file_path,
            line,
            col,
            buffer_version,
            hover_text,
            cached_at: std::time::Instant::now(),
        }
    }
}

/// Pending LSP request with task handle for cancellation (Phase 5)
pub struct PendingLspRequest<T> {
    pub task: tokio::task::JoinHandle<anyhow::Result<T>>,
    pub receiver: tokio::sync::oneshot::Receiver<anyhow::Result<T>>,
    pub started: std::time::Instant,
}

/// Concurrent pending LSP response slots -- each request type has its own field,
/// so hover and goto can be in-flight simultaneously without conflict.
pub struct PendingLspResponses {
    pub hover: Option<PendingLspRequest<Option<String>>>,
    /// (new_tab, request) -- collapses Definition/DefinitionNewTab into one field
    pub definition: Option<(bool, PendingLspRequest<Option<lsp_types::Location>>)>,
    /// (new_tab, request) -- collapses Implementation/ImplementationNewTab
    pub implementation: Option<(bool, PendingLspRequest<Option<lsp_types::Location>>)>,
    pub type_definition: Option<PendingLspRequest<Option<lsp_types::Location>>>,
}

impl Default for PendingLspResponses {
    fn default() -> Self {
        Self {
            hover: None,
            definition: None,
            implementation: None,
            type_definition: None,
        }
    }
}

impl PendingLspResponses {
    /// Returns true if any response slot is occupied.
    pub fn any_pending(&self) -> bool {
        self.hover.is_some()
            || self.definition.is_some()
            || self.implementation.is_some()
            || self.type_definition.is_some()
    }

    /// Abort and clear all pending response slots.
    pub fn abort_all(&mut self) {
        if let Some(old) = self.hover.take() {
            old.task.abort();
        }
        if let Some((_, old)) = self.definition.take() {
            old.task.abort();
        }
        if let Some((_, old)) = self.implementation.take() {
            old.task.abort();
        }
        if let Some(old) = self.type_definition.take() {
            old.task.abort();
        }
    }
}

/// Pending completion request (kept separate from PendingLspResponses to avoid
/// blocking other LSP actions while completions are in-flight).
pub struct PendingCompletionRequest {
    pub seq: u64,
    pub request: PendingLspRequest<CompletionTaskResult>,
}

#[derive(Debug, Clone)]
pub struct CompletionTaskResult {
    pub items: Vec<lsp_types::CompletionItem>,
    pub file_path: String,
    /// If we successfully flushed content to LSP, record the new synced content.
    pub synced_content: Option<String>,
}

/// Pending inlay hint request (tracked separately so cosmetic hint refreshes do
/// not block other LSP actions).
pub struct PendingInlayHintRequest {
    pub seq: u64,
    pub request_key: InlayHintRequestKey,
    pub request: PendingLspRequest<InlayHintTaskResult>,
}

#[derive(Debug, Clone)]
pub struct InlayHintTaskResult {
    pub request_key: InlayHintRequestKey,
    /// If we successfully flushed content to LSP, record the new synced content.
    pub synced_content: Option<String>,
    pub hints: Vec<lsp_types::InlayHint>,
}

#[derive(Debug, Clone)]
pub struct AvailableCodeAction {
    /// LSP server ID that produced this action (language ID for primary server).
    pub server_id: String,
    /// The code action payload as returned by the server.
    pub action: lsp_types::CodeActionOrCommand,
    /// Whether this action has been resolved via `codeAction/resolve`.
    pub resolved: bool,
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
    GoToDefinitionNewTab,
    GoToImplementation,
    GoToImplementationNewTab,
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
    /// Horizontal scroll offset for hover window (columns)
    pub hover_h_scroll: usize,
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
    pub available_code_actions: Vec<AvailableCodeAction>,
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
    /// Last viewport/content fingerprint used for an inlay hint request
    pub last_inlay_hint_request: Option<InlayHintRequestKey>,
    /// Timestamp of the most recent inlay hint request attempt
    pub last_inlay_hint_request_at: Option<Instant>,
    /// Viewport/content fingerprint of the inlay hints currently rendered
    pub applied_inlay_hint_request: Option<InlayHintRequestKey>,
    /// Currently active LSP result type (for picker navigation)
    pub active_lsp_result_type: Option<LspResultType>,
    /// Cached diagnostics for current file (for inline display)
    pub current_file_diagnostics: Vec<lsp_types::Diagnostic>,
    /// Buffer version when diagnostics were last cached — used to detect staleness
    pub diagnostics_buffer_version: usize,
    /// File path when diagnostics were last cached.
    /// Prevents showing diagnostics from a previous file after save-as/path swaps.
    pub diagnostics_file_path: Option<String>,
    /// LSP document version when diagnostics were last cached.
    /// Together with `diagnostics_buffer_version`, provides full provenance:
    /// diagnostics are only shown if BOTH the buffer version AND the LSP
    /// version match their current values.  (OV-00161)
    pub diagnostics_lsp_version: i32,
    /// Current LSP document version for the active file.
    /// Updated in `send_lsp_changes_if_modified` and `update_diagnostic_cache`.
    /// Compared against `diagnostics_lsp_version` in rendering guards.
    pub current_file_lsp_version: i32,
    /// Cached hover result to avoid redundant LSP requests
    pub hover_cache: Option<HoverCache>,
    /// Pending LSP responses (each request type has its own slot)
    pub pending_lsp_responses: PendingLspResponses,
    /// Pending completion request (non-blocking, high-frequency)
    pub pending_completion: Option<PendingCompletionRequest>,
    /// Monotonic completion request sequence to ignore stale responses
    pub completion_request_seq: u64,
    /// Pending inlay hint request (non-blocking, viewport-scoped)
    pub pending_inlay_hints: Option<PendingInlayHintRequest>,
    /// Monotonic inlay hint request sequence to ignore stale responses
    pub inlay_hint_request_seq: u64,
    /// Request a diagnostic cache refresh on next tick (safety net for cases where
    /// the `diagnostics_changed` flag is missed).
    pub diagnostics_refresh_requested: bool,
    /// Content type for hover window (LSP hover vs diagnostic)
    pub hover_content_type: HoverContentType,
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
            hover_h_scroll: 0,
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
            last_inlay_hint_request: None,
            last_inlay_hint_request_at: None,
            applied_inlay_hint_request: None,
            active_lsp_result_type: None,
            current_file_diagnostics: Vec::new(),
            diagnostics_buffer_version: 0,
            diagnostics_file_path: None,
            diagnostics_lsp_version: 0,
            current_file_lsp_version: 0,
            hover_cache: None,
            pending_lsp_responses: PendingLspResponses::default(),
            pending_completion: None,
            completion_request_seq: 0,
            pending_inlay_hints: None,
            inlay_hint_request_seq: 0,
            diagnostics_refresh_requested: false,
            hover_content_type: HoverContentType::default(),
        }
    }

    /// Get language IDs of currently active/running LSP servers
    pub fn running_server_languages(&self) -> Vec<String> {
        self.active_lsp_servers.keys().cloned().collect()
    }
}

impl Default for LspState {
    fn default() -> Self {
        Self::new()
    }
}
