use super::filetree::FileTree;
use super::path_completion::PathCompletionState;
use super::quickfix::{LocationList, QuickfixList};
use crate::dashboard::DashboardAnimation;

/// UI panel and overlay state.
///
/// Groups fields for file tree, quickfix/location lists,
/// path completion, dashboard, cat animation, and diagnostic badge.
pub struct UiPanels {
    /// File tree explorer
    pub file_tree: FileTree,
    /// Quickfix list (global error/location list)
    pub quickfix_list: QuickfixList,
    /// Location list (per-window error/location list)
    pub location_list: LocationList,
    /// Whether quickfix window is open
    pub quickfix_window_open: bool,
    /// Whether location list window is open
    pub location_window_open: bool,
    /// Path completion state for command-line mode
    pub path_completion: PathCompletionState,
    /// Dashboard menu selected index (0-5)
    pub dashboard_selected: usize,
    /// Dashboard animation state (concrete type lives in binary crate)
    pub cat_animation: Option<Box<dyn DashboardAnimation>>,
    /// Whether the diagnostic badge overlay has been dismissed (double-Escape)
    pub diagnostic_badge_dismissed: bool,
    /// Last diagnostic count when badge state was set (for detecting changes)
    pub diagnostic_badge_last_count: (usize, usize),
    /// Last time Escape was pressed in normal mode (for double-Escape detection)
    pub last_escape_time: Option<std::time::Instant>,
}

impl Default for UiPanels {
    fn default() -> Self {
        Self {
            file_tree: FileTree::new(),
            quickfix_list: QuickfixList::new(),
            location_list: LocationList::new(),
            quickfix_window_open: false,
            location_window_open: false,
            path_completion: PathCompletionState::new(),
            dashboard_selected: 0,
            cat_animation: None,
            diagnostic_badge_dismissed: false,
            diagnostic_badge_last_count: (0, 0),
            last_escape_time: None,
        }
    }
}
