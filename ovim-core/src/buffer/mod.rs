mod cursor;
mod encoding;
mod file_io;
mod highlighting;
mod line_ending;
mod text_ops;

pub use cursor::Cursor;
pub use encoding::FileEncoding;
pub use highlighting::LineHighlights;
pub use line_ending::LineEnding;

use crate::change::ChangeManager;
use crate::edit::Edit;
use crate::git::GitBlame;
use crate::syntax::{CodeBlockCache, SyntaxHighlighter};
use crate::GitStatus;
use ropey::Rope;
use std::path::PathBuf;

/// Represents a text buffer using a Rope data structure for efficient editing
pub struct Buffer {
    /// The rope data structure holding the text content
    pub(super) rope: Rope,
    /// The current cursor position
    pub(super) cursor: Cursor,
    /// Whether the buffer has been modified since last save
    pub(super) modified: bool,
    /// Optional file path for this buffer
    pub(super) file_path: Option<String>,
    /// Line ending style for this buffer (LF or CRLF)
    pub(super) line_ending: LineEnding,
    /// File encoding for this buffer
    pub(super) encoding: FileEncoding,
    /// Optional syntax highlighter
    pub(super) syntax: Option<SyntaxHighlighter>,
    /// Cached syntax highlights per line (line_idx -> Vec<(range, group)>)
    pub(super) cached_highlights: Option<LineHighlights>,
    /// Version counter for highlight cache (incremented on every edit)
    pub(super) highlight_version: u64,
    /// Whether re-highlighting is pending
    pub(super) pending_rehighlight: bool,
    /// Fold manager for code folding
    pub(super) fold_manager: crate::fold::FoldManager,
    /// Git status for this buffer
    pub(super) git_status: GitStatus,
    /// Git blame data (loaded on demand via :set blame)
    pub(super) git_blame: Option<GitBlame>,
    /// Change manager for undo/redo (per-buffer)
    pub(super) change_manager: ChangeManager,
    /// Last known file modification time (for external change detection)
    pub(super) file_mtime: Option<std::time::SystemTime>,
    /// Whether the file is read-only (no write permission)
    pub(super) read_only: bool,
    /// Cached semantic token highlights from LSP (line_idx -> Vec<(range, group)>)
    /// These take precedence over tree-sitter highlights when available
    pub(super) semantic_highlights: Option<LineHighlights>,
    /// Monotonically increasing version number, incremented on every edit
    /// Used for cache invalidation in LSP hover, completion, etc.
    pub(super) version: usize,
    /// Code block cache for markdown files (language-specific highlighting inside fenced code blocks)
    pub(super) code_block_cache: Option<CodeBlockCache>,
    /// When Some, insert_text_at/delete_range append Edit records here.
    /// Used by `record()` to capture buffer mutations.
    recording: Option<Vec<Edit>>,
}

impl Buffer {
    /// Creates a new empty buffer
    pub fn new() -> Self {
        Self {
            rope: Rope::new(),
            cursor: Cursor::new(0, 0),
            modified: false,
            file_path: None,
            line_ending: LineEnding::default(),
            encoding: FileEncoding::default(),
            syntax: None,
            cached_highlights: None,
            highlight_version: 0,
            pending_rehighlight: false,
            fold_manager: crate::fold::FoldManager::new(),
            git_status: GitStatus::new(),
            git_blame: None,
            change_manager: ChangeManager::new(),
            file_mtime: None,
            read_only: false,
            semantic_highlights: None,
            version: 0,
            code_block_cache: None,
            recording: None,
        }
    }

    /// Creates a buffer from a string
    ///
    /// # Behavior
    /// - Empty string → empty rope (0 chars, 1 empty line)
    /// - Content without trailing newline → content + "\n" added
    /// - Content with trailing newline → content unchanged
    ///
    /// # Examples
    /// ```
    /// use ovim_core::buffer::Buffer;
    ///
    /// // Empty buffer has 0 chars, 1 empty line
    /// let buf = Buffer::new_from_str("");
    /// assert_eq!(buf.rope().len_chars(), 0);
    /// assert_eq!(buf.line_count(), 1);
    ///
    /// // Content gets trailing newline added if missing
    /// let buf = Buffer::new_from_str("hello");
    /// assert_eq!(buf.rope().to_string(), "hello\n");
    ///
    /// // Content with newline unchanged
    /// let buf = Buffer::new_from_str("hello\n");
    /// assert_eq!(buf.rope().to_string(), "hello\n");
    /// ```
    pub fn new_from_str(content: &str) -> Self {
        // Ensure content always ends with newline (Vim behavior)
        // Empty string is valid and creates an empty rope (0 chars, 1 empty line)
        let rope = if content.is_empty() || content.ends_with('\n') {
            Rope::from_str(content)
        } else {
            // Only allocate when we need to add a newline
            let mut rope = Rope::from_str(content);
            rope.insert(rope.len_chars(), "\n");
            rope
        };

        // Debug assertions to validate expected rope state
        #[cfg(debug_assertions)]
        {
            if content.is_empty() {
                // Empty input creates empty rope (0 chars)
                assert_eq!(
                    rope.len_chars(),
                    0,
                    "Empty buffer should have 0 chars, got {}",
                    rope.len_chars()
                );
                assert_eq!(
                    rope.len_lines(),
                    1,
                    "Empty buffer should have 1 empty line, got {}",
                    rope.len_lines()
                );
            } else {
                // Non-empty content must end with newline
                assert!(
                    rope.to_string().ends_with('\n'),
                    "Buffer content must end with newline"
                );
                // Must have at least 1 character (the newline)
                assert!(
                    rope.len_chars() > 0,
                    "Non-empty buffer must have at least 1 char (newline)"
                );
            }
        }

        Self {
            rope,
            cursor: Cursor::new(0, 0),
            modified: false,
            file_path: None,
            line_ending: LineEnding::detect(content.as_bytes()),
            encoding: FileEncoding::Utf8, // from_str always gets valid UTF-8
            syntax: None,
            cached_highlights: None,
            highlight_version: 0,
            pending_rehighlight: false,
            fold_manager: crate::fold::FoldManager::new(),
            git_status: GitStatus::new(),
            git_blame: None,
            change_manager: ChangeManager::new(),
            file_mtime: None,
            read_only: false,
            semantic_highlights: None,
            version: 0,
            code_block_cache: None,
            recording: None,
        }
    }

    /// Gets the line ending style for this buffer
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    /// Gets the file encoding for this buffer
    pub fn encoding(&self) -> FileEncoding {
        self.encoding
    }

    /// Sets the line ending style for this buffer
    pub fn set_line_ending(&mut self, line_ending: LineEnding) {
        self.line_ending = line_ending;
    }

    /// Gets the rope reference
    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// Gets a mutable rope reference
    pub fn rope_mut(&mut self) -> &mut Rope {
        self.modified = true;
        // Blame data becomes stale after any edit
        self.git_blame = None;
        &mut self.rope
    }

    /// Gets the cursor
    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    /// Gets a mutable cursor reference
    pub fn cursor_mut(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    /// Validates and corrects cursor position to ensure it's within buffer bounds
    /// This should be called after operations that may leave cursor in invalid state
    pub fn validate_cursor_position(&mut self) {
        let line = self.cursor.line();
        let col = self.cursor.col();

        // Clamp line to valid range
        let max_line = self.line_count().saturating_sub(1);
        if line > max_line {
            self.cursor.set_line(max_line);
        }

        // Clamp column to valid range for current line
        let current_line = self.cursor.line();
        if let Some(line_content) = self.line(current_line) {
            // Use grapheme clusters for proper multi-codepoint emoji handling
            let line_len = crate::unicode::grapheme_count(line_content.trim_end_matches('\n'));
            if col >= line_len {
                let new_col = if line_len > 0 { line_len - 1 } else { 0 };
                self.cursor.set_col(new_col);
            }
        }
    }

    /// Returns whether the buffer has been modified
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Returns whether the buffer is read-only
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Sets the read-only status of the buffer
    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    /// Returns the current version of this buffer.
    /// The version increments on every edit operation (insert, delete, etc.)
    /// and is used for cache invalidation.
    pub fn version(&self) -> usize {
        self.version
    }

    /// Gets the file path if set
    pub fn file_path(&self) -> Option<&str> {
        self.file_path.as_deref()
    }

    /// Sets the file path
    ///
    /// CRITICAL: This normalizes the path and should ONLY be called when:
    /// 1. Creating a new buffer with a file path
    /// 2. Explicitly changing the buffer's associated file (e.g., :w newfile.txt)
    ///
    /// DO NOT call this during regular saves - it will break LSP URI tracking!
    /// The normalized path becomes the stable URI for LSP communication.
    pub fn set_file_path(&mut self, path: String) {
        let path_buf = PathBuf::from(path);
        let absolute_path = file_io::normalize_path(&path_buf);
        self.file_path = Some(absolute_path.to_string_lossy().to_string());
    }

    /// Gets the number of lines in the buffer (Vim semantics)
    ///
    /// Ropey's `len_lines()` counts a "phantom" empty line after trailing newlines.
    /// For "hello\n", Ropey returns 2 lines, but Vim displays "1 line".
    /// This function adjusts for Vim-compatible line counting.
    ///
    /// Use `raw_line_count()` when you need the actual Ropey line count for
    /// internal rope operations.
    pub fn line_count(&self) -> usize {
        let raw_count = self.rope.len_lines();
        // If buffer ends with newline, don't count the phantom empty line
        // Exception: empty buffer (0 chars) should still report 1 line
        if raw_count > 1 && self.rope.len_chars() > 0 {
            let last_char = self.rope.char(self.rope.len_chars() - 1);
            if last_char == '\n' {
                return raw_count - 1;
            }
        }
        raw_count
    }

    /// Gets the raw Ropey line count (includes phantom empty line after trailing newline)
    ///
    /// Use this for internal rope operations where you need to access all lines
    /// including the phantom empty line. For user-facing line counts, use `line_count()`.
    pub fn raw_line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Gets a specific line as a String
    pub fn line(&self, idx: usize) -> Option<String> {
        if idx < self.line_count() {
            Some(self.rope.line(idx).to_string())
        } else {
            None
        }
    }

    /// Gets a specific line as a RopeSlice (zero-allocation)
    /// Prefer this over line() when you don't need ownership
    pub fn line_slice(&self, idx: usize) -> Option<ropey::RopeSlice<'_>> {
        if idx < self.line_count() {
            Some(self.rope.line(idx))
        } else {
            None
        }
    }

    /// Gets the length of a line in characters (excluding newline)
    /// More efficient than getting the line and calling .len() on it
    pub fn line_len(&self, idx: usize) -> usize {
        self.line_content_len(idx)
    }

    /// Characters excluding trailing newline (content length only).
    /// Use this when you need the length of visible content on a line.
    pub fn line_content_len(&self, idx: usize) -> usize {
        if idx >= self.line_count() {
            return 0;
        }

        let line_slice = self.rope.line(idx);
        let mut len = line_slice.len_chars();

        // Subtract 1 if line ends with newline
        if len > 0 && line_slice.char(len - 1) == '\n' {
            len -= 1;
        }

        len
    }

    /// Characters including trailing newline (raw rope line length).
    /// Use this when you need the actual rope character count for a line,
    /// e.g. for computing absolute char offsets.
    pub fn line_raw_len(&self, idx: usize) -> usize {
        if idx >= self.line_count() {
            return 0;
        }

        self.rope.line(idx).len_chars()
    }

    /// Gets a character at a specific position in a line (zero-allocation)
    /// Returns None if the line or column is out of bounds
    pub fn char_at(&self, line_idx: usize, col: usize) -> Option<char> {
        let line_slice = self.line_slice(line_idx)?;
        let len = line_slice.len_chars();

        // Exclude newline character
        if col < len {
            let ch = line_slice.char(col);
            if ch != '\n' {
                return Some(ch);
            }
        }
        None
    }

    /// Checks if a line is blank (contains only whitespace, zero-allocation)
    pub fn is_line_blank(&self, idx: usize) -> bool {
        if let Some(line_slice) = self.line_slice(idx) {
            // Check all characters in the line (excluding the newline)
            for ch in line_slice.chars() {
                if ch == '\n' {
                    break;
                }
                if !ch.is_whitespace() {
                    return false;
                }
            }
            true
        } else {
            true
        }
    }

    /// Finds the column of the first non-whitespace character on a line (zero-allocation)
    /// Returns 0 if the line is blank or doesn't exist
    pub fn first_non_blank_col(&self, idx: usize) -> usize {
        if let Some(line_slice) = self.line_slice(idx) {
            for (i, ch) in line_slice.chars().enumerate() {
                if ch == '\n' {
                    break;
                }
                if !ch.is_whitespace() {
                    return i;
                }
            }
        }
        0
    }

    /// Finds the column of the last non-whitespace character on a line (zero-allocation)
    /// Returns 0 if the line is blank or doesn't exist
    pub fn last_non_blank_col(&self, idx: usize) -> usize {
        if let Some(line_slice) = self.line_slice(idx) {
            let mut last_non_blank = 0;
            for (i, ch) in line_slice.chars().enumerate() {
                if ch == '\n' {
                    break;
                }
                if !ch.is_whitespace() {
                    last_non_blank = i;
                }
            }
            return last_non_blank;
        }
        0
    }

    /// Finds the position of a character in a line starting from a given column (zero-allocation)
    /// Returns None if the character is not found
    pub fn find_char_in_line(
        &self,
        line_idx: usize,
        start_col: usize,
        target: char,
    ) -> Option<usize> {
        let line_slice = self.line_slice(line_idx)?;

        for (i, ch) in line_slice.chars().enumerate() {
            if ch == '\n' {
                break;
            }
            if i >= start_col && ch == target {
                return Some(i);
            }
        }
        None
    }

    /// Finds the position of a character in a line searching backwards from a given column (zero-allocation)
    /// Returns None if the character is not found
    pub fn find_char_in_line_rev(
        &self,
        line_idx: usize,
        start_col: usize,
        target: char,
    ) -> Option<usize> {
        let line_slice = self.line_slice(line_idx)?;

        let chars_up_to: Vec<(usize, char)> = line_slice
            .chars()
            .enumerate()
            .take_while(|(_, ch)| *ch != '\n')
            .take(start_col + 1)
            .collect();

        for (i, ch) in chars_up_to.iter().rev() {
            if *ch == target {
                return Some(*i);
            }
        }
        None
    }

    /// Executes a closure while recording all buffer edits (insert_text_at, delete_range).
    /// Returns the closure's result and the recorded edits.
    ///
    /// Recording is opt-in: existing code that doesn't call `record()` is unaffected.
    /// Nested `record()` calls are not supported — the inner call will overwrite the
    /// outer recording. This is intentional for now; nesting isn't needed yet.
    pub fn record<F, R>(&mut self, f: F) -> (R, Vec<Edit>)
    where
        F: FnOnce(&mut Self) -> R,
    {
        self.recording = Some(Vec::new());
        let result = f(self);
        let edits = self.recording.take().unwrap_or_default();
        (result, edits)
    }

    /// Returns whether the buffer is currently recording edits.
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Marks the buffer as unmodified (e.g., after saving)
    pub fn mark_clean(&mut self) {
        self.modified = false;
    }

    /// Checks if a line is hidden by a fold
    pub fn is_line_folded(&self, line: usize) -> bool {
        self.fold_manager.is_line_hidden(line)
    }

    /// Gets the fold manager
    pub fn fold_manager(&self) -> &crate::fold::FoldManager {
        &self.fold_manager
    }

    /// Gets mutable fold manager
    pub fn fold_manager_mut(&mut self) -> &mut crate::fold::FoldManager {
        &mut self.fold_manager
    }

    /// Creates a fold from start_line to end_line
    pub fn create_fold(&mut self, start_line: usize, end_line: usize) {
        self.fold_manager.create_fold(start_line, end_line);
    }

    /// Opens fold at a line
    pub fn open_fold(&mut self, line: usize) {
        self.fold_manager.open_fold_at(line);
    }

    /// Closes fold at a line
    pub fn close_fold(&mut self, line: usize) {
        self.fold_manager.close_fold_at(line);
    }

    /// Toggles fold at a line
    pub fn toggle_fold(&mut self, line: usize) {
        self.fold_manager.toggle_fold_at(line);
    }

    /// Clears all folds
    pub fn clear_folds(&mut self) {
        self.fold_manager.delete_all();
    }

    /// Refreshes git status for this buffer
    /// Returns the duration in microseconds if git status was refreshed
    pub fn refresh_git_status(&mut self) -> Option<u64> {
        if let Some(ref path) = self.file_path {
            let start = std::time::Instant::now();
            self.git_status = GitStatus::from_file(path).unwrap_or_else(|_| GitStatus::new());
            Some(start.elapsed().as_micros() as u64)
        } else {
            None
        }
    }

    /// Gets the git status for this buffer
    pub fn git_status(&self) -> &GitStatus {
        &self.git_status
    }

    /// Gets the git blame data for this buffer (if loaded)
    pub fn git_blame(&self) -> Option<&GitBlame> {
        self.git_blame.as_ref()
    }

    /// Loads git blame data for the current file
    pub fn load_git_blame(&mut self) {
        if let Some(ref path) = self.file_path {
            self.git_blame = GitBlame::from_file(path).ok().filter(|b| !b.is_empty());
        }
    }

    /// Clears cached git blame data (e.g. after edits make it stale)
    pub fn clear_git_blame(&mut self) {
        self.git_blame = None;
    }

    /// Gets a reference to the change manager
    pub fn change_manager(&self) -> &ChangeManager {
        &self.change_manager
    }

    /// Gets a mutable reference to the change manager
    pub fn change_manager_mut(&mut self) -> &mut ChangeManager {
        &mut self.change_manager
    }

    /// Undoes the last change
    pub fn undo(&mut self) -> bool {
        // Pop change from undo stack
        if let Some(change) = self.change_manager.undo_stack.pop() {
            // Apply the undo
            change.undo(self);
            // Push to redo stack
            self.change_manager.redo_stack.push(change);
            true
        } else {
            false
        }
    }

    /// Redoes the next change
    pub fn redo(&mut self) -> bool {
        // Pop change from redo stack
        if let Some(change) = self.change_manager.redo_stack.pop() {
            // Re-apply the change
            change.apply(self);
            // Push to undo stack
            self.change_manager.undo_stack.push(change);
            // Validate cursor position (apply may restore insert-mode cursor_after
            // which can be past end of line in normal mode)
            self.validate_cursor_position();
            true
        } else {
            false
        }
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_str_empty_buffer() {
        // Empty string creates empty rope with 1 empty line
        let buf = Buffer::new_from_str("");

        assert_eq!(
            buf.rope().len_chars(),
            0,
            "Empty buffer should have 0 chars"
        );
        assert_eq!(
            buf.rope().len_lines(),
            1,
            "Empty buffer should have 1 empty line"
        );
        assert_eq!(buf.line_count(), 1, "Empty buffer should report 1 line");
        assert_eq!(
            buf.line(0),
            Some("".to_string()),
            "First line should be empty string"
        );
        assert_eq!(buf.cursor().line(), 0, "Cursor should be at line 0");
        assert_eq!(buf.cursor().col(), 0, "Cursor should be at col 0");
        assert!(!buf.is_modified(), "New buffer should not be modified");
    }

    #[test]
    fn test_from_str_single_newline() {
        // Single newline creates rope with 1 char, 2 lines (one empty, one after newline)
        let buf = Buffer::new_from_str("\n");

        assert_eq!(
            buf.rope().len_chars(),
            1,
            "Single newline should have 1 char"
        );
        assert_eq!(
            buf.rope().len_lines(),
            2,
            "Single newline should have 2 lines"
        );
        assert_eq!(
            buf.rope().to_string(),
            "\n",
            "Content should be just newline"
        );
    }

    #[test]
    fn test_from_str_content_without_newline() {
        // Content without trailing newline gets one added
        let buf = Buffer::new_from_str("hello");

        assert_eq!(buf.rope().to_string(), "hello\n", "Newline should be added");
        assert_eq!(buf.rope().len_chars(), 6, "Should have 5 chars + newline");
        assert_eq!(
            buf.rope().len_lines(),
            2,
            "Ropey counts 2 lines (content + phantom empty)"
        );
        assert_eq!(buf.line_count(), 1, "Vim semantics: 1 line for 'hello\\n'");
        assert_eq!(
            buf.line(0),
            Some("hello\n".to_string()),
            "First line should include newline"
        );
    }

    #[test]
    fn test_from_str_content_with_newline() {
        // Content with trailing newline is unchanged
        let buf = Buffer::new_from_str("hello\n");

        assert_eq!(
            buf.rope().to_string(),
            "hello\n",
            "Content should be unchanged"
        );
        assert_eq!(buf.rope().len_chars(), 6, "Should have 5 chars + newline");
        assert_eq!(buf.rope().len_lines(), 2, "Should have 2 lines");
    }

    #[test]
    fn test_from_str_multiple_lines() {
        // Multiple lines with trailing newline
        let buf = Buffer::new_from_str("line1\nline2\nline3\n");

        assert_eq!(
            buf.rope().to_string(),
            "line1\nline2\nline3\n",
            "Content should be unchanged"
        );
        assert_eq!(buf.line_count(), 3, "Vim semantics: 3 lines");
        assert_eq!(buf.line(0), Some("line1\n".to_string()));
        assert_eq!(buf.line(1), Some("line2\n".to_string()));
        assert_eq!(buf.line(2), Some("line3\n".to_string()));
    }

    #[test]
    fn test_from_str_multiple_lines_no_trailing_newline() {
        // Multiple lines without trailing newline gets one added
        let buf = Buffer::new_from_str("line1\nline2\nline3");

        assert_eq!(
            buf.rope().to_string(),
            "line1\nline2\nline3\n",
            "Newline should be added"
        );
        assert_eq!(buf.line_count(), 3, "Vim semantics: 3 lines");
        assert_eq!(
            buf.line(2),
            Some("line3\n".to_string()),
            "Last line should have newline added"
        );
    }

    #[test]
    fn test_from_str_preserves_internal_empty_lines() {
        // Empty lines in the middle should be preserved
        let buf = Buffer::new_from_str("line1\n\nline3\n");

        assert_eq!(buf.rope().to_string(), "line1\n\nline3\n");
        assert_eq!(
            buf.line_count(),
            3,
            "Vim semantics: 3 lines (line1, empty, line3)"
        );
        assert_eq!(buf.line(0), Some("line1\n".to_string()));
        assert_eq!(
            buf.line(1),
            Some("\n".to_string()),
            "Middle line should be just newline"
        );
        assert_eq!(buf.line(2), Some("line3\n".to_string()));
    }

    #[test]
    fn test_from_str_initial_state() {
        // Verify all initial state is set correctly
        let buf = Buffer::new_from_str("test");

        assert_eq!(buf.cursor().line(), 0);
        assert_eq!(buf.cursor().col(), 0);
        assert!(!buf.is_modified());
        assert!(buf.file_path().is_none());
    }

    #[test]
    fn test_new_vs_from_str_empty() {
        // new() and from_str("") should create equivalent buffers
        let buf1 = Buffer::new();
        let buf2 = Buffer::new_from_str("");

        assert_eq!(buf1.rope().len_chars(), buf2.rope().len_chars());
        assert_eq!(buf1.rope().len_lines(), buf2.rope().len_lines());
        assert_eq!(buf1.line_count(), buf2.line_count());
    }

    // --- line_content_len / line_raw_len tests ---

    #[test]
    fn test_line_content_len_basic() {
        let buf = Buffer::new_from_str("hello\nworld\n");
        assert_eq!(buf.line_content_len(0), 5); // "hello" without \n
        assert_eq!(buf.line_content_len(1), 5); // "world" without \n
    }

    #[test]
    fn test_line_content_len_empty_line() {
        let buf = Buffer::new_from_str("hello\n\nworld\n");
        assert_eq!(buf.line_content_len(0), 5);
        assert_eq!(buf.line_content_len(1), 0); // empty line
        assert_eq!(buf.line_content_len(2), 5);
    }

    #[test]
    fn test_line_content_len_empty_buffer() {
        let buf = Buffer::new_from_str("");
        assert_eq!(buf.line_content_len(0), 0);
    }

    #[test]
    fn test_line_content_len_out_of_bounds() {
        let buf = Buffer::new_from_str("hello\n");
        assert_eq!(buf.line_content_len(99), 0);
    }

    #[test]
    fn test_line_raw_len_basic() {
        let buf = Buffer::new_from_str("hello\nworld\n");
        assert_eq!(buf.line_raw_len(0), 6); // "hello\n"
        assert_eq!(buf.line_raw_len(1), 6); // "world\n"
    }

    #[test]
    fn test_line_raw_len_empty_line() {
        let buf = Buffer::new_from_str("hello\n\nworld\n");
        assert_eq!(buf.line_raw_len(0), 6);
        assert_eq!(buf.line_raw_len(1), 1); // just "\n"
        assert_eq!(buf.line_raw_len(2), 6);
    }

    #[test]
    fn test_line_raw_len_empty_buffer() {
        let buf = Buffer::new_from_str("");
        assert_eq!(buf.line_raw_len(0), 0);
    }

    #[test]
    fn test_line_raw_len_out_of_bounds() {
        let buf = Buffer::new_from_str("hello\n");
        assert_eq!(buf.line_raw_len(99), 0);
    }

    #[test]
    fn test_line_len_matches_line_content_len() {
        // line_len should be identical to line_content_len
        let buf = Buffer::new_from_str("hello\n\nworld\n");
        for i in 0..buf.line_count() {
            assert_eq!(buf.line_len(i), buf.line_content_len(i));
        }
    }

    // --- Buffer recording tests ---

    #[test]
    fn test_record_insert() {
        let mut buf = Buffer::new_from_str("hello\n");
        let ((), edits) = buf.record(|b| {
            b.insert_text_at(0, 5, " world");
        });
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0],
            crate::edit::Edit::Insert {
                offset: 5,
                text: " world".to_string()
            }
        );
        assert_eq!(buf.rope().to_string(), "hello world\n");
    }

    #[test]
    fn test_record_delete() {
        let mut buf = Buffer::new_from_str("hello world\n");
        let (deleted, edits) = buf.record(|b| b.delete_range(0, 5, 0, 11));
        assert_eq!(deleted, " world");
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0],
            crate::edit::Edit::Delete {
                offset: 5,
                text: " world".to_string()
            }
        );
        assert_eq!(buf.rope().to_string(), "hello\n");
    }

    #[test]
    fn test_record_multiple_ops() {
        let mut buf = Buffer::new_from_str("hello world\n");
        let ((), edits) = buf.record(|b| {
            // Delete " world"
            b.delete_range(0, 5, 0, 11);
            // Insert " rust"
            b.insert_text_at(0, 5, " rust");
        });
        assert_eq!(edits.len(), 2);
        assert_eq!(buf.rope().to_string(), "hello rust\n");
    }

    #[test]
    fn test_record_no_ops() {
        let mut buf = Buffer::new_from_str("hello\n");
        let ((), edits) = buf.record(|_b| {
            // do nothing
        });
        assert!(edits.is_empty());
        assert_eq!(buf.rope().to_string(), "hello\n");
    }

    #[test]
    fn test_not_recording_by_default() {
        let mut buf = Buffer::new_from_str("hello\n");
        assert!(!buf.is_recording());
        buf.insert_text_at(0, 5, " world");
        // No recording vec, so nothing captured — that's fine
        assert_eq!(buf.rope().to_string(), "hello world\n");
    }

    #[test]
    fn test_record_join_lines() {
        let mut buf = Buffer::new_from_str("line1\nline2\n");
        buf.cursor_mut().set_position(0, 0);
        let (result, edits) = buf.record(|b| b.join_lines(1));
        assert!(result.is_ok());
        assert_eq!(buf.rope().to_string(), "line1 line2\n");
        // join_lines does delete_range + insert_text_at internally
        assert_eq!(edits.len(), 2);
    }

    #[test]
    fn test_record_delete_char_range() {
        let mut buf = Buffer::new_from_str("hello world\n");
        let ((), edits) = buf.record(|b| {
            b.delete_char_range(5, 11);
        });
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0],
            crate::edit::Edit::Delete {
                offset: 5,
                text: " world".to_string()
            }
        );
        assert_eq!(buf.rope().to_string(), "hello\n");
    }
}
