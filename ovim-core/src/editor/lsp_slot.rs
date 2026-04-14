//! Generic `Slot<T>` for tracking in-flight LSP requests.
//!
//! A `Slot<T>` holds at most one in-flight request.  `fire()` cancels any
//! previous request before storing the new one, so same-type cancellation is
//! structural — no sequence counters needed.  Different features use separate
//! slots, so they can coexist without interference.

use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// A single in-flight LSP request with its expected result type.
pub struct Slot<T> {
    inflight: Option<Inflight<T>>,
}

struct Inflight<T> {
    task: JoinHandle<()>,
    rx: oneshot::Receiver<anyhow::Result<T>>,
    started: Instant,
    #[allow(dead_code)]
    buffer_version: u64,
}

impl<T> Slot<T> {
    /// Create an empty slot.
    pub fn new() -> Self {
        Self { inflight: None }
    }

    /// Fire a new request.  If one is already in flight, abort it first.
    pub fn fire(
        &mut self,
        task: JoinHandle<()>,
        rx: oneshot::Receiver<anyhow::Result<T>>,
        buffer_version: u64,
    ) {
        if let Some(old) = self.inflight.take() {
            old.task.abort();
        }
        self.inflight = Some(Inflight {
            task,
            rx,
            started: Instant::now(),
            buffer_version,
        });
    }

    /// Non-blocking poll.  Returns `Some(result)` when the response has
    /// arrived, `None` while still waiting.  Automatically aborts requests
    /// that have been in flight longer than `timeout`.
    #[allow(dead_code)]
    pub fn poll(&mut self) -> Option<anyhow::Result<T>> {
        self.poll_with_timeout(Duration::from_secs(15))
    }

    /// Like [`poll`] but with a caller-chosen timeout.
    pub fn poll_with_timeout(&mut self, timeout: Duration) -> Option<anyhow::Result<T>> {
        let inflight = self.inflight.as_mut()?;
        match inflight.rx.try_recv() {
            Ok(result) => {
                self.inflight.take();
                Some(result)
            }
            Err(oneshot::error::TryRecvError::Empty) => {
                if inflight.started.elapsed() > timeout {
                    self.inflight.take().unwrap().task.abort();
                }
                None
            }
            Err(oneshot::error::TryRecvError::Closed) => {
                self.inflight.take();
                None
            }
        }
    }

    /// Returns `true` if a request is currently in flight.
    pub fn is_pending(&self) -> bool {
        self.inflight.is_some()
    }

    /// Abort and clear the in-flight request, if any.
    pub fn cancel(&mut self) {
        if let Some(old) = self.inflight.take() {
            old.task.abort();
        }
    }

    /// The buffer version that was current when the request was fired.
    #[allow(dead_code)]
    pub fn buffer_version(&self) -> Option<u64> {
        self.inflight.as_ref().map(|i| i.buffer_version)
    }

    /// How long the current request has been in flight.
    #[allow(dead_code)]
    pub fn elapsed(&self) -> Option<Duration> {
        self.inflight.as_ref().map(|i| i.started.elapsed())
    }
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Result types for each slot ----

/// Result of a goto-definition / goto-implementation / goto-type-definition request.
pub struct GotoLocationResult {
    pub location: Option<lsp_types::Location>,
    /// Whether to open the result in a new tab.
    pub new_tab: bool,
}

/// Result of a hover request.
pub struct HoverResult {
    /// The rendered hover text, or `None` if the server had nothing to show.
    pub hover_text: Option<String>,
}

/// Result of a completion request.
#[derive(Debug)]
pub struct CompletionResult {
    pub items: Vec<lsp_types::CompletionItem>,
    pub file_path: String,
    /// If we successfully flushed content to LSP, record the new synced content.
    pub synced_content: Option<String>,
    pub synced_lsp_version: Option<i32>,
}

/// Result of an inlay hint request.
#[derive(Debug)]
pub struct InlayHintResult {
    pub request_key: super::lsp_state::InlayHintRequestKey,
    pub buffer_version: usize,
    /// If we successfully flushed content to LSP, record the new synced content.
    pub synced_content: Option<String>,
    pub synced_lsp_version: Option<i32>,
    pub hints: Vec<lsp_types::InlayHint>,
}

/// Result of a diagnostic refresh request.
#[derive(Debug)]
pub struct DiagnosticResult {
    pub file_path: String,
    pub buffer_version: usize,
    pub lsp_version: i32,
    pub lsp_sent_version: i32,
    pub diagnostics: Vec<lsp_types::Diagnostic>,
    pub count: (usize, usize, usize, usize),
    pub deferred: bool,
}

/// Result of a format-document request.
pub struct FormatResult {
    pub edits: Vec<lsp_types::TextEdit>,
}

/// Result of a find-references request.
pub struct ReferencesResult {
    pub locations: Vec<lsp_types::Location>,
}

/// Result of a document-symbols request.
pub struct DocumentSymbolsResult {
    pub symbols: Vec<lsp_types::DocumentSymbol>,
    pub file_path: String,
}

/// Result of a workspace-symbols request.
pub struct WorkspaceSymbolsResult {
    pub symbols: Vec<lsp_types::SymbolInformation>,
}

/// Result of a code-actions request.
pub struct CodeActionsResult {
    pub actions: Vec<super::lsp_state::AvailableCodeAction>,
}

/// Result of a rename request.
pub struct RenameResult {
    pub edit: Option<lsp_types::WorkspaceEdit>,
    pub new_name: String,
}

/// Result of an organize-imports request.
pub struct OrganizeImportsResult {
    pub action: Option<super::lsp_state::AvailableCodeAction>,
}

/// Result of a call-hierarchy request (incoming or outgoing).
pub struct CallHierarchyResult {
    pub locations: Vec<lsp_types::Location>,
    pub direction: CallHierarchyDirection,
}

#[derive(Debug, Clone, Copy)]
pub enum CallHierarchyDirection {
    Incoming,
    Outgoing,
}

/// Result of a type-hierarchy request.
pub struct TypeHierarchyResult {
    pub types: Vec<(String, lsp_types::Location)>,
    pub all_locations: Vec<lsp_types::Location>,
}

/// Result of a semantic-tokens request.
pub struct SemanticTokensSlotResult {
    pub tokens: Option<lsp_types::SemanticTokens>,
    pub legend: Option<lsp_types::SemanticTokensLegend>,
}

/// All LSP request slots, grouped for easy access from the editor.
///
/// Each feature gets its own slot so different-type requests coexist.
/// Same-type requests cancel the previous one automatically via `Slot::fire()`.
#[derive(Default)]
pub struct LspSlots {
    // -- Navigation (Step 2-3) --
    pub goto_definition: Slot<GotoLocationResult>,
    pub goto_implementation: Slot<GotoLocationResult>,
    pub goto_type_definition: Slot<GotoLocationResult>,
    pub hover: Slot<HoverResult>,
    // -- Query (Step 4) --
    pub completion: Slot<CompletionResult>,
    pub inlay_hints: Slot<InlayHintResult>,
    pub diagnostics: Slot<DiagnosticResult>,
    // -- Actions (Step 5) --
    pub format: Slot<FormatResult>,
    pub references: Slot<ReferencesResult>,
    pub document_symbols: Slot<DocumentSymbolsResult>,
    pub workspace_symbols: Slot<WorkspaceSymbolsResult>,
    pub code_actions: Slot<CodeActionsResult>,
    pub rename: Slot<RenameResult>,
    pub organize_imports: Slot<OrganizeImportsResult>,
    pub call_hierarchy: Slot<CallHierarchyResult>,
    pub type_hierarchy: Slot<TypeHierarchyResult>,
    pub semantic_tokens: Slot<SemanticTokensSlotResult>,
}

impl LspSlots {
    /// Abort all in-flight requests.
    pub fn cancel_all(&mut self) {
        self.goto_definition.cancel();
        self.goto_implementation.cancel();
        self.goto_type_definition.cancel();
        self.hover.cancel();
        self.completion.cancel();
        self.inlay_hints.cancel();
        self.diagnostics.cancel();
        self.format.cancel();
        self.references.cancel();
        self.document_symbols.cancel();
        self.workspace_symbols.cancel();
        self.code_actions.cancel();
        self.rename.cancel();
        self.organize_imports.cancel();
        self.call_hierarchy.cancel();
        self.type_hierarchy.cancel();
        self.semantic_tokens.cancel();
    }

    /// Returns true if any slot has an in-flight request.
    pub fn any_pending(&self) -> bool {
        self.goto_definition.is_pending()
            || self.goto_implementation.is_pending()
            || self.goto_type_definition.is_pending()
            || self.hover.is_pending()
            || self.completion.is_pending()
            || self.inlay_hints.is_pending()
            || self.diagnostics.is_pending()
            || self.format.is_pending()
            || self.references.is_pending()
            || self.document_symbols.is_pending()
            || self.workspace_symbols.is_pending()
            || self.code_actions.is_pending()
            || self.rename.is_pending()
            || self.organize_imports.is_pending()
            || self.call_hierarchy.is_pending()
            || self.type_hierarchy.is_pending()
            || self.semantic_tokens.is_pending()
    }
}
