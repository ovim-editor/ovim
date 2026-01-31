use super::search::Search;
use crate::mode::Mode;

/// Visual search state - saved when entering search from visual mode
#[derive(Debug, Clone)]
pub struct VisualSearchState {
    /// Original visual anchor position (line, col) when search was initiated
    pub anchor: (usize, usize),
    /// Original visual mode type (Visual, VisualLine, VisualBlock)
    pub mode: Mode,
}

/// Search-related state for the editor
#[derive(Debug, Clone)]
pub struct SearchContext {
    /// Search buffer (for / and ? commands)
    pub search_buffer: String,
    /// Search direction: true for forward (/), false for backward (?)
    pub search_forward: bool,
    /// Current search state
    pub current_search: Option<Search>,
    /// Search start position (line, col) - saved when entering search mode, restored on ESC
    pub search_start_pos: Option<(usize, usize)>,
    /// Visual search state - saved when entering search from visual mode
    pub visual_search_state: Option<VisualSearchState>,
}

impl SearchContext {
    /// Create a new SearchContext with default values
    pub fn new() -> Self {
        Self {
            search_buffer: String::new(),
            search_forward: true,
            current_search: None,
            search_start_pos: None,
            visual_search_state: None,
        }
    }
}

impl Default for SearchContext {
    fn default() -> Self {
        Self::new()
    }
}
