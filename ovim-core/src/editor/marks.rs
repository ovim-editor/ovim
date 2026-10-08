use std::collections::HashMap;

/// Represents a global mark (includes file path)
#[derive(Clone, Debug)]
pub struct GlobalMark {
    /// File path for cross-file global marks. `None` means mark was set in an
    /// unnamed buffer and should resolve against the current buffer only.
    pub file_path: Option<String>,
    pub line: usize,
    pub col: usize,
}

/// Manages the global marks (A-Z), which persist across files. The
/// buffer-local marks (a-z, `<`, `>`) live on their `Buffer`.
#[derive(Clone, Debug, Default)]
pub struct MarkManager {
    global_marks: HashMap<char, GlobalMark>,
}

impl MarkManager {
    /// Creates a new mark manager
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a global mark (A-Z) at the given position
    pub fn set_global_mark(
        &mut self,
        name: char,
        line: usize,
        col: usize,
        file_path: Option<&str>,
    ) -> bool {
        if !name.is_ascii_uppercase() {
            return false;
        }
        self.global_marks.insert(
            name,
            GlobalMark {
                file_path: file_path.map(str::to_string),
                line,
                col,
            },
        );
        true
    }

    /// Gets a global mark by name (A-Z)
    pub fn get_global_mark(&self, name: char) -> Option<&GlobalMark> {
        self.global_marks.get(&name)
    }

    /// Returns an iterator over all global marks
    pub fn iter_global(&self) -> impl Iterator<Item = (char, &GlobalMark)> + '_ {
        self.global_marks.iter().map(|(k, v)| (*k, v))
    }
}

/// One position in the jump list: a file plus a cursor position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JumpEntry {
    /// Canonical path of the buffer, `None` for a buffer without a file.
    pub file: Option<String>,
    pub line: usize,
    pub col: usize,
}

impl JumpEntry {
    pub fn new(file: Option<String>, line: usize, col: usize) -> Self {
        Self { file, line, col }
    }

    /// Vim de-duplicates on (buffer, line): the column is irrelevant.
    fn same_place(&self, other: &Self) -> bool {
        self.line == other.line
            && match (&self.file, &other.file) {
                (Some(a), Some(b)) => crate::editor::buffer_manager::paths_identify_same_file(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

/// Vim-style jump list (`Ctrl-O` / `Ctrl-I`), spanning files.
///
/// Mirrors Vim's algorithm (`setpcmark`, `cleanup_jumplist`, `movemark`):
/// a jump appends the position it leaves; the first `Ctrl-O` after a jump
/// records where you are so `Ctrl-I` can return; earlier entries for the
/// same line are dropped, keeping the most recent.
#[derive(Clone, Debug, Default)]
pub struct JumpList {
    jumps: Vec<JumpEntry>,
    /// Index of the entry the user is "at"; `== jumps.len()` when not
    /// currently navigating the list.
    current: usize,
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

    /// Records the position a jump leaves (Vim's `setpcmark`).
    pub fn add_jump(&mut self, entry: JumpEntry) {
        self.jumps.push(entry);
        if self.jumps.len() > self.max_size.max(1) {
            self.jumps.remove(0);
        }
        self.current = self.jumps.len();
    }

    /// Vim's `cleanup_jumplist`: of several entries on the same line only the
    /// last survives; `current` follows the surviving entries.
    fn cleanup(&mut self) {
        let mut kept: Vec<JumpEntry> = Vec::with_capacity(self.jumps.len());
        let mut new_current = None;
        for from in 0..self.jumps.len() {
            if self.current == from {
                new_current = Some(kept.len());
            }
            let has_later_duplicate = self.jumps[from + 1..]
                .iter()
                .any(|later| later.same_place(&self.jumps[from]));
            if !has_later_duplicate {
                kept.push(self.jumps[from].clone());
            }
        }
        self.current = new_current.unwrap_or(kept.len());
        self.jumps = kept;
    }

    /// Steps `count` entries through the list (`-1` = Ctrl-O, `+1` = Ctrl-I).
    /// `here` is the cursor position, recorded on the first step after a jump.
    fn step(&mut self, count: isize, here: JumpEntry) -> Option<JumpEntry> {
        self.cleanup();
        // Not navigating yet: a last jump that left from this very line is a phantom
        // (jumping "back" would not move), so it goes (Vim's `cleanup_jumplist`).
        if self.current == self.jumps.len()
            && self.jumps.last().is_some_and(|last| last.same_place(&here))
        {
            self.jumps.pop();
            self.current = self.jumps.len();
        }
        if self.jumps.is_empty() {
            return None;
        }
        let len = self.jumps.len() as isize;
        let idx = self.current as isize;
        if idx + count < 0 || idx + count >= len {
            return None;
        }
        let mut idx = idx;
        if self.current == self.jumps.len() {
            // First Ctrl-O after a jump: remember where we are.
            self.add_jump(here);
            idx = self.jumps.len() as isize - 1; // skip the entry just added
            if idx + count < 0 {
                return None;
            }
        }
        idx += count;
        self.current = idx as usize;
        self.jumps.get(self.current).cloned()
    }

    /// Jumps back in the jump list (Ctrl-O)
    pub fn jump_back(&mut self, here: JumpEntry) -> Option<JumpEntry> {
        self.step(-1, here)
    }

    /// Jumps forward in the jump list (Ctrl-I)
    pub fn jump_forward(&mut self, here: JumpEntry) -> Option<JumpEntry> {
        self.step(1, here)
    }

    /// Entries, oldest first (for `:jumps`-style listings and tests).
    pub fn entries(&self) -> &[JumpEntry] {
        &self.jumps
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

    fn at(file: &str, line: usize) -> JumpEntry {
        JumpEntry::new(Some(file.to_string()), line, 0)
    }

    /// Cross-checked against `nvim --headless`: a jump from A:1 to B:10 then
    /// Ctrl-O returns to A:1 and Ctrl-I comes back to B:10.
    #[test]
    fn jump_back_then_forward_round_trips_across_files() {
        let mut list = JumpList::new();
        list.add_jump(at("/a", 1)); // left A:1 to go to B:10
        assert_eq!(list.jump_back(at("/b", 10)), Some(at("/a", 1)));
        assert_eq!(list.jump_forward(at("/a", 1)), Some(at("/b", 10)));
        assert_eq!(list.jump_forward(at("/b", 10)), None);
    }

    #[test]
    fn jump_back_with_empty_list_does_nothing() {
        let mut list = JumpList::new();
        assert_eq!(list.jump_back(at("/a", 1)), None);
        assert_eq!(list.jump_forward(at("/a", 1)), None);
    }

    #[test]
    fn earlier_entries_for_the_same_line_are_dropped() {
        let mut list = JumpList::new();
        list.add_jump(at("/a", 1));
        list.add_jump(at("/a", 5));
        list.add_jump(at("/a", 1)); // same line as the first entry
        assert_eq!(list.jump_back(at("/a", 9)), Some(at("/a", 1)));
        assert_eq!(list.jump_back(at("/a", 1)), Some(at("/a", 5)));
        assert_eq!(list.jump_back(at("/a", 5)), None);
    }

    #[test]
    fn new_jump_after_navigating_appends_at_the_end() {
        let mut list = JumpList::new();
        list.add_jump(at("/a", 1));
        list.add_jump(at("/a", 2));
        assert_eq!(list.jump_back(at("/a", 3)), Some(at("/a", 2)));
        list.add_jump(at("/a", 2)); // jump away from the entry we sit on
        assert_eq!(list.jump_back(at("/a", 7)), Some(at("/a", 2)));
    }

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
