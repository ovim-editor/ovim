use std::collections::HashMap;

/// Represents a position in the buffer
#[derive(Clone, Copy, Debug)]
pub struct Mark {
    pub line: usize,
    pub col: usize,
}

/// Manages marks (a-z for buffer-local marks)
#[derive(Clone, Debug)]
pub struct MarkManager {
    /// Buffer-local marks (a-z)
    marks: HashMap<char, Mark>,
}

impl MarkManager {
    /// Creates a new mark manager
    pub fn new() -> Self {
        Self {
            marks: HashMap::new(),
        }
    }

    /// Sets a mark at the given position
    pub fn set_mark(&mut self, name: char, line: usize, col: usize) -> bool {
        if name.is_ascii_lowercase() {
            self.marks.insert(name, Mark { line, col });
            true
        } else {
            false
        }
    }

    /// Gets a mark by name
    pub fn get_mark(&self, name: char) -> Option<Mark> {
        self.marks.get(&name).copied()
    }

    /// Clears all marks
    pub fn clear(&mut self) {
        self.marks.clear();
    }

    /// Returns an iterator over all marks
    pub fn iter(&self) -> impl Iterator<Item = (char, Mark)> + '_ {
        self.marks.iter().map(|(k, v)| (*k, *v))
    }
}

/// Manages the jump list for navigating through cursor positions
#[derive(Clone, Debug)]
pub struct JumpList {
    /// List of jump positions (line, col)
    jumps: Vec<(usize, usize)>,
    /// Current position in the jump list
    current: usize,
    /// Maximum size of the jump list
    max_size: usize,
}

impl JumpList {
    /// Creates a new jump list
    pub fn new() -> Self {
        Self {
            jumps: Vec::new(),
            current: 0,
            max_size: 100,
        }
    }

    /// Adds a new jump position
    pub fn add_jump(&mut self, line: usize, col: usize) {
        // If we're not at the end, truncate everything after current
        if self.current < self.jumps.len() {
            self.jumps.truncate(self.current + 1);
        }

        // Add the new jump
        self.jumps.push((line, col));

        // Limit size
        if self.jumps.len() > self.max_size {
            self.jumps.drain(0..1);
        } else {
            self.current = self.jumps.len().saturating_sub(1);
        }
    }

    /// Jumps back in the jump list (Ctrl-O)
    pub fn jump_back(&mut self) -> Option<(usize, usize)> {
        if self.current > 0 {
            self.current -= 1;
            self.jumps.get(self.current).copied()
        } else {
            None
        }
    }

    /// Jumps forward in the jump list (Ctrl-I)
    pub fn jump_forward(&mut self) -> Option<(usize, usize)> {
        if self.current + 1 < self.jumps.len() {
            self.current += 1;
            self.jumps.get(self.current).copied()
        } else {
            None
        }
    }

    /// Returns whether we can jump back
    pub fn can_jump_back(&self) -> bool {
        self.current > 0
    }

    /// Returns whether we can jump forward
    pub fn can_jump_forward(&self) -> bool {
        self.current + 1 < self.jumps.len()
    }
}
