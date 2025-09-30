/// Represents a cursor position in a buffer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// Line number (0-indexed)
    line: usize,
    /// Column number (0-indexed, in characters not bytes)
    col: usize,
    /// Visual column for handling tabs (used for display)
    visual_col: usize,
    /// Desired column for vertical movement (sticky column)
    desired_col: usize,
}

impl Cursor {
    /// Creates a new cursor at the specified position
    pub fn new(line: usize, col: usize) -> Self {
        Self {
            line,
            col,
            visual_col: col,
            desired_col: col,
        }
    }

    /// Gets the line number
    pub fn line(&self) -> usize {
        self.line
    }

    /// Gets the column number
    pub fn col(&self) -> usize {
        self.col
    }

    /// Gets the visual column
    pub fn visual_col(&self) -> usize {
        self.visual_col
    }

    /// Gets the desired column
    pub fn desired_col(&self) -> usize {
        self.desired_col
    }

    /// Sets the line number
    pub fn set_line(&mut self, line: usize) {
        self.line = line;
    }

    /// Sets the column number
    pub fn set_col(&mut self, col: usize) {
        self.col = col;
        self.visual_col = col;
        self.desired_col = col;
    }

    /// Sets both line and column
    pub fn set_position(&mut self, line: usize, col: usize) {
        self.line = line;
        self.col = col;
        self.visual_col = col;
        self.desired_col = col;
    }

    /// Updates the desired column (for sticky column behavior)
    pub fn update_desired_col(&mut self, col: usize) {
        self.desired_col = col;
    }

    /// Sets the visual column
    pub fn set_visual_col(&mut self, visual_col: usize) {
        self.visual_col = visual_col;
    }

    /// Moves the cursor up by n lines
    pub fn move_up(&mut self, n: usize) {
        self.line = self.line.saturating_sub(n);
    }

    /// Moves the cursor down by n lines
    pub fn move_down(&mut self, n: usize) {
        self.line = self.line.saturating_add(n);
    }

    /// Moves the cursor left by n columns
    pub fn move_left(&mut self, n: usize) {
        self.col = self.col.saturating_sub(n);
        self.visual_col = self.col;
        self.desired_col = self.col;
    }

    /// Moves the cursor right by n columns
    pub fn move_right(&mut self, n: usize) {
        self.col = self.col.saturating_add(n);
        self.visual_col = self.col;
        self.desired_col = self.col;
    }
}
