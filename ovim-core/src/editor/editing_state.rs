use super::{PendingSemanticChange, ReplaceModeState};

/// State for active editing operations (insert, replace, substitute, rename).
pub struct EditingState {
    /// Last insert position (line, col) for gi command
    pub last_insert_position: Option<(usize, usize)>,
    /// Pending semantic change operation (for ci", cw, etc.)
    /// When Some, insert mode exit will create a semantic change instead of composite
    pub pending_semantic_change: Option<PendingSemanticChange>,
    /// Replace mode tracking for dot-repeat
    pub replace_mode_state: Option<ReplaceModeState>,
    /// Substitute confirmation state: matches to confirm (line, start_col, end_col, replacement)
    pub substitute_matches: Vec<(usize, usize, usize, String)>,
    /// Current match index for substitute confirmation
    pub substitute_match_index: usize,
    /// Regex pattern for substitute confirmation (for highlighting)
    pub substitute_pattern: Option<regex::Regex>,
    /// Rename input buffer (for LSP rename mode)
    pub rename_buffer: String,
    /// Cursor position within the rename input buffer
    pub rename_cursor: usize,
}

impl Default for EditingState {
    fn default() -> Self {
        Self {
            last_insert_position: None,
            pending_semantic_change: None,
            replace_mode_state: None,
            substitute_matches: Vec::new(),
            substitute_match_index: 0,
            substitute_pattern: None,
            rename_buffer: String::new(),
            rename_cursor: 0,
        }
    }
}
