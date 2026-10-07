use crate::lsp::{diagnostic_covers_line, diagnostic_range_is_valid, LspManager};
use ropey::Rope;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

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
    pub last_flushed_content: Option<Arc<str>>,
    pub last_queued_content: Option<Arc<str>>,
    pub target_lsp_version: Option<i32>,
    /// Track whether we've sent didOpen for this document
    pub did_open_sent: bool,
    /// Earliest tick at which a failed didSave may be retried. A server that
    /// is not draining stdin refuses the notification instantly, and without
    /// this the tick would retry (and log) every 16 ms while it stays wedged.
    pub save_retry_after: Option<std::time::Instant>,
    /// Earliest tick at which a failed didOpen may be retried by the
    /// background open-document sync (same wedge-avoidance as
    /// `save_retry_after`).
    pub open_retry_after: Option<std::time::Instant>,
    /// The buffer content changed without the server hearing about it (e.g.
    /// reload after an external write). The next sync MUST send a full
    /// document update: reconcile seeding and the content-equality no-op
    /// guard are both bypassed, because they assume "server text == buffer
    /// text", which is exactly what is broken here. (OV-00324)
    pub force_full_resend: bool,
    /// The document version the server last received for this buffer (the
    /// version its `TextDocumentEdit`s will carry). Tracked per buffer so
    /// edits addressed to a hidden buffer can be checked against it.
    pub flushed_lsp_version: i32,
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
            && self
                .save_retry_after
                .is_none_or(|at| std::time::Instant::now() >= at)
    }

    /// Hold off on retrying didSave for `delay`.
    pub fn defer_save_retry(&mut self, delay: std::time::Duration) {
        self.save_retry_after = Some(std::time::Instant::now() + delay);
    }

    pub fn flushed_content(&self) -> Option<&str> {
        self.last_flushed_content.as_deref()
    }

    pub fn queued_content(&self) -> Option<&str> {
        self.last_queued_content.as_deref()
    }

    pub fn mark_change_queued(&mut self, queued_content: Arc<str>, target_lsp_version: i32) {
        self.buffer_modified = true;
        self.last_queued_content = Some(queued_content);
        self.target_lsp_version = Some(target_lsp_version);
    }

    pub fn mark_change_flushed(
        &mut self,
        flushed_content: Arc<str>,
        flushed_version: i32,
        current_content: Option<&str>,
    ) {
        self.last_flushed_content = Some(flushed_content.clone());
        self.flushed_lsp_version = flushed_version;

        if self
            .target_lsp_version
            .is_some_and(|target| target <= flushed_version)
        {
            self.target_lsp_version = None;
            if self.last_queued_content.as_deref() == Some(&*flushed_content) {
                self.last_queued_content = None;
            }
        }

        self.buffer_modified = current_content.is_some_and(|current| {
            current != &*flushed_content || self.target_lsp_version.is_some()
        });
    }

    pub fn mark_save_sent(&mut self) {
        self.buffer_saved = false;
        self.save_retry_after = None;
    }
}

/// Fingerprint of the most recent viewport-scoped inlay hint request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlayHintRequestKey {
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
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

/// Rope-anchored char offsets for one cached diagnostic, computed against the
/// buffer at placement time. `line_start` is the exact anchor
/// `decorations_from_diagnostics` gives the diagnostic's EOL decoration, so
/// projecting both through the edit log keeps every consumer (squiggle,
/// gutter sign, echo, float) on the same line as the virtual text. (OV-00328)
#[derive(Debug, Clone)]
pub struct DiagnosticAnchor {
    pub line_start: usize,
    pub start: usize,
    pub end: usize,
}

/// Anchors for `current_file_diagnostics`, parallel by index.
#[derive(Debug, Clone)]
pub struct DiagnosticAnchors {
    pub anchors: Vec<DiagnosticAnchor>,
    /// Buffer version the anchors were computed against.
    pub source_version: u64,
}

/// Diagnostics projected through the edit log, grouped by their projected
/// line. Mirrors `ProjectedDecorations`: built once per render pass so the
/// per-visible-line lookups don't re-project every diagnostic. (OV-00328)
#[derive(Debug, Default, Clone)]
pub struct ProjectedDiagnostics {
    by_line: BTreeMap<usize, Vec<lsp_types::Diagnostic>>,
    /// References into `by_line`; each diagnostic's payload is stored once.
    /// A file-wide range costs one entry, independent of its line count.
    multi_line: Vec<(usize, usize)>,
}

impl ProjectedDiagnostics {
    pub(crate) fn new(mut by_line: BTreeMap<usize, Vec<lsp_types::Diagnostic>>) -> Self {
        by_line.retain(|_, diagnostics| {
            diagnostics.retain(|d| diagnostic_range_is_valid(&d.range));
            !diagnostics.is_empty()
        });
        let multi_line = by_line
            .iter()
            .flat_map(|(&line, diagnostics)| {
                diagnostics
                    .iter()
                    .enumerate()
                    .filter_map(move |(index, d)| {
                        (d.range.end.line > d.range.start.line).then_some((line, index))
                    })
            })
            .collect();
        Self {
            by_line,
            multi_line,
        }
    }

    /// Diagnostics whose projected start line equals `line`.
    pub fn for_line(&self, line: usize) -> &[lsp_types::Diagnostic] {
        self.by_line.get(&line).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Diagnostics covering this line, including spans starting earlier.
    /// Cursor feedback and underlines use coverage; gutter/EOL use `for_line`.
    pub fn covering(&self, line: usize) -> impl Iterator<Item = &lsp_types::Diagnostic> + '_ {
        self.for_line(line).iter().chain(
            self.multi_line
                .iter()
                .filter(move |(start, _)| *start != line)
                .map(|(start, index)| &self.by_line[start][*index])
                .filter(move |d| diagnostic_covers_line(d, line)),
        )
    }

    /// Owned form of [`covering`](Self::covering), for callers that outlive
    /// the snapshot (the diagnostic float builds its message from these).
    pub fn covering_line(&self, line: usize) -> Vec<lsp_types::Diagnostic> {
        self.covering(line).cloned().collect()
    }

    /// 64-bit fingerprint of the line's full diagnostic set (projected
    /// ranges and severities), for render cache invalidation: the underline
    /// squiggle is baked into cached rows and the set can change without a
    /// buffer edit (save → republish). Lines with no diagnostics always hash
    /// to the same value. (OV-00329)
    pub fn line_hash(&self, line: usize) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for diag in self.covering(line) {
            diag.range.start.line.hash(&mut hasher);
            diag.range.start.character.hash(&mut hasher);
            diag.range.end.line.hash(&mut hasher);
            diag.range.end.character.hash(&mut hasher);
            severity_rank(diag.severity).hash(&mut hasher);
        }
        hasher.finish()
    }
}

/// Distinct value per severity for hashing. Missing severity is its own
/// bucket: it renders as ERROR today, but folding it into ERROR here would
/// mask a republish that only flips between the two.
fn severity_rank(severity: Option<lsp_types::DiagnosticSeverity>) -> u8 {
    match severity {
        Some(lsp_types::DiagnosticSeverity::ERROR) => 1,
        Some(lsp_types::DiagnosticSeverity::WARNING) => 2,
        Some(lsp_types::DiagnosticSeverity::INFORMATION) => 3,
        Some(lsp_types::DiagnosticSeverity::HINT) => 4,
        None => 0,
        _ => 5,
    }
}

/// Convert an LSP position to an absolute char offset in the rope.
fn position_to_char_offset(rope: &Rope, pos: lsp_types::Position) -> usize {
    let line = pos.line as usize;
    if line >= rope.len_lines() {
        return rope.len_chars();
    }
    let line_text = crate::display::line_content(rope, line);
    let char_idx = crate::lsp::utf16_to_char_col(&line_text, pos.character);
    (rope.line_to_char(line) + char_idx).min(rope.len_chars())
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

/// Display model of one signature help popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureHelpState {
    /// Full signature label, e.g. `join(String delimiter, String... parts)`.
    pub label: String,
    /// Char range (not bytes) of the active parameter within `label`.
    pub active_param: Option<(usize, usize)>,
    /// Zero-based index of the active parameter, when known.
    pub active_param_index: Option<usize>,
    /// Which overload is shown (zero-based) and how many the server offered.
    pub signature_index: usize,
    pub signature_count: usize,
    /// Signature documentation (markdown/plain), if any.
    pub documentation: Option<String>,
    /// Documentation of the active parameter, if any.
    pub parameter_documentation: Option<String>,
    /// Cursor position `(line, grapheme col)` the popup is anchored to.
    pub anchor: (usize, usize),
}

/// LSP-related state for the editor
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LspResultType {
    References,
    WorkspaceSymbols,
    CallHierarchy,
    TypeHierarchy,
}

/// Per-feature intent flags for LSP actions.
///
/// Multiple intents can be set simultaneously (unlike the old single-slot
/// `Option<LspAction>` which lost actions when two were queued in the same
/// frame). Each flag is checked and cleared independently by
/// `dispatch_pending_intents()`.
#[derive(Default)]
pub struct LspIntents {
    pub goto_definition: bool,
    pub goto_definition_new_tab: bool,
    pub goto_implementation: bool,
    pub goto_implementation_new_tab: bool,
    pub goto_type: bool,
    pub hover: bool,
    pub folding_ranges: bool,
    pub signature_help: bool,
    /// Pending completion request and why it is being made.
    pub completion: Option<CompletionIntent>,
    /// Identifier typing waits for the typist to pause before asking.
    pub completion_due: Option<(std::time::Instant, CompletionIntent)>,
    pub format_document: bool,
    pub code_actions: bool,
    pub call_hierarchy_incoming: bool,
    pub call_hierarchy_outgoing: bool,
    pub type_hierarchy: bool,
    pub find_references: bool,
    /// Pending `workspace/symbol` query (live symbol picker).
    pub workspace_symbols: Option<String>,
    pub organize_imports: bool,
    pub rename: Option<String>,
    pub semantic_tokens: bool,
}

/// Why a completion request is pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionIntent {
    /// Explicit request (Ctrl-Space).
    Invoked,
    /// Identifier characters were typed (auto popup).
    Identifier,
    /// A non-identifier character was typed; it only triggers when a server
    /// advertises it as a trigger character.
    Typed(char),
    /// The list on screen is `isIncomplete` and the user kept typing.
    Incomplete,
}

impl LspIntents {
    /// Clear all intent flags.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// A call- or type-hierarchy item as returned by the server.
#[derive(Debug, Clone)]
pub enum HierarchyItem {
    Call(lsp_types::CallHierarchyItem),
    Type(lsp_types::TypeHierarchyItem),
}

/// What drilling into a hierarchy entry asks the server for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HierarchyExpand {
    Incoming,
    Outgoing,
    Supertypes,
    Subtypes,
}

/// One row of a hierarchy level.
#[derive(Debug, Clone)]
pub struct HierarchyEntry {
    pub item: HierarchyItem,
    pub expand: HierarchyExpand,
    /// Direction marker shown before the name (type hierarchy only).
    pub marker: &'static str,
    pub location: lsp_types::Location,
}

/// One screen of the hierarchy browser. The browser is a stack of levels:
/// drilling down pushes one, going back pops it.
#[derive(Debug, Clone)]
pub struct HierarchyLevel {
    pub title: String,
    pub entries: Vec<HierarchyEntry>,
    pub selected: usize,
}

#[derive(Debug, Clone, Default)]
pub struct HierarchyState {
    pub language_id: String,
    pub levels: Vec<HierarchyLevel>,
}

/// Container for all LSP-related state in the editor
pub struct LspState {
    /// LSP manager (optional, only if LSP is enabled)
    pub lsp_manager: Option<Arc<LspManager>>,
    /// Cached diagnostic count (errors, warnings, info, hints) for status line display
    pub diagnostic_count: (usize, usize, usize, usize),
    /// Hover information to display (from LSP)
    pub hover_info: Option<String>,
    /// Parameter-hints popup for the call being typed (insert mode).
    pub signature_help: Option<Box<SignatureHelpState>>,
    /// Nonmodal commit details opened by pointing at the blame gutter.
    pub blame_mouse_hover: bool,
    /// Scroll offset for hover window (line number)
    pub hover_scroll: usize,
    /// Horizontal scroll offset for hover window (columns)
    pub hover_h_scroll: usize,
    /// Cursor position when hover was triggered (line, col) - for positioning popup
    pub hover_position: Option<(usize, usize)>,
    /// Per-document sync state (tracked by canonical file path)
    pub document_sync: HashMap<String, DocumentSyncState>,
    /// Latest status published by the LSP subsystem.
    pub status: String,
    /// Active LSP servers (language_id -> server_name)
    pub active_lsp_servers: HashMap<String, String>,
    /// Flag to indicate LSP needs initialization for current file
    pub needs_lsp_init: bool,
    /// Documents that need a `didClose`, oldest first (a tick can close
    /// several: a workspace edit deleting two files, `:bufdo bd`).
    pub pending_did_close: Vec<String>,
    /// Buffer version the fold debounce is waiting on, and since when.
    pub fold_tracking: Option<(usize, std::time::Instant)>,
    /// `(header line, hidden line count)` of the fold markers currently shown.
    pub fold_markers: Vec<(usize, usize)>,
    /// Buffer and fold generation `fold_markers` was derived from, so a key
    /// that changed no fold skips the marker pass.
    pub fold_markers_key: Option<(crate::buffer::BufferId, u64)>,
    /// Buffers created purely to carry a workspace edit for a file the user
    /// never opened (OV-00450). Only these may be written through to disk;
    /// a buffer the user opened, even if hidden, is never persisted behind
    /// their back.
    pub workspace_edit_carriers: Vec<crate::buffer::BufferId>,
    /// File-explorer rename waiting for its `willRenameFiles` round trip.
    pub pending_file_rename: Option<(std::path::PathBuf, String)>,
    /// The rename whose `willRenameFiles` answers are being collected.
    pub(crate) file_rename_in_flight: Option<super::file_rename::RenameInFlight>,
    /// Server `showMessageRequest`s waiting their turn, the one currently
    /// offered to the user, and answers ready to be sent.
    pub queued_message_requests: std::collections::VecDeque<crate::lsp::MessageRequest>,
    pub active_message_request: Option<crate::lsp::MessageRequest>,
    pub message_replies: Vec<(crate::lsp::MessageRequest, Option<String>)>,
    /// Hierarchy browser stack while a call/type hierarchy picker is open.
    pub hierarchy: Option<HierarchyState>,
    /// Watches the workspace for changes made outside the editor (feeds
    /// `workspace/didChangeWatchedFiles`).
    pub workspace_watcher: super::workspace_watch::WorkspaceWatcher,
    /// Available code actions at current cursor position
    pub available_code_actions: Vec<AvailableCodeAction>,
    /// Available completion items at current cursor position
    pub available_completions: Vec<lsp_types::CompletionItem>,
    /// The server each completion item came from, by item identity, for
    /// `completionItem/resolve`.
    pub completion_sources: HashMap<crate::editor::completion::ItemKey, String>,
    /// `CompletionItem.command`s of accepted items, run on the next tick.
    pub pending_completion_commands: Vec<lsp_types::Command>,
    /// Available LSP references at current cursor position
    pub available_references: Vec<lsp_types::Location>,
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
    /// Line-indexed view of `current_file_diagnostics`. Values are indices
    /// into the flat Vec, so per-line lookup is O(log L) without cloning.
    /// Kept in sync via `set_current_file_diagnostics` / `clear_current_file_diagnostics`.
    diagnostics_by_line: BTreeMap<usize, Vec<usize>>,
    /// Indices into `current_file_diagnostics` for diagnostics that span more
    /// than one line. `diagnostics_by_line` keys on the START line only, so
    /// continuation lines are found through this list instead (see
    /// `ProjectedDiagnostics::multi_line` for why they aren't indexed per line).
    multi_line_diagnostics: Vec<usize>,
    /// Rope-anchored offsets for `current_file_diagnostics`, set by
    /// `anchor_current_file_diagnostics` when diagnostics are placed against a
    /// known rope/version. `None` (e.g. diagnostics stored without a rope)
    /// disables projection — lookups fall back to the raw LSP ranges.
    pub diagnostic_anchors: Option<DiagnosticAnchors>,
    /// File path when diagnostics were last cached.
    /// Prevents showing diagnostics from a previous file after save-as/path swaps.
    pub diagnostics_file_path: Option<String>,
    /// Current LSP document version for the active file.
    /// Updated in `send_lsp_changes_if_modified` and diagnostic refresh.
    pub current_file_lsp_version: i32,
    /// Last LSP document version definitely seen by the server for the active
    /// file (didOpen/didChange flushed, not merely queued locally).
    pub current_file_lsp_sent_version: i32,
    /// Cached hover result to avoid redundant LSP requests
    pub hover_cache: Option<HoverCache>,
    /// Content type for hover window (LSP hover vs diagnostic)
    pub hover_content_type: HoverContentType,
}

impl LspState {
    /// Creates a new LspState with default values
    pub fn new() -> Self {
        Self {
            lsp_manager: None,
            diagnostic_count: (0, 0, 0, 0),
            hover_info: None,
            signature_help: None,
            blame_mouse_hover: false,
            hover_scroll: 0,
            hover_h_scroll: 0,
            hover_position: None,
            document_sync: HashMap::new(),
            status: String::new(),
            active_lsp_servers: HashMap::new(),
            needs_lsp_init: false,
            pending_did_close: Vec::new(),
            fold_tracking: None,
            fold_markers: Vec::new(),
            fold_markers_key: None,
            workspace_edit_carriers: Vec::new(),
            pending_file_rename: None,
            file_rename_in_flight: None,
            hierarchy: None,
            queued_message_requests: Default::default(),
            active_message_request: None,
            message_replies: Vec::new(),
            workspace_watcher: Default::default(),
            available_code_actions: Vec::new(),
            available_completions: Vec::new(),
            completion_sources: HashMap::new(),
            pending_completion_commands: Vec::new(),
            available_references: Vec::new(),
            available_workspace_symbols: Vec::new(),
            available_call_hierarchy: Vec::new(),
            available_type_hierarchy: Vec::new(),
            inlay_hints: Vec::new(),
            active_lsp_result_type: None,
            current_file_diagnostics: Vec::new(),
            diagnostics_by_line: BTreeMap::new(),
            multi_line_diagnostics: Vec::new(),
            diagnostic_anchors: None,
            diagnostics_file_path: None,
            current_file_lsp_version: 0,
            current_file_lsp_sent_version: 0,
            hover_cache: None,
            hover_content_type: HoverContentType::default(),
        }
    }

    /// Get language IDs of currently active/running LSP servers
    pub fn running_server_languages(&self) -> Vec<String> {
        self.active_lsp_servers.keys().cloned().collect()
    }

    /// Borrow the covering set from the current diagnostic cache.
    pub fn diagnostics_covering_line(
        &self,
        line: usize,
    ) -> impl Iterator<Item = &lsp_types::Diagnostic> {
        self.diagnostics_by_line
            .get(&line)
            .into_iter()
            .flatten()
            .copied()
            .chain(
                self.multi_line_diagnostics
                    .iter()
                    .copied()
                    .filter(move |&index| {
                        let diagnostic = &self.current_file_diagnostics[index];
                        diagnostic.range.start.line as usize != line
                            && diagnostic_covers_line(diagnostic, line)
                    }),
            )
            .map(|index| &self.current_file_diagnostics[index])
    }

    /// Replace the cached diagnostics and rebuild the line index.
    pub fn set_current_file_diagnostics(&mut self, mut diagnostics: Vec<lsp_types::Diagnostic>) {
        diagnostics.retain(|d| diagnostic_range_is_valid(&d.range));
        self.diagnostics_by_line.clear();
        self.multi_line_diagnostics.clear();
        self.diagnostic_anchors = None;
        for (idx, diag) in diagnostics.iter().enumerate() {
            self.diagnostics_by_line
                .entry(diag.range.start.line as usize)
                .or_default()
                .push(idx);
            if diag.range.end.line > diag.range.start.line {
                self.multi_line_diagnostics.push(idx);
            }
        }
        self.current_file_diagnostics = diagnostics;
    }

    /// Anchor the cached diagnostics to rope char offsets so lookups can be
    /// projected through the edit log. Must be called with the same
    /// rope/version pair the diagnostics' EOL decorations are placed against
    /// (see `poll_pending_diagnostic_refresh_response`). (OV-00328)
    pub fn anchor_current_file_diagnostics(&mut self, rope: &Rope, source_version: u64) {
        let anchors = self
            .current_file_diagnostics
            .iter()
            .map(|diag| {
                let line = diag.range.start.line as usize;
                // Same line-start anchor decorations_from_diagnostics uses.
                let line_start = if line < rope.len_lines() {
                    rope.line_to_char(line)
                } else {
                    rope.len_chars()
                };
                let start = position_to_char_offset(rope, diag.range.start);
                let end = position_to_char_offset(rope, diag.range.end).max(start);
                DiagnosticAnchor {
                    line_start,
                    start,
                    end,
                }
            })
            .collect();
        self.diagnostic_anchors = Some(DiagnosticAnchors {
            anchors,
            source_version,
        });
    }

    /// Clear cached diagnostics and the line index together.
    pub fn clear_current_file_diagnostics(&mut self) {
        self.current_file_diagnostics.clear();
        self.diagnostics_by_line.clear();
        self.multi_line_diagnostics.clear();
        self.diagnostic_anchors = None;
    }
}

impl Default for LspState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostic(range: lsp_types::Range, message: &str) -> lsp_types::Diagnostic {
        lsp_types::Diagnostic {
            range,
            severity: Some(lsp_types::DiagnosticSeverity::ERROR),
            message: message.to_string(),
            ..lsp_types::Diagnostic::default()
        }
    }

    fn projected(diagnostics: Vec<lsp_types::Diagnostic>) -> ProjectedDiagnostics {
        let mut by_line: BTreeMap<usize, Vec<lsp_types::Diagnostic>> = BTreeMap::new();
        for diag in diagnostics {
            by_line
                .entry(diag.range.start.line as usize)
                .or_default()
                .push(diag);
        }
        ProjectedDiagnostics::new(by_line)
    }

    /// OV-00345: `for_line` keeps its "starts here" meaning (gutter sign, EOL
    /// message), while `covering_line` reaches the whole span (squiggle,
    /// float, echo).
    #[test]
    fn covering_line_reaches_continuation_lines_that_for_line_does_not() {
        let snapshot = projected(vec![diagnostic(
            lsp_types::Range::new(
                lsp_types::Position::new(1, 16),
                lsp_types::Position::new(4, 6),
            ),
            "incompatible types",
        )]);

        assert_eq!(snapshot.for_line(1).len(), 1);
        for line in 2..=4 {
            assert!(
                snapshot.for_line(line).is_empty(),
                "line {line} does not start the span"
            );
            assert_eq!(
                snapshot.covering_line(line).len(),
                1,
                "line {line} is inside the span"
            );
        }
        assert!(snapshot.covering_line(0).is_empty());
        assert!(snapshot.covering_line(5).is_empty());
    }

    /// The render cache keys on `line_hash`, so a continuation line whose
    /// squiggle appears must not hash the same as a clean line — otherwise
    /// cached rows keep the pre-publication look (the OV-00329 bug class).
    #[test]
    fn line_hash_distinguishes_a_covered_continuation_line_from_a_clean_one() {
        let snapshot = projected(vec![diagnostic(
            lsp_types::Range::new(
                lsp_types::Position::new(1, 0),
                lsp_types::Position::new(3, 4),
            ),
            "unreachable",
        )]);

        assert_ne!(snapshot.line_hash(2), snapshot.line_hash(9));
    }

    #[test]
    fn diagnostic_covers_line_respects_half_open_end() {
        let diag = diagnostic(
            lsp_types::Range::new(
                lsp_types::Position::new(1, 0),
                lsp_types::Position::new(3, 0),
            ),
            "unreachable",
        );
        assert!(diagnostic_covers_line(&diag, 1));
        assert!(diagnostic_covers_line(&diag, 2));
        assert!(!diagnostic_covers_line(&diag, 3));
        assert!(!diagnostic_covers_line(&diag, 0));
    }

    /// A zero-width diagnostic (start == end) still belongs to its own line.
    #[test]
    fn diagnostic_covers_line_keeps_zero_width_ranges_on_their_line() {
        let diag = diagnostic(
            lsp_types::Range::new(
                lsp_types::Position::new(2, 0),
                lsp_types::Position::new(2, 0),
            ),
            "missing semicolon",
        );
        assert!(diagnostic_covers_line(&diag, 2));
        assert!(!diagnostic_covers_line(&diag, 3));
    }
}
