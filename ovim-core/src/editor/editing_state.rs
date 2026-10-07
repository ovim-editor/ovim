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

/// Which number a literal-insert sequence (`<C-v>`) spells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiteralKind {
    /// `<C-v>065`: up to three decimal digits, at most 255.
    Decimal,
    /// `<C-v>o101`: up to three octal digits, at most 0o377.
    Octal,
    /// `<C-v>x41`, `<C-v>u20ac`, `<C-v>U0001f600`: that many hex digits at most.
    Hex(usize),
}

/// Waiting for what `<C-v>` in Insert mode should insert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingLiteral {
    /// Just `<C-v>`: the next key is inserted as is, or starts a number.
    Start,
    /// Collecting the digits of a number (none yet after `x`, `u`, `U` or `o`).
    Digits { kind: LiteralKind, digits: String },
}

/// State for active editing operations (insert, replace, substitute, rename).
#[derive(Default)]
pub struct EditingState {
    /// Last insert position (line, col) for gi command
    pub last_insert_position: Option<(usize, usize)>,
    /// Pending change repeat — describes the delete phase for dot-repeat (cc, C, s, etc.).
    pub pending_change_repeat: Option<PendingChangeRepeat>,
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
    /// Awaiting the literal for Ctrl-V in insert mode
    pub pending_literal: Option<PendingLiteral>,
    /// Awaiting one normal-mode command for Ctrl-O in insert mode
    pub insert_normal_pending: bool,
    /// The line Ctrl-O was pressed on when the insert cursor was past the end of it.
    /// The command's cursor is clamped onto the last character; coming back it is
    /// restored past the end (vim's `ins_at_eol`).
    pub insert_normal_eol_line: Option<usize>,
    /// The goal column to give that clamped cursor, so `j`/`k` onto a longer line
    /// still land past the end of the text as well.
    pub insert_normal_eol_goal: Option<usize>,
    /// Text and cursor state for LSP rename mode.
    pub rename_input: SingleLineInput,
    /// Tab stops of the snippet just expanded from a completion.
    pub snippet: Option<Box<super::SnippetSession>>,
}
