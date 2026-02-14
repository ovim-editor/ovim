use crate::mode::Mode;

/// Visual selection: (start_position, end_position, mode)
pub type VisualSelection = ((usize, usize), (usize, usize), Mode);

/// Context for visual mode state
pub struct VisualContext {
    /// Visual mode selection start (line, col)
    pub visual_start: Option<(usize, usize)>,

    /// Visual block insert/append state: (start_line, end_line, col, is_append, move_to_end)
    /// - start_line: first line of the visual block
    /// - end_line: last line of the visual block
    /// - col: column position for insertion/append
    /// - is_append: true for 'A' (append), false for 'I'/'c' (insert)
    /// - move_to_end: true for I/A (cursor at end_line), false for c (cursor at start_line)
    pub visual_block_insert_state: Option<(usize, usize, usize, bool, bool)>,

    /// Last visual selection (start, end, mode) for `gv` command
    pub last_visual_selection: Option<VisualSelection>,

    /// True when `$` was pressed in visual block mode — means "extend each
    /// line to its own end-of-line" rather than a fixed column.
    pub visual_block_dollar: bool,
}

impl VisualContext {
    pub fn new() -> Self {
        Self {
            visual_start: None,
            visual_block_insert_state: None,
            last_visual_selection: None,
            visual_block_dollar: false,
        }
    }
}

impl Default for VisualContext {
    fn default() -> Self {
        Self::new()
    }
}
