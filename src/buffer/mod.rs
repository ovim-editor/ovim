mod cursor;

pub use cursor::Cursor;

use crate::editor::ChangeManager;
use crate::syntax::{HighlightGroup, LanguageRegistry, SyntaxHighlighter};
use crate::GitStatus;
use anyhow::{Context, Result};
use ropey::Rope;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// Line ending style for the buffer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    /// Unix-style line endings (LF, \n)
    #[default]
    Lf,
    /// Windows-style line endings (CRLF, \r\n)
    Crlf,
}

impl LineEnding {
    /// Detects the line ending style from file content bytes
    pub fn detect(content: &[u8]) -> Self {
        // Look for \r\n first (Windows)
        for window in content.windows(2) {
            if window == b"\r\n" {
                return LineEnding::Crlf;
            }
        }
        // Default to LF (Unix) - this handles \n only or no line endings
        LineEnding::Lf
    }

    /// Returns the string representation of this line ending
    pub fn as_str(&self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
        }
    }

    /// Returns a short display name for the status line
    pub fn display_name(&self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::Crlf => "CRLF",
        }
    }
}

/// Large file threshold in lines - files above this disable expensive features
const LARGE_FILE_LINES: usize = 50_000;

/// Large file threshold in bytes - files above this disable expensive features
const LARGE_FILE_BYTES: usize = 5 * 1024 * 1024; // 5MB

/// Normalizes a path to an absolute, canonical form.
///
/// CRITICAL: This function MUST be deterministic and stable across the file lifecycle.
/// The path returned here becomes the URI for LSP communication. If it changes,
/// LSP loses track of the document, causing "No definition found" and other failures.
///
/// Strategy:
/// - Always make paths absolute (resolve relative paths)
/// - For existing files: canonicalize to resolve symlinks and normalize separators
/// - For non-existent files: use absolute path as-is (no canonicalization)
/// - NEVER re-normalize after initial buffer creation
fn normalize_path(path: &Path) -> PathBuf {
    // Step 1: Make absolute
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        // Resolve relative to current directory
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(path),
            Err(_) => path.to_path_buf(), // Fallback if cwd fails
        }
    };

    // Step 2: Canonicalize ONLY if file exists
    // This prevents path changes when file is created later
    match absolute.try_exists() {
        Ok(true) => {
            // File exists - safe to canonicalize
            std::fs::canonicalize(&absolute).unwrap_or(absolute)
        }
        Ok(false) | Err(_) => {
            // File doesn't exist or we can't determine - use absolute path as-is
            // This ensures new files have stable URIs before their first save
            absolute
        }
    }
}

/// Represents a text buffer using a Rope data structure for efficient editing
pub struct Buffer {
    /// The rope data structure holding the text content
    rope: Rope,
    /// The current cursor position
    cursor: Cursor,
    /// Whether the buffer has been modified since last save
    modified: bool,
    /// Optional file path for this buffer
    file_path: Option<String>,
    /// Line ending style for this buffer (LF or CRLF)
    line_ending: LineEnding,
    /// Optional syntax highlighter
    syntax: Option<SyntaxHighlighter>,
    /// Cached syntax highlights per line (line_idx -> Vec<(range, group)>)
    cached_highlights: Option<Vec<Vec<(Range<usize>, HighlightGroup)>>>,
    /// Version counter for highlight cache (incremented on every edit)
    highlight_version: u64,
    /// Whether re-highlighting is pending
    pending_rehighlight: bool,
    /// Fold manager for code folding
    fold_manager: crate::editor::FoldManager,
    /// Git status for this buffer
    git_status: GitStatus,
    /// Change manager for undo/redo (per-buffer)
    change_manager: ChangeManager,
    /// Last known file modification time (for external change detection)
    file_mtime: Option<std::time::SystemTime>,
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
            syntax: None,
            cached_highlights: None,
            highlight_version: 0,
            pending_rehighlight: false,
            fold_manager: crate::editor::FoldManager::new(),
            git_status: GitStatus::new(),
            change_manager: ChangeManager::new(),
            file_mtime: None,
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
    /// use ovim::buffer::Buffer;
    ///
    /// // Empty buffer has 0 chars, 1 empty line
    /// let buf = Buffer::from_str("");
    /// assert_eq!(buf.rope().len_chars(), 0);
    /// assert_eq!(buf.line_count(), 1);
    ///
    /// // Content gets trailing newline added if missing
    /// let buf = Buffer::from_str("hello");
    /// assert_eq!(buf.rope().to_string(), "hello\n");
    ///
    /// // Content with newline unchanged
    /// let buf = Buffer::from_str("hello\n");
    /// assert_eq!(buf.rope().to_string(), "hello\n");
    /// ```
    pub fn from_str(content: &str) -> Self {
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
            syntax: None,
            cached_highlights: None,
            highlight_version: 0,
            pending_rehighlight: false,
            fold_manager: crate::editor::FoldManager::new(),
            git_status: GitStatus::new(),
            change_manager: ChangeManager::new(),
            file_mtime: None,
        }
    }

    /// Gets the line ending style for this buffer
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
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

    /// Returns whether the buffer has been modified
    pub fn is_modified(&self) -> bool {
        self.modified
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
        let absolute_path = normalize_path(&path_buf);
        self.file_path = Some(absolute_path.to_string_lossy().to_string());
    }

    /// Gets the number of lines in the buffer
    pub fn line_count(&self) -> usize {
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
    pub fn line_slice(&self, idx: usize) -> Option<ropey::RopeSlice> {
        if idx < self.line_count() {
            Some(self.rope.line(idx))
        } else {
            None
        }
    }

    /// Gets the length of a line in characters (excluding newline)
    /// More efficient than getting the line and calling .len() on it
    pub fn line_len(&self, idx: usize) -> usize {
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

    /// Marks the buffer as unmodified (e.g., after saving)
    pub fn mark_clean(&mut self) {
        self.modified = false;
    }

    /// Inserts text at a specific position (line, col)
    pub fn insert_text_at(&mut self, line: usize, col: usize, text: &str) {
        if line >= self.line_count() {
            return;
        }

        let line_start = self.rope.line_to_char(line);
        let insert_pos = line_start + col;

        // Clamp to valid position
        let insert_pos = insert_pos.min(self.rope.len_chars());

        // Shift highlights BEFORE modifying rope
        self.shift_highlights_for_insertion(line, col, text);

        self.rope.insert(insert_pos, text);
        self.modified = true;

        // Increment version and mark re-highlight as pending
        self.highlight_version = self.highlight_version.wrapping_add(1);
        self.pending_rehighlight = true;
    }

    /// Deletes text in a range and returns the deleted text
    pub fn delete_range(
        &mut self,
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
    ) -> String {
        if start_line >= self.line_count() {
            return String::new();
        }

        // Validate start column is within line bounds to prevent addition overflow
        let start_line_len = self.line_len(start_line);
        let actual_start_col = start_col.min(start_line_len);

        let start_line_char = self.rope.line_to_char(start_line);
        let start_pos = start_line_char + actual_start_col;

        let end_pos = if end_line >= self.line_count() {
            self.rope.len_chars()
        } else {
            // Validate end column is within line bounds to prevent addition overflow
            let end_line_len = self.line_len(end_line);
            let actual_end_col = end_col.min(end_line_len);

            let end_line_char = self.rope.line_to_char(end_line);
            end_line_char + actual_end_col
        };

        // Final safety clamp to buffer length (should be redundant after column validation)
        let start_pos = start_pos.min(self.rope.len_chars());
        let end_pos = end_pos.min(self.rope.len_chars());

        if start_pos >= end_pos {
            return String::new();
        }

        let deleted = self.rope.slice(start_pos..end_pos).to_string();

        // Shift highlights BEFORE modifying rope
        self.shift_highlights_for_deletion(start_line, start_col, end_line, end_col);

        self.rope.remove(start_pos..end_pos);
        self.modified = true;

        // Increment version and mark re-highlight as pending
        self.highlight_version = self.highlight_version.wrapping_add(1);
        self.pending_rehighlight = true;

        deleted
    }

    /// Loads a file into the buffer (async version)
    pub async fn load_file_async<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_ref = path.as_ref();
        let absolute_path = normalize_path(path_ref);
        let path_str = absolute_path.to_string_lossy().to_string();

        // Get file modification time for external change detection
        let file_mtime = tokio::fs::metadata(&absolute_path)
            .await
            .ok()
            .and_then(|m| m.modified().ok());

        // Read as bytes first to detect line endings and validate UTF-8
        let bytes = tokio::fs::read(&absolute_path)
            .await
            .context(format!("Failed to read file: {}", path_str))?;

        // Detect line ending style before conversion
        let line_ending = LineEnding::detect(&bytes);

        // Validate UTF-8 with clear error message
        let content = String::from_utf8(bytes).map_err(|e| {
            let valid_up_to = e.utf8_error().valid_up_to();
            anyhow::anyhow!(
                "File '{}' contains invalid UTF-8 at byte position {}\n\
                     This file may be a binary file or use a non-UTF-8 encoding.\n\
                     Only UTF-8 encoded text files are supported.",
                path_str,
                valid_up_to
            )
        })?;

        // Normalize CRLF to LF for internal representation
        // (rope uses LF internally, we convert back on save if needed)
        let content = if line_ending == LineEnding::Crlf {
            content.replace("\r\n", "\n")
        } else {
            content
        };

        let buffer = Self {
            rope: Rope::from_str(&content),
            cursor: Cursor::new(0, 0),
            modified: false,
            file_path: Some(path_str.clone()),
            line_ending,
            syntax: None,
            cached_highlights: None,
            highlight_version: 0,
            pending_rehighlight: false,
            fold_manager: crate::editor::FoldManager::new(),
            git_status: GitStatus::new(),
            change_manager: ChangeManager::new(),
            file_mtime,
        };

        // Don't enable syntax highlighting immediately - defer for lazy loading
        // This makes file loading instant even for large files
        // Syntax highlighting will be triggered later when the buffer is displayed

        // Skip git status on load too - it can be loaded lazily
        // buffer.refresh_git_status();

        Ok(buffer)
    }

    /// Loads a file into the buffer (blocking wrapper for async contexts)
    pub fn load_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_ref = path.as_ref();
        let absolute_path = normalize_path(path_ref);

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(Self::load_file_async(&absolute_path))
        })
    }

    /// Saves the buffer to its file path (async version)
    pub async fn save_async(&mut self) -> Result<()> {
        let path = self
            .file_path
            .as_ref()
            .context("No file path set for buffer")?;
        self.save_as_async(path.clone()).await?;
        Ok(())
    }

    /// Saves the buffer to a specific file path (async version)
    /// Uses atomic write pattern: write to temp file, sync, then rename
    pub async fn save_as_async<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        use tokio::io::AsyncWriteExt;

        let path_ref = path.as_ref();
        let path_str_input = path_ref.to_string_lossy();

        // CRITICAL: Only normalize if this is a NEW path
        // Re-normalizing an existing path can change URIs, breaking LSP tracking
        let absolute_path = if let Some(existing_path) = &self.file_path {
            // Check if input path matches existing path (regular save)
            if path_str_input == existing_path.as_str() {
                // Regular save - use existing path without re-normalization
                PathBuf::from(existing_path)
            } else {
                // Save As with different path - normalize the new path
                normalize_path(path_ref)
            }
        } else {
            // No existing path (new buffer) - normalize it
            normalize_path(path_ref)
        };

        let path_ref = absolute_path.as_path();
        let path_str = path_ref.to_string_lossy().to_string();

        // Get content and convert line endings if needed
        let content = self.rope.to_string();
        let content = if self.line_ending == LineEnding::Crlf {
            // Convert LF to CRLF for Windows files
            content.replace('\n', "\r\n")
        } else {
            content
        };

        // Create temp file in same directory (ensures atomic rename on same filesystem)
        let temp_path = if let Some(parent) = path_ref.parent() {
            parent.join(format!(
                ".{}.tmp",
                path_ref
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("buffer")
            ))
        } else {
            PathBuf::from(format!("{}.tmp", path_str))
        };

        // Write to temp file
        let mut file = tokio::fs::File::create(&temp_path).await.context(format!(
            "Failed to create temp file: {}",
            temp_path.display()
        ))?;

        file.write_all(content.as_bytes())
            .await
            .context("Failed to write file content")?;

        // CRITICAL: Ensure data reaches disk before rename
        file.sync_all()
            .await
            .context("Failed to sync file to disk")?;

        // Close file before rename
        drop(file);

        // Atomic rename (overwrites destination if exists)
        tokio::fs::rename(&temp_path, path_ref)
            .await
            .context(format!("Failed to rename temp file to {}", path_str))?;

        // CRITICAL: Only update file_path if it changed (Save As scenario)
        // Preserves URI stability for LSP tracking
        if self.file_path.as_deref() != Some(&path_str) {
            self.file_path = Some(path_str);
        }
        self.modified = false;

        // Update mtime after successful save
        self.file_mtime = tokio::fs::metadata(path_ref)
            .await
            .ok()
            .and_then(|m| m.modified().ok());

        Ok(())
    }

    /// Saves the buffer to its file path (blocking wrapper)
    pub fn save(&mut self) -> Result<()> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.save_async())
        })
    }

    /// Saves the buffer to a specific file path (blocking wrapper)
    pub fn save_as<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.save_as_async(path))
        });

        // Refresh git status after save
        if result.is_ok() {
            self.refresh_git_status();
        }

        result
    }

    /// Replaces the entire buffer content
    pub fn replace_all(&mut self, content: &str) {
        self.rope = Rope::from_str(content);
        self.modified = true;
        // Reset cursor to beginning
        self.cursor = Cursor::new(0, 0);
    }

    /// Gets the word under the cursor
    /// Returns the word and its (start_col, end_col) on the current line
    pub fn word_under_cursor(&self) -> Option<(String, usize, usize)> {
        let line_idx = self.cursor.line();
        let col = self.cursor.col();

        if line_idx >= self.line_count() {
            return None;
        }

        // Use rope slice to avoid allocation until we need the final word
        let line_slice = self.line_slice(line_idx)?;

        // Build a chars vector from the slice (excluding newline)
        let chars: Vec<char> = line_slice.chars().take_while(|&c| c != '\n').collect();

        if chars.is_empty() || col >= chars.len() {
            return None;
        }

        // Check if cursor is on a word character
        let is_word_char = |c: char| c.is_alphanumeric() || c == '_';

        if !is_word_char(chars[col]) {
            return None;
        }

        // Find start of word
        let mut start = col;
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }

        // Find end of word
        let mut end = col;
        while end < chars.len() && is_word_char(chars[end]) {
            end += 1;
        }

        // Only allocate String for the final word
        let word: String = chars[start..end].iter().collect();
        Some((word, start, end))
    }

    /// Enables syntax highlighting for this buffer based on file path
    /// Automatically skips large files for performance
    pub fn enable_syntax_highlighting(&mut self) {
        // Don't enable syntax for large files
        if self.is_large_file() {
            eprintln!(
                "Syntax highlighting disabled for large file ({} lines, {:.2} MB)",
                self.line_count(),
                self.rope.len_bytes() as f64 / (1024.0 * 1024.0)
            );
            return;
        }

        if let Some(ref path) = self.file_path {
            if let Some(lang) = LanguageRegistry::detect_from_path(path) {
                if let Ok(mut highlighter) = SyntaxHighlighter::new(lang) {
                    let source = self.rope.to_string();
                    highlighter.parse(&source);

                    // Build initial highlight cache
                    self.build_highlight_cache(&highlighter, &source);

                    self.syntax = Some(highlighter);
                }
            }
        }
    }

    /// Checks if syntax highlighting should be initialized (lazy loading)
    /// Returns true if the buffer has a file path with supported language but no syntax yet
    pub fn should_init_syntax(&self) -> bool {
        // Don't initialize syntax for large files
        if self.is_large_file() {
            return false;
        }

        // Has file path, no syntax yet, and language is supported
        if self.syntax.is_some() {
            return false;
        }

        if let Some(ref path) = self.file_path {
            LanguageRegistry::detect_from_path(path).is_some()
        } else {
            false
        }
    }

    /// Checks if this is a large file (exceeds line or byte threshold)
    pub fn is_large_file(&self) -> bool {
        let line_count = self.line_count();
        let byte_count = self.rope.len_bytes();

        line_count > LARGE_FILE_LINES || byte_count > LARGE_FILE_BYTES
    }

    /// Gets the large file threshold for lines
    pub fn large_file_line_threshold() -> usize {
        LARGE_FILE_LINES
    }

    /// Gets the large file threshold for bytes
    pub fn large_file_byte_threshold() -> usize {
        LARGE_FILE_BYTES
    }

    /// Builds the highlight cache for all lines
    fn build_highlight_cache(&mut self, highlighter: &SyntaxHighlighter, source: &str) {
        // Use the efficient single-pass method that queries the tree once
        self.cached_highlights = Some(highlighter.highlights_for_all_lines(source));
    }

    /// Shifts highlights after an insertion
    fn shift_highlights_for_insertion(&mut self, line: usize, col: usize, text: &str) {
        let Some(ref mut cache) = self.cached_highlights else {
            return; // No cache to shift
        };

        if line >= cache.len() {
            return;
        }

        // Check if insertion contains newlines
        let newline_count = text.matches('\n').count();

        if newline_count == 0 {
            // Single-line insertion: shift highlights on the same line
            let char_count = text.chars().count();

            for (range, _) in &mut cache[line] {
                if range.start >= col {
                    // Highlight starts after insertion point: shift right
                    range.start += char_count;
                    range.end += char_count;
                } else if range.end > col {
                    // Highlight contains insertion point: extend end
                    range.end += char_count;
                }
            }
        } else {
            // Multi-line insertion: handle line splits and shifts
            let lines: Vec<&str> = text.split('\n').collect();
            let last_line_len = lines.last().map(|s| s.chars().count()).unwrap_or(0);

            // Split the current line's highlights at the insertion point
            let current_line_highlights = cache[line].clone();
            let mut before_insert = Vec::new();
            let mut after_insert = Vec::new();

            for (range, group) in current_line_highlights {
                if range.end <= col {
                    // Entirely before insertion
                    before_insert.push((range, group));
                } else if range.start >= col {
                    // Entirely after insertion: will move to new line
                    // Adjust column position (relative to start of new line)
                    let new_start = range.start - col + last_line_len;
                    let new_end = range.end - col + last_line_len;
                    after_insert.push((new_start..new_end, group));
                } else {
                    // Spans insertion point: keep the before part only
                    before_insert.push((range.start..col, group));
                    // The after part would be on the new line, but it's cut off
                    // (We can't split highlights perfectly without re-parsing)
                }
            }

            // Update current line with highlights before insertion
            cache[line] = before_insert;

            // Insert new empty lines for the newlines in the inserted text
            for _ in 0..newline_count {
                cache.insert(line + 1, Vec::new());
            }

            // The last new line gets the highlights that were after the insertion
            if line + newline_count < cache.len() {
                cache[line + newline_count] = after_insert;
            }
        }
    }

    /// Shifts highlights after a deletion
    fn shift_highlights_for_deletion(
        &mut self,
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
    ) {
        let Some(ref mut cache) = self.cached_highlights else {
            return; // No cache to shift
        };

        if start_line >= cache.len() {
            return;
        }

        if start_line == end_line {
            // Single-line deletion
            if start_line >= cache.len() {
                return;
            }

            let deleted_chars = end_col.saturating_sub(start_col);
            let highlights = &mut cache[start_line];

            // Filter and adjust highlights
            highlights.retain_mut(|(range, _)| {
                if range.end <= start_col {
                    // Before deletion: keep as-is
                    true
                } else if range.start >= end_col {
                    // After deletion: shift left
                    range.start = range.start.saturating_sub(deleted_chars);
                    range.end = range.end.saturating_sub(deleted_chars);
                    true
                } else if range.start >= start_col && range.end <= end_col {
                    // Entirely within deletion: remove
                    false
                } else if range.start < start_col && range.end > end_col {
                    // Contains deletion: shrink
                    range.end = start_col + (range.end - end_col);
                    true
                } else if range.start < start_col {
                    // Starts before, ends within deletion
                    range.end = start_col;
                    true
                } else {
                    // Starts within, ends after deletion
                    range.start = start_col;
                    range.end = start_col + (range.end - end_col);
                    true
                }
            });
        } else {
            // Multi-line deletion
            let deleted_lines = end_line - start_line;

            // Get highlights from end of deletion range that survive
            let surviving_highlights = if end_line < cache.len() {
                cache[end_line]
                    .iter()
                    .filter_map(|(range, group)| {
                        if range.start >= end_col {
                            // After deletion point: shift to start line
                            let new_start = start_col + (range.start - end_col);
                            let new_end = start_col + (range.end - end_col);
                            Some((new_start..new_end, *group))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };

            // Trim start line highlights
            if start_line < cache.len() {
                cache[start_line].retain(|(range, _)| range.end <= start_col);
                // Add surviving highlights from end line
                cache[start_line].extend(surviving_highlights);
            }

            // Remove deleted lines
            if start_line + 1 < cache.len() {
                let end = (start_line + deleted_lines + 1).min(cache.len());
                cache.drain(start_line + 1..end);
            }
        }
    }

    /// Gets syntax highlights for a specific line
    /// Returns a list of (column_range, highlight_group) tuples
    pub fn highlights_for_line(&self, line_idx: usize) -> Vec<(Range<usize>, HighlightGroup)> {
        // Use cached highlights if available
        if let Some(ref cache) = self.cached_highlights {
            if line_idx < cache.len() {
                return cache[line_idx].clone();
            }
        }
        Vec::new()
    }

    /// Checks if syntax highlighting is enabled
    pub fn has_syntax_highlighting(&self) -> bool {
        self.syntax.is_some()
    }

    /// Checks if re-highlighting is needed
    pub fn needs_rehighlight(&self) -> bool {
        self.pending_rehighlight && self.syntax.is_some()
    }

    /// Gets data needed for re-highlighting (content, version, language)
    pub fn get_rehighlight_data(&self) -> Option<(String, u64, crate::syntax::Language)> {
        if !self.needs_rehighlight() {
            return None;
        }

        let syntax = self.syntax.as_ref()?;
        let content = self.rope.to_string();
        let version = self.highlight_version;
        let language = syntax.language();

        Some((content, version, language))
    }

    /// Applies re-highlighted results if version matches
    pub fn apply_highlights(
        &mut self,
        highlights: Vec<Vec<(Range<usize>, HighlightGroup)>>,
        version: u64,
    ) -> bool {
        // Only apply if version matches (buffer hasn't changed since re-parse started)
        if self.highlight_version == version {
            self.cached_highlights = Some(highlights);
            self.pending_rehighlight = false;
            true
        } else {
            false
        }
    }

    /// Checks if a line is hidden by a fold
    pub fn is_line_folded(&self, line: usize) -> bool {
        self.fold_manager.is_line_hidden(line)
    }

    /// Gets the fold manager
    pub fn fold_manager(&self) -> &crate::editor::FoldManager {
        &self.fold_manager
    }

    /// Gets mutable fold manager
    pub fn fold_manager_mut(&mut self) -> &mut crate::editor::FoldManager {
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
            true
        } else {
            false
        }
    }

    /// Repeats the last change
    pub fn repeat_last_change(&mut self) -> bool {
        // Clone the last change
        if let Some(ref change) = self.change_manager.last_change {
            let repeated_change = change.clone();
            // Repeat it
            repeated_change.repeat(self);
            // Push to undo stack as a new change
            self.change_manager.undo_stack.push(repeated_change.clone());
            self.change_manager.redo_stack.clear();
            self.change_manager.last_change = Some(repeated_change);
            true
        } else {
            false
        }
    }

    // ========== Swap/Backup File Support ==========

    /// Returns the path to the swap file for this buffer
    /// Swap files are named .{filename}.swp in the same directory
    pub fn swap_file_path(&self) -> Option<PathBuf> {
        let file_path = self.file_path.as_ref()?;
        let path = Path::new(file_path);
        let parent = path.parent().unwrap_or(Path::new("."));
        let file_name = path.file_name()?.to_str()?;
        Some(parent.join(format!(".{}.swp", file_name)))
    }

    /// Returns the path to the backup file for this buffer
    /// Backup files are named {filename}~ in the same directory
    pub fn backup_file_path(&self) -> Option<PathBuf> {
        let file_path = self.file_path.as_ref()?;
        Some(PathBuf::from(format!("{}~", file_path)))
    }

    /// Checks if a swap file exists for this buffer
    pub fn has_swap_file(&self) -> bool {
        self.swap_file_path()
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// Writes the current buffer content to the swap file
    pub fn write_swap_file(&self) -> Result<()> {
        let swap_path = self.swap_file_path()
            .context("Cannot write swap file: no file path set")?;

        let content = self.rope.to_string();
        std::fs::write(&swap_path, &content)
            .context(format!("Failed to write swap file: {}", swap_path.display()))?;

        Ok(())
    }

    /// Deletes the swap file if it exists
    pub fn delete_swap_file(&self) -> Result<()> {
        if let Some(swap_path) = self.swap_file_path() {
            if swap_path.exists() {
                std::fs::remove_file(&swap_path)
                    .context(format!("Failed to delete swap file: {}", swap_path.display()))?;
            }
        }
        Ok(())
    }

    /// Creates a backup of the original file before saving
    pub fn create_backup_file(&self) -> Result<()> {
        let file_path = self.file_path.as_ref()
            .context("Cannot create backup: no file path set")?;
        let backup_path = self.backup_file_path()
            .context("Cannot create backup path")?;

        let source = Path::new(file_path);
        if source.exists() {
            std::fs::copy(source, &backup_path)
                .context(format!("Failed to create backup: {}", backup_path.display()))?;
        }

        Ok(())
    }

    /// Reads content from the swap file if it exists
    pub fn read_swap_file(&self) -> Result<Option<String>> {
        let swap_path = match self.swap_file_path() {
            Some(p) if p.exists() => p,
            _ => return Ok(None),
        };

        let content = std::fs::read_to_string(&swap_path)
            .context(format!("Failed to read swap file: {}", swap_path.display()))?;

        Ok(Some(content))
    }

    /// Recovers buffer content from the swap file
    pub fn recover_from_swap_file(&mut self) -> Result<bool> {
        if let Some(content) = self.read_swap_file()? {
            // Ensure content ends with newline
            let content = if content.ends_with('\n') {
                content
            } else {
                format!("{}\n", content)
            };

            self.rope = Rope::from_str(&content);
            self.modified = true; // Mark as modified so user knows to save
            self.pending_rehighlight = true; // Trigger rehighlighting

            // Delete the swap file after successful recovery
            self.delete_swap_file()?;

            Ok(true)
        } else {
            Ok(false)
        }
    }

    // ========== External File Change Detection ==========

    /// Returns the last known file modification time
    pub fn file_mtime(&self) -> Option<std::time::SystemTime> {
        self.file_mtime
    }

    /// Updates the stored file modification time
    pub fn set_file_mtime(&mut self, mtime: Option<std::time::SystemTime>) {
        self.file_mtime = mtime;
    }

    /// Checks if the file has been modified externally since we loaded/saved it
    /// Returns Ok(true) if file changed, Ok(false) if unchanged, Err if can't determine
    pub fn check_external_modification(&self) -> Result<bool> {
        let file_path = self.file_path.as_ref()
            .context("No file path set")?;

        let path = Path::new(file_path);
        if !path.exists() {
            // File was deleted externally
            return Ok(true);
        }

        let current_mtime = std::fs::metadata(path)
            .context("Failed to get file metadata")?
            .modified()
            .context("Failed to get file modification time")?;

        match self.file_mtime {
            Some(stored_mtime) => Ok(current_mtime != stored_mtime),
            None => Ok(false), // No stored mtime means this is a new buffer
        }
    }

    /// Reloads the buffer from disk if it was modified externally
    /// Returns true if reload happened, false if no change
    pub async fn reload_if_changed(&mut self) -> Result<bool> {
        if !self.check_external_modification()? {
            return Ok(false);
        }

        let file_path = self.file_path.clone()
            .context("No file path set")?;

        // Read file content
        let bytes = tokio::fs::read(&file_path)
            .await
            .context(format!("Failed to read file: {}", file_path))?;

        // Update mtime
        self.file_mtime = tokio::fs::metadata(&file_path)
            .await
            .ok()
            .and_then(|m| m.modified().ok());

        // Detect line ending
        self.line_ending = LineEnding::detect(&bytes);

        // Validate UTF-8
        let content = String::from_utf8(bytes).map_err(|e| {
            anyhow::anyhow!("File contains invalid UTF-8 at byte {}", e.utf8_error().valid_up_to())
        })?;

        // Normalize CRLF
        let content = if self.line_ending == LineEnding::Crlf {
            content.replace("\r\n", "\n")
        } else {
            content
        };

        // Update rope
        self.rope = Rope::from_str(&content);
        self.modified = false;
        self.pending_rehighlight = true;

        Ok(true)
    }

    /// Blocking version of reload_if_changed
    pub fn reload_if_changed_sync(&mut self) -> Result<bool> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.reload_if_changed())
        })
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
        let buf = Buffer::from_str("");

        assert_eq!(buf.rope().len_chars(), 0, "Empty buffer should have 0 chars");
        assert_eq!(buf.rope().len_lines(), 1, "Empty buffer should have 1 empty line");
        assert_eq!(buf.line_count(), 1, "Empty buffer should report 1 line");
        assert_eq!(buf.line(0), Some("".to_string()), "First line should be empty string");
        assert_eq!(buf.cursor().line(), 0, "Cursor should be at line 0");
        assert_eq!(buf.cursor().col(), 0, "Cursor should be at col 0");
        assert!(!buf.is_modified(), "New buffer should not be modified");
    }

    #[test]
    fn test_from_str_single_newline() {
        // Single newline creates rope with 1 char, 2 lines (one empty, one after newline)
        let buf = Buffer::from_str("\n");

        assert_eq!(buf.rope().len_chars(), 1, "Single newline should have 1 char");
        assert_eq!(buf.rope().len_lines(), 2, "Single newline should have 2 lines");
        assert_eq!(buf.rope().to_string(), "\n", "Content should be just newline");
    }

    #[test]
    fn test_from_str_content_without_newline() {
        // Content without trailing newline gets one added
        let buf = Buffer::from_str("hello");

        assert_eq!(buf.rope().to_string(), "hello\n", "Newline should be added");
        assert_eq!(buf.rope().len_chars(), 6, "Should have 5 chars + newline");
        assert_eq!(buf.rope().len_lines(), 2, "Should have 2 lines (content + empty)");
        assert_eq!(buf.line_count(), 2, "Should report 2 lines");
        assert_eq!(buf.line(0), Some("hello\n".to_string()), "First line should include newline");
    }

    #[test]
    fn test_from_str_content_with_newline() {
        // Content with trailing newline is unchanged
        let buf = Buffer::from_str("hello\n");

        assert_eq!(buf.rope().to_string(), "hello\n", "Content should be unchanged");
        assert_eq!(buf.rope().len_chars(), 6, "Should have 5 chars + newline");
        assert_eq!(buf.rope().len_lines(), 2, "Should have 2 lines");
    }

    #[test]
    fn test_from_str_multiple_lines() {
        // Multiple lines with trailing newline
        let buf = Buffer::from_str("line1\nline2\nline3\n");

        assert_eq!(buf.rope().to_string(), "line1\nline2\nline3\n", "Content should be unchanged");
        assert_eq!(buf.line_count(), 4, "Should have 4 lines (3 + empty)");
        assert_eq!(buf.line(0), Some("line1\n".to_string()));
        assert_eq!(buf.line(1), Some("line2\n".to_string()));
        assert_eq!(buf.line(2), Some("line3\n".to_string()));
    }

    #[test]
    fn test_from_str_multiple_lines_no_trailing_newline() {
        // Multiple lines without trailing newline gets one added
        let buf = Buffer::from_str("line1\nline2\nline3");

        assert_eq!(buf.rope().to_string(), "line1\nline2\nline3\n", "Newline should be added");
        assert_eq!(buf.line_count(), 4, "Should have 4 lines");
        assert_eq!(buf.line(2), Some("line3\n".to_string()), "Last line should have newline added");
    }

    #[test]
    fn test_from_str_preserves_internal_empty_lines() {
        // Empty lines in the middle should be preserved
        let buf = Buffer::from_str("line1\n\nline3\n");

        assert_eq!(buf.rope().to_string(), "line1\n\nline3\n");
        assert_eq!(buf.line_count(), 4, "Should have 4 lines");
        assert_eq!(buf.line(0), Some("line1\n".to_string()));
        assert_eq!(buf.line(1), Some("\n".to_string()), "Middle line should be just newline");
        assert_eq!(buf.line(2), Some("line3\n".to_string()));
    }

    #[test]
    fn test_from_str_initial_state() {
        // Verify all initial state is set correctly
        let buf = Buffer::from_str("test");

        assert_eq!(buf.cursor().line(), 0);
        assert_eq!(buf.cursor().col(), 0);
        assert!(!buf.is_modified());
        assert!(buf.file_path().is_none());
    }

    #[test]
    fn test_new_vs_from_str_empty() {
        // new() and from_str("") should create equivalent buffers
        let buf1 = Buffer::new();
        let buf2 = Buffer::from_str("");

        assert_eq!(buf1.rope().len_chars(), buf2.rope().len_chars());
        assert_eq!(buf1.rope().len_lines(), buf2.rope().len_lines());
        assert_eq!(buf1.line_count(), buf2.line_count());
    }
}
