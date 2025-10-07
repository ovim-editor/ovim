mod cursor;

pub use cursor::Cursor;

use anyhow::{Context, Result};
use ropey::Rope;
use std::fs;
use std::path::Path;
use crate::syntax::{SyntaxHighlighter, LanguageRegistry, HighlightGroup};
use std::ops::Range;
use std::collections::HashSet;

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
    /// Optional syntax highlighter
    syntax: Option<SyntaxHighlighter>,
    /// Cached syntax highlights per line (line_idx -> Vec<(range, group)>)
    cached_highlights: Option<Vec<Vec<(Range<usize>, HighlightGroup)>>>,
    /// Version counter for highlight cache (incremented on every edit)
    highlight_version: u64,
    /// Whether re-highlighting is pending
    pending_rehighlight: bool,
    /// Set of line numbers that are folded (start line of each fold)
    /// Lines are 0-indexed
    folded_lines: HashSet<usize>,
}

impl Buffer {
    /// Creates a new empty buffer
    pub fn new() -> Self {
        Self {
            rope: Rope::new(),
            cursor: Cursor::new(0, 0),
            modified: false,
            file_path: None,
            syntax: None,
            cached_highlights: None,
            highlight_version: 0,
            pending_rehighlight: false,
            folded_lines: HashSet::new(),
        }
    }

    /// Creates a buffer from a string
    pub fn from_str(content: &str) -> Self {
        Self {
            rope: Rope::from_str(content),
            cursor: Cursor::new(0, 0),
            modified: false,
            file_path: None,
            syntax: None,
            cached_highlights: None,
            highlight_version: 0,
            pending_rehighlight: false,
            folded_lines: HashSet::new(),
        }
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
    pub fn set_file_path(&mut self, path: String) {
        self.file_path = Some(path);
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
    pub fn delete_range(&mut self, start_line: usize, start_col: usize, end_line: usize, end_col: usize) -> String {
        if start_line >= self.line_count() {
            return String::new();
        }

        let start_line_char = self.rope.line_to_char(start_line);
        let start_pos = start_line_char + start_col;

        let end_pos = if end_line >= self.line_count() {
            self.rope.len_chars()
        } else {
            let end_line_char = self.rope.line_to_char(end_line);
            end_line_char + end_col
        };

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

    /// Loads a file into the buffer
    pub fn load_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        let content = fs::read_to_string(&path)
            .context(format!("Failed to read file: {}", path_str))?;

        let mut buffer = Self {
            rope: Rope::from_str(&content),
            cursor: Cursor::new(0, 0),
            modified: false,
            file_path: Some(path_str),
            syntax: None,
            cached_highlights: None,
            highlight_version: 0,
            pending_rehighlight: false,
            folded_lines: HashSet::new(),
        };

        // Enable syntax highlighting based on file extension
        buffer.enable_syntax_highlighting();

        Ok(buffer)
    }

    /// Saves the buffer to its file path
    pub fn save(&mut self) -> Result<()> {
        let path = self.file_path.as_ref()
            .context("No file path set for buffer")?;
        self.save_as(path.clone())?;
        Ok(())
    }

    /// Saves the buffer to a specific file path
    pub fn save_as<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        let content = self.rope.to_string();
        fs::write(&path, content)
            .context(format!("Failed to write file: {}", path_str))?;

        self.file_path = Some(path_str);
        self.modified = false;
        Ok(())
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

        let line_text = self.line(line_idx)?;
        let line_text = line_text.trim_end_matches('\n');
        let chars: Vec<char> = line_text.chars().collect();

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

        let word: String = chars[start..end].iter().collect();
        Some((word, start, end))
    }

    /// Enables syntax highlighting for this buffer based on file path
    pub fn enable_syntax_highlighting(&mut self) {
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

    /// Builds the highlight cache for all lines
    fn build_highlight_cache(&mut self, highlighter: &SyntaxHighlighter, source: &str) {
        let line_count = self.rope.len_lines();
        let mut cache = Vec::with_capacity(line_count);

        for line_idx in 0..line_count {
            let highlights = highlighter.highlights_for_line(line_idx, source);
            cache.push(highlights);
        }

        self.cached_highlights = Some(cache);
    }

    /// Invalidates the highlight cache (called on buffer edits)
    fn invalidate_highlight_cache(&mut self) {
        // Clear cache - will be empty until next re-parse
        self.cached_highlights = None;
        // Increment version to invalidate any in-flight async re-parse
        self.highlight_version = self.highlight_version.wrapping_add(1);
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
    fn shift_highlights_for_deletion(&mut self, start_line: usize, start_col: usize, end_line: usize, end_col: usize) {
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
    pub fn apply_highlights(&mut self, highlights: Vec<Vec<(Range<usize>, HighlightGroup)>>, version: u64) -> bool {
        // Only apply if version matches (buffer hasn't changed since re-parse started)
        if self.highlight_version == version {
            self.cached_highlights = Some(highlights);
            self.pending_rehighlight = false;
            true
        } else {
            false
        }
    }

    /// Checks if a line is folded
    pub fn is_line_folded(&self, line: usize) -> bool {
        self.folded_lines.contains(&line)
    }

    /// Folds a line (hides lines from start_line to end_line)
    pub fn fold_line(&mut self, start_line: usize) {
        self.folded_lines.insert(start_line);
    }

    /// Unfolds a line
    pub fn unfold_line(&mut self, start_line: usize) {
        self.folded_lines.remove(&start_line);
    }

    /// Toggles fold state for a line
    pub fn toggle_fold(&mut self, start_line: usize) {
        if self.folded_lines.contains(&start_line) {
            self.folded_lines.remove(&start_line);
        } else {
            self.folded_lines.insert(start_line);
        }
    }

    /// Gets all folded lines
    pub fn folded_lines(&self) -> &HashSet<usize> {
        &self.folded_lines
    }

    /// Clears all folds
    pub fn clear_folds(&mut self) {
        self.folded_lines.clear();
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}
