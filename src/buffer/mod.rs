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
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}
