use std::path::PathBuf;
use std::time::Instant;

use tokio::sync::mpsc;

use crate::buffer::{BufferId, LineHighlights};
use crate::editor;
use crate::syntax::Language;

/// Everything [`crate::editor::Editor::tick`] keeps between calls: the
/// background-task channels, LSP startup jobs, and the cadence clocks for the
/// external-file check and the rehighlight debounce.
///
/// A frontend builds exactly one `TickState` per `Editor` and passes `&mut`
/// into every tick. Keeping the clocks here (instead of in each event loop)
/// is what makes the rehighlight and external-file policies identical for
/// the TUI, headless and GUI frontends.
///
/// `preview_rx` and `file_rx` are `pub` so a frontend's `select!` loop can
/// receive on them directly when it wants lower latency than the tick
/// cadence (the headless loop does). The tick drains them either way.
pub struct TickState {
    pub(super) preview_tx: mpsc::Sender<(String, editor::PreviewCache)>,
    pub preview_rx: mpsc::Receiver<(String, editor::PreviewCache)>,
    pub(super) file_tx: mpsc::Sender<(u64, Vec<editor::PickerResult>)>,
    pub file_rx: mpsc::Receiver<(u64, Vec<editor::PickerResult>)>,
    pub(super) syntax_tx: mpsc::Sender<(BufferId, Language, Option<LineHighlights>, u64)>,
    pub(super) syntax_rx: mpsc::Receiver<(BufferId, Language, Option<LineHighlights>, u64)>,
    pub(super) file_list_cache_tx: mpsc::Sender<(PathBuf, PathBuf, Vec<editor::PickerResult>)>,
    pub(super) file_list_cache_rx: mpsc::Receiver<(PathBuf, PathBuf, Vec<editor::PickerResult>)>,
    pub(super) lsp_startup: crate::lsp_init::LspStartup,
    pub(super) last_external_file_check: Instant,
    /// Buffer id and version seen by the previous tick; a change restarts
    /// the rehighlight debounce.
    pub(super) edit_seen: Option<(BufferId, usize)>,
    pub(super) last_edit: Instant,
    /// The user has been told about mistakes in their `languages.toml`.
    pub(super) config_warnings_shown: bool,
}

impl Default for TickState {
    fn default() -> Self {
        Self::new()
    }
}

impl TickState {
    /// Build the channel set with the capacities every frontend has used
    /// historically: 100 for preview loads, 64 file-finder batches, 16 for
    /// background syntax highlighting, and 4 for the file-list cache handoff
    /// (small because it only ever holds one pending batch).
    pub fn new() -> Self {
        let (preview_tx, preview_rx) = mpsc::channel(100);
        // Batches of files, not single files — capacity bounds memory while a
        // parallel walker streams a large repo faster than the UI drains it.
        let (file_tx, file_rx) = mpsc::channel(64);
        let (syntax_tx, syntax_rx) = mpsc::channel(16);
        let (file_list_cache_tx, file_list_cache_rx) = mpsc::channel(4);
        let now = Instant::now();
        Self {
            preview_tx,
            preview_rx,
            file_tx,
            file_rx,
            syntax_tx,
            syntax_rx,
            file_list_cache_tx,
            file_list_cache_rx,
            lsp_startup: Default::default(),
            last_external_file_check: now,
            edit_seen: None,
            last_edit: now,
            config_warnings_shown: false,
        }
    }
}
