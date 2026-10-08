use crate::change::ChangeToken;
use crate::mode::Mode;
use crate::repeat_action::BlockColumn;

/// Visual selection: (start_position, end_position, mode)
pub type VisualSelection = ((usize, usize), (usize, usize), Mode);

/// Context for visual mode state
pub struct VisualContext {
    /// Visual mode selection start (line, col)
    pub visual_start: Option<(usize, usize)>,

    /// A visual-block I / A / c waiting for its insert session to end.
    pub block_insert: Option<BlockInsert>,

    /// Last visual selection (start, end, mode) for `gv` command
    pub last_visual_selection: Option<VisualSelection>,

    /// The selection as it stood when the key being handled arrived. An
    /// operator that moves the cursor before Visual mode ends must not change
    /// what `'<` and `'>` record.
    pub key_selection: Option<VisualSelection>,

    /// True when `$` was pressed in visual block mode — means "extend each
    /// line to its own end-of-line" rather than a fixed column.
    pub visual_block_dollar: bool,
}

impl VisualContext {
    pub fn new() -> Self {
        Self {
            visual_start: None,
            block_insert: None,
            last_visual_selection: None,
            key_selection: None,
            visual_block_dollar: false,
        }
    }
}

/// A visual-block `I` / `A` / `c`: the text typed on the first block line is
/// replicated onto the others when Insert mode ends.
#[derive(Debug, Clone)]
pub struct BlockInsert {
    pub start_line: usize,
    pub end_line: usize,
    /// Left edge of the block; `I` / `A` leave the cursor there.
    pub left_col: usize,
    /// Where the text goes on each line (absolute columns).
    pub column: BlockColumn,
    /// `c`: the deleted block width and the delete's undo entry, merged
    /// with the insert into one undo step.
    pub change: Option<(usize, Option<ChangeToken>)>,
}

impl Default for VisualContext {
    fn default() -> Self {
        Self::new()
    }
}
