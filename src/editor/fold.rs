/// Represents a text fold (collapsed region)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fold {
    /// Starting line (inclusive)
    start_line: usize,
    /// Ending line (inclusive)
    end_line: usize,
    /// Whether this fold is currently open (false = folded/hidden)
    open: bool,
}

impl Fold {
    /// Creates a new fold
    pub fn new(start_line: usize, end_line: usize) -> Self {
        Self {
            start_line,
            end_line,
            open: false, // Start collapsed
        }
    }

    /// Gets the start line
    pub fn start_line(&self) -> usize {
        self.start_line
    }

    /// Gets the end line
    pub fn end_line(&self) -> usize {
        self.end_line
    }

    /// Whether this fold is open
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens the fold
    pub fn open(&mut self) {
        self.open = true;
    }

    /// Closes the fold
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Toggles the fold state
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Whether this fold contains the given line
    pub fn contains_line(&self, line: usize) -> bool {
        line >= self.start_line && line <= self.end_line
    }

    /// Whether this fold overlaps with another fold
    pub fn overlaps(&self, other: &Fold) -> bool {
        !(self.end_line < other.start_line || self.start_line > other.end_line)
    }
}

/// Manages folds for a buffer
#[derive(Debug, Clone)]
pub struct FoldManager {
    /// All folds in the buffer (sorted by start line)
    folds: Vec<Fold>,
}

impl FoldManager {
    /// Creates a new empty fold manager
    pub fn new() -> Self {
        Self { folds: Vec::new() }
    }

    /// Creates a new fold
    pub fn create_fold(&mut self, start_line: usize, end_line: usize) {
        // Don't create invalid folds
        if start_line >= end_line {
            return;
        }

        let fold = Fold::new(start_line, end_line);

        // Remove any overlapping folds
        self.folds.retain(|f| !f.overlaps(&fold));

        // Insert the new fold in sorted order
        let insert_pos = self
            .folds
            .binary_search_by_key(&start_line, |f| f.start_line())
            .unwrap_or_else(|pos| pos);
        self.folds.insert(insert_pos, fold);
    }

    /// Opens a fold at the given line
    pub fn open_fold_at(&mut self, line: usize) {
        for fold in &mut self.folds {
            if fold.start_line() == line && !fold.is_open() {
                fold.open();
                return;
            }
        }
    }

    /// Opens all folds containing the given line recursively
    pub fn open_fold_at_recursive(&mut self, line: usize) {
        for fold in &mut self.folds {
            if fold.contains_line(line) && !fold.is_open() {
                fold.open();
            }
        }
    }

    /// Closes a fold at the given line
    pub fn close_fold_at(&mut self, line: usize) {
        for fold in &mut self.folds {
            if fold.start_line() == line && fold.is_open() {
                fold.close();
                return;
            }
        }
    }

    /// Closes all folds containing the given line recursively
    pub fn close_fold_at_recursive(&mut self, line: usize) {
        for fold in &mut self.folds {
            if fold.contains_line(line) && fold.is_open() {
                fold.close();
            }
        }
    }

    /// Toggles a fold at the given line
    pub fn toggle_fold_at(&mut self, line: usize) {
        for fold in &mut self.folds {
            if fold.start_line() == line {
                fold.toggle();
                return;
            }
        }
    }

    /// Deletes a fold at the given line
    pub fn delete_fold_at(&mut self, line: usize) {
        self.folds.retain(|f| f.start_line() != line);
    }

    /// Opens all folds
    pub fn open_all(&mut self) {
        for fold in &mut self.folds {
            fold.open();
        }
    }

    /// Closes all folds
    pub fn close_all(&mut self) {
        for fold in &mut self.folds {
            fold.close();
        }
    }

    /// Deletes all folds
    pub fn delete_all(&mut self) {
        self.folds.clear();
    }

    /// Gets all folds
    pub fn folds(&self) -> &[Fold] {
        &self.folds
    }

    /// Checks if a line is hidden by a closed fold
    pub fn is_line_hidden(&self, line: usize) -> bool {
        for fold in &self.folds {
            if !fold.is_open() && line > fold.start_line() && line <= fold.end_line() {
                return true;
            }
        }
        false
    }

    /// Gets the fold that starts at the given line
    pub fn fold_at(&self, line: usize) -> Option<&Fold> {
        self.folds.iter().find(|f| f.start_line() == line)
    }

    /// Whether there's a closed fold at the given line
    pub fn has_closed_fold_at(&self, line: usize) -> bool {
        self.folds
            .iter()
            .any(|f| f.start_line() == line && !f.is_open())
    }
}

impl Default for FoldManager {
    fn default() -> Self {
        Self::new()
    }
}
