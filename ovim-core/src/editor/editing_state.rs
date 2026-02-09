use super::{PendingSemanticChange, ReplaceModeState};
use crate::change::ChangeToken;
use crate::repeat_action::RepeatAction;

/// Describes the delete phase of a change operator for dot-repeat.
///
/// Set before entering insert mode; consumed by `exit_insert_mode()` to
/// build a `RepeatAction::Change` that combines the semantic delete with
/// the text typed during insert mode.
pub struct PendingChangeRepeat {
    pub delete_action: RepeatAction,
    pub linewise: bool,
    /// Token for the delete-phase undo entry. None if the delete phase
    /// produced no edits (e.g., `C` at end of line, `s` on empty line).
    pub delete_token: Option<ChangeToken>,
}

/// State for active editing operations (insert, replace, substitute, rename).
pub struct EditingState {
    /// Last insert position (line, col) for gi command
    pub last_insert_position: Option<(usize, usize)>,
    /// Pending semantic change operation (for ci", cw, etc.)
    /// When Some, insert mode exit will create a semantic change instead of composite
    pub pending_semantic_change: Option<PendingSemanticChange>,
    /// Pending change repeat — describes the delete phase for dot-repeat (cc, C, s, etc.)
    /// Mutually exclusive with pending_semantic_change.
    pub pending_change_repeat: Option<PendingChangeRepeat>,
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
            pending_change_repeat: None,
            replace_mode_state: None,
            substitute_matches: Vec::new(),
            substitute_match_index: 0,
            substitute_pattern: None,
            rename_buffer: String::new(),
            rename_cursor: 0,
        }
    }
}
