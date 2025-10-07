/// Represents the different modes in the editor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Normal mode - for navigation and commands
    Normal,
    /// Insert mode - for inserting text
    Insert,
    /// Visual mode - for selecting text (character-wise)
    Visual,
    /// Visual Line mode - for selecting text (line-wise)
    VisualLine,
    /// Visual Block mode - for selecting text (block-wise)
    VisualBlock,
    /// Command mode - for entering ex commands
    Command,
    /// Search mode - for entering search patterns (/ or ?)
    Search,
    /// Replace mode - for replacing characters
    Replace,
    /// Picker mode - for fuzzy finding files/grep
    Picker,
    /// HoverWindow mode - for displaying and scrolling hover information
    HoverWindow,
}

impl Mode {
    /// Returns the display name of the mode
    pub fn display_name(&self) -> &str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual => "VISUAL",
            Mode::VisualLine => "VISUAL_LINE",
            Mode::VisualBlock => "VISUAL_BLOCK",
            Mode::Command => "COMMAND",
            Mode::Search => "SEARCH",
            Mode::Replace => "REPLACE",
            Mode::Picker => "PICKER",
            Mode::HoverWindow => "HOVER",
        }
    }

    /// Returns whether this mode is a visual mode
    pub fn is_visual(&self) -> bool {
        matches!(self, Mode::Visual | Mode::VisualLine | Mode::VisualBlock)
    }
}

impl Default for Mode {
    fn default() -> Self {
        Mode::Normal
    }
}
