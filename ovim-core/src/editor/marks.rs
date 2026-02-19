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
    /// File path for cross-file global marks. `None` means mark was set in an
    /// unnamed buffer and should resolve against the current buffer only.
    pub file_path: Option<String>,
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
            self.global_marks.insert(
                name,
                GlobalMark {
                    file_path: file_path.map(str::to_string),
                    line,
                    col,
                },
            );
            true
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
            result.push((*name, mark.line, mark.col, mark.file_path.clone()));
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

/// Represents a single entry in the tag stack (for Ctrl-T navigation)
/// Stores the location we jumped FROM when using gd/gD/gy
#[derive(Clone, Debug)]
pub struct TagEntry {
    /// Full path to the file
    pub file_path: String,
    /// Line number (0-indexed)
    pub line: usize,
    /// Column number (0-indexed)
    pub col: usize,
    /// Optional context (e.g., symbol name at that location)
    pub context: String,
}

impl TagEntry {
    /// Creates a new tag entry
    pub fn new(file_path: String, line: usize, col: usize) -> Self {
        Self {
            file_path,
            line,
            col,
            context: String::new(),
        }
    }

    /// Creates a new tag entry with context
    pub fn with_context(file_path: String, line: usize, col: usize, context: String) -> Self {
        Self {
            file_path,
            line,
            col,
            context,
        }
    }
}

/// Tag stack for tracking LSP-based goto locations (gd/gD/gy)
/// Unlike JumpList (bidirectional), this is a pure LIFO stack.
/// Used with Ctrl-T to navigate back to where you jumped FROM.
#[derive(Clone, Debug, Default)]
pub struct TagStack {
    /// Stack of tag entries (most recent at end)
    stack: Vec<TagEntry>,
    /// Maximum stack depth
    max_size: usize,
}

impl TagStack {
    /// Creates a new tag stack with default max size (100)
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            max_size: 100,
        }
    }

    /// Pushes a new entry onto the tag stack
    pub fn push(&mut self, entry: TagEntry) {
        self.stack.push(entry);

        // Enforce max size by removing oldest entries
        while self.stack.len() > self.max_size {
            self.stack.remove(0);
        }
    }

    /// Pops and returns the most recent tag entry
    pub fn pop(&mut self) -> Option<TagEntry> {
        self.stack.pop()
    }

    /// Returns the most recent tag entry without removing it
    pub fn peek(&self) -> Option<&TagEntry> {
        self.stack.last()
    }

    /// Returns true if the tag stack is empty
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Returns the number of entries in the tag stack
    pub fn len(&self) -> usize {
        self.stack.len()
    }

    /// Clears the tag stack
    pub fn clear(&mut self) {
        self.stack.clear();
    }

    /// Returns an iterator over the tag stack (oldest to newest)
    pub fn iter(&self) -> impl Iterator<Item = &TagEntry> {
        self.stack.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tag_stack_push_pop() {
        let mut stack = TagStack::new();
        assert!(stack.is_empty());

        stack.push(TagEntry::new("file1.rs".to_string(), 10, 5));
        stack.push(TagEntry::new("file2.rs".to_string(), 20, 10));

        assert_eq!(stack.len(), 2);
        assert!(!stack.is_empty());

        let entry = stack.pop().unwrap();
        assert_eq!(entry.file_path, "file2.rs");
        assert_eq!(entry.line, 20);
        assert_eq!(entry.col, 10);

        let entry = stack.pop().unwrap();
        assert_eq!(entry.file_path, "file1.rs");
        assert_eq!(entry.line, 10);

        assert!(stack.is_empty());
        assert!(stack.pop().is_none());
    }

    #[test]
    fn test_tag_stack_max_size() {
        let mut stack = TagStack::new();

        // Push 105 entries (max is 100)
        for i in 0..105 {
            stack.push(TagEntry::new(format!("file{}.rs", i), i, 0));
        }

        // Should only have 100 entries
        assert_eq!(stack.len(), 100);

        // First entry should be file5.rs (0-4 were dropped)
        let mut count = 0;
        for (idx, entry) in stack.iter().enumerate() {
            assert_eq!(entry.file_path, format!("file{}.rs", idx + 5));
            count += 1;
        }
        assert_eq!(count, 100);
    }

    #[test]
    fn test_tag_stack_peek() {
        let mut stack = TagStack::new();
        assert!(stack.peek().is_none());

        stack.push(TagEntry::new("test.rs".to_string(), 42, 7));

        let peeked = stack.peek().unwrap();
        assert_eq!(peeked.line, 42);
        assert_eq!(stack.len(), 1); // peek doesn't remove

        stack.pop();
        assert!(stack.peek().is_none());
    }

    #[test]
    fn test_tag_entry_with_context() {
        let entry =
            TagEntry::with_context("main.rs".to_string(), 100, 15, "fn calculate".to_string());
        assert_eq!(entry.context, "fn calculate");
    }
}
