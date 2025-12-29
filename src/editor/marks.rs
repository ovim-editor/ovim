use std::collections::HashMap;

/// Represents a position in the buffer
#[derive(Clone, Copy, Debug)]
pub struct Mark {
    pub line: usize,
    pub col: usize,
}

/// Represents a global mark (includes file path)
#[derive(Clone, Debug)]
pub struct GlobalMark {
    pub file_path: String,
    pub line: usize,
    pub col: usize,
}

/// Manages marks (a-z for buffer-local marks, A-Z for global marks)
#[derive(Clone, Debug, Default)]
pub struct MarkManager {
    /// Buffer-local marks (a-z)
    marks: HashMap<char, Mark>,
    /// Global marks (A-Z) - persist across files
    global_marks: HashMap<char, GlobalMark>,
}

impl MarkManager {
    /// Creates a new mark manager
    pub fn new() -> Self {
        Self {
            marks: HashMap::new(),
            global_marks: HashMap::new(),
        }
    }

    /// Sets a mark at the given position (buffer-local for a-z, global for A-Z)
    pub fn set_mark(
        &mut self,
        name: char,
        line: usize,
        col: usize,
        file_path: Option<&str>,
    ) -> bool {
        if name.is_ascii_lowercase() {
            self.marks.insert(name, Mark { line, col });
            true
        } else if name.is_ascii_uppercase() {
            // Global marks require a file path
            if let Some(path) = file_path {
                self.global_marks.insert(
                    name,
                    GlobalMark {
                        file_path: path.to_string(),
                        line,
                        col,
                    },
                );
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Gets a local mark by name (a-z)
    pub fn get_mark(&self, name: char) -> Option<Mark> {
        self.marks.get(&name).copied()
    }

    /// Gets a global mark by name (A-Z)
    pub fn get_global_mark(&self, name: char) -> Option<&GlobalMark> {
        if name.is_ascii_uppercase() {
            self.global_marks.get(&name)
        } else {
            None
        }
    }

    /// Clears all local marks (a-z) - global marks are preserved
    pub fn clear(&mut self) {
        self.marks.clear();
    }

    /// Clears all marks including global marks
    pub fn clear_all(&mut self) {
        self.marks.clear();
        self.global_marks.clear();
    }

    /// Returns an iterator over all local marks
    pub fn iter(&self) -> impl Iterator<Item = (char, Mark)> + '_ {
        self.marks.iter().map(|(k, v)| (*k, *v))
    }

    /// Returns an iterator over all global marks
    pub fn iter_global(&self) -> impl Iterator<Item = (char, &GlobalMark)> + '_ {
        self.global_marks.iter().map(|(k, v)| (*k, v))
    }

    /// Lists all marks as (name, line, col, file_path) tuples for display
    pub fn list_marks(&self) -> Vec<(char, usize, usize, Option<String>)> {
        let mut result = Vec::new();

        // Local marks (a-z) sorted
        let mut local: Vec<_> = self.marks.iter().collect();
        local.sort_by_key(|(k, _)| *k);
        for (name, mark) in local {
            result.push((*name, mark.line, mark.col, None));
        }

        // Global marks (A-Z) sorted
        let mut global: Vec<_> = self.global_marks.iter().collect();
        global.sort_by_key(|(k, _)| *k);
        for (name, mark) in global {
            result.push((*name, mark.line, mark.col, Some(mark.file_path.clone())));
        }

        result
    }
}

/// Manages the jump list for navigating through cursor positions
#[derive(Clone, Debug, Default)]
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
            // After draining first element, update current index
            self.current = self.jumps.len().saturating_sub(1);
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
