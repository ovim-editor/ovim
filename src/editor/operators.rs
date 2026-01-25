// TODO (Bug 5): Count of 0 handling inconsistency
// Different operators handle count=0 differently. Some treat it as count=1 (e.g., dd with 0dd),
// others ignore it. Vim's behavior varies by operator - standardizing this would require
// careful testing against Vim to match expected behavior. Low priority - users rarely use count=0.

/// Represents the different operators in Vim
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Delete,
    Change,
    Yank,
    Indent,
    Dedent,
    AutoIndent,
    Lowercase,
    Uppercase,
    ToggleCase,
    Fold,
}
