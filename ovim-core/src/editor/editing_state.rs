use super::{ReplaceModeState, SingleLineInput};
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
#[derive(Default)]
pub struct EditingState {
    /// Last insert position (line, col) for gi command
    pub last_insert_position: Option<(usize, usize)>,
    /// Pending change repeat — describes the delete phase for dot-repeat (cc, C, s, etc.).
    pub pending_change_repeat: Option<PendingChangeRepeat>,
    /// Pending visual-block change repeat payload: (line_count, width).
    /// Set for `Ctrl-V ... c ...` and consumed when insert mode exits.
    pub pending_visual_block_change_repeat: Option<(usize, usize)>,
    /// Token for the visual-block change delete-phase undo entry.
    /// Set alongside `pending_visual_block_change_repeat` for `Ctrl-V ... c ...`
    /// and redeemed during insert-mode exit merge.
    pub pending_visual_block_change_delete_token: Option<ChangeToken>,
    /// Replace mode tracking for dot-repeat
    pub replace_mode_state: Option<ReplaceModeState>,
    /// Substitute confirmation state: matches to confirm (line, start_col, end_col, replacement)
    pub substitute_matches: Vec<(usize, usize, usize, String)>,
    /// Current match index for substitute confirmation
    pub substitute_match_index: usize,
    /// Regex pattern for substitute confirmation (for highlighting)
    pub substitute_pattern: Option<regex::Regex>,
    /// Awaiting register char for Ctrl-R in insert mode
    pub pending_register_insert: bool,
    /// Awaiting one normal-mode command for Ctrl-O in insert mode
    pub insert_normal_pending: bool,
    /// Text and cursor state for LSP rename mode.
    pub rename_input: SingleLineInput,
}
