use crate::unicode::GraphemeCol;

/// Represents a cursor position in a buffer.
///
/// **Column semantics**: `col` is a [`GraphemeCol`] — a grapheme cluster index.
/// A grapheme cluster is what a user perceives as a single character (e.g., 👨‍👩‍👧‍👦 is
/// 1 grapheme but 7 Unicode scalar values).
///
/// When passing a cursor column to rope operations (which work in char indices),
/// convert using `grapheme_to_char_col()`. When converting rope results back to
/// cursor positions, use `char_to_grapheme_col()`. Both are in `crate::unicode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// Line number (0-indexed)
    line: usize,
    /// Column number (0-indexed, in grapheme clusters — NOT chars or bytes)
    col: usize,
    /// Visual column for handling tabs (used for display)
    visual_col: usize,
    /// Desired column for vertical movement (sticky column)
    desired_col: usize,
}

impl Cursor {
    /// Creates a new cursor at the specified position
    pub fn new(line: usize, col: GraphemeCol) -> Self {
        Self {
            line,
            col: col.0,
            visual_col: col.0,
            desired_col: col.0,
        }
    }

    /// Gets the line number
    pub fn line(&self) -> usize {
        self.line
    }

    /// Gets the column number as a [`GraphemeCol`] (0-indexed).
    ///
    /// Convert to char index via `grapheme_to_char_col()` before passing to rope operations.
    pub fn col(&self) -> GraphemeCol {
        GraphemeCol(self.col)
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
    pub fn set_col(&mut self, col: GraphemeCol) {
        self.col = col.0;
        self.visual_col = col.0;
        self.desired_col = col.0;
    }

    /// Sets the column number without updating desired_col (for vertical movement)
    pub fn set_col_preserve_desired(&mut self, col: GraphemeCol) {
        self.col = col.0;
        self.visual_col = col.0;
    }

    /// Sets both line and column
    pub fn set_position(&mut self, line: usize, col: GraphemeCol) {
        self.line = line;
        self.col = col.0;
        self.visual_col = col.0;
        self.desired_col = col.0;
    }

    /// Updates the desired column (for sticky column behavior)
    pub fn update_desired_col(&mut self, col: GraphemeCol) {
        self.desired_col = col.0;
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
