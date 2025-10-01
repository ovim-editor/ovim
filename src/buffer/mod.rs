mod cursor;

pub use cursor::Cursor;

use anyhow::{Context, Result};
use ropey::Rope;
use std::fs;
use std::path::Path;

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
}

impl Buffer {
    /// Creates a new empty buffer
    pub fn new() -> Self {
        Self {
            rope: Rope::new(),
            cursor: Cursor::new(0, 0),
            modified: false,
            file_path: None,
        }
    }

    /// Creates a buffer from a string
    pub fn from_str(content: &str) -> Self {
        Self {
            rope: Rope::from_str(content),
            cursor: Cursor::new(0, 0),
            modified: false,
            file_path: None,
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

        self.rope.insert(insert_pos, text);
        self.modified = true;
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
        self.rope.remove(start_pos..end_pos);
        self.modified = true;

        deleted
    }

    /// Loads a file into the buffer
    pub fn load_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        let content = fs::read_to_string(&path)
            .context(format!("Failed to read file: {}", path_str))?;

        Ok(Self {
            rope: Rope::from_str(&content),
            cursor: Cursor::new(0, 0),
            modified: false,
            file_path: Some(path_str),
        })
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
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}
