use std::path::PathBuf;

/// Represents a single entry in a quickfix or location list
#[derive(Debug, Clone)]
pub struct QuickfixEntry {
    /// File path (optional, can be empty for non-file entries)
    pub filename: Option<PathBuf>,
    /// Line number (1-indexed, 0 means no specific line)
    pub lnum: usize,
    /// Column number (1-indexed, 0 means no specific column)
    pub col: usize,
    /// Error/warning type or pattern
    pub entry_type: QuickfixEntryType,
    /// Text description
    pub text: String,
}

/// Type of quickfix entry
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickfixEntryType {
    Error,
    Warning,
    Info,
    Note,
}

impl QuickfixEntry {
    /// Creates a new quickfix entry
    pub fn new(
        filename: Option<PathBuf>,
        lnum: usize,
        col: usize,
        entry_type: QuickfixEntryType,
        text: String,
    ) -> Self {
        Self {
            filename,
            lnum,
            col,
            entry_type,
            text,
        }
    }

    /// Creates an error entry
    pub fn error(filename: Option<PathBuf>, lnum: usize, col: usize, text: String) -> Self {
        Self::new(filename, lnum, col, QuickfixEntryType::Error, text)
    }

    /// Creates a warning entry
    pub fn warning(filename: Option<PathBuf>, lnum: usize, col: usize, text: String) -> Self {
        Self::new(filename, lnum, col, QuickfixEntryType::Warning, text)
    }

    /// Creates an info entry
    pub fn info(filename: Option<PathBuf>, lnum: usize, col: usize, text: String) -> Self {
        Self::new(filename, lnum, col, QuickfixEntryType::Info, text)
    }

    /// Gets display text for this entry
    pub fn display_text(&self) -> String {
        let type_char = match self.entry_type {
            QuickfixEntryType::Error => 'E',
            QuickfixEntryType::Warning => 'W',
            QuickfixEntryType::Info => 'I',
            QuickfixEntryType::Note => 'N',
        };

        let file_part = if let Some(ref path) = self.filename {
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            format!("{}:{}:{}", filename, self.lnum, self.col)
        } else {
            "".to_string()
        };

        if file_part.is_empty() {
            format!("{} {}", type_char, self.text)
        } else {
            format!("{} {} {}", file_part, type_char, self.text)
        }
    }
}

/// Quickfix list - global list of locations
#[derive(Debug, Clone)]
pub struct QuickfixList {
    /// List of entries
    entries: Vec<QuickfixEntry>,
    /// Currently selected index
    selected_index: usize,
    /// Title for this list
    title: String,
}

impl QuickfixList {
    /// Creates a new empty quickfix list
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            selected_index: 0,
            title: String::new(),
        }
    }

    /// Creates a quickfix list with entries
    pub fn with_entries(entries: Vec<QuickfixEntry>, title: String) -> Self {
        Self {
            entries,
            selected_index: 0,
            title,
        }
    }

    /// Sets the entries
    pub fn set_entries(&mut self, entries: Vec<QuickfixEntry>, title: String) {
        self.entries = entries;
        self.title = title;
        self.selected_index = 0;
    }

    /// Gets all entries
    pub fn entries(&self) -> &[QuickfixEntry] {
        &self.entries
    }

    /// Gets the title
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Whether the list is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Gets the number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Gets the currently selected index
    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    /// Gets the currently selected entry
    pub fn current_entry(&self) -> Option<&QuickfixEntry> {
        self.entries.get(self.selected_index)
    }

    /// Moves to the next entry
    pub fn next(&mut self) {
        if !self.entries.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.entries.len();
        }
    }

    /// Moves to the previous entry
    pub fn previous(&mut self) {
        if !self.entries.is_empty() {
            if self.selected_index == 0 {
                self.selected_index = self.entries.len() - 1;
            } else {
                self.selected_index -= 1;
            }
        }
    }

    /// Moves to the first entry
    pub fn first(&mut self) {
        self.selected_index = 0;
    }

    /// Moves to the last entry
    pub fn last(&mut self) {
        if !self.entries.is_empty() {
            self.selected_index = self.entries.len() - 1;
        }
    }

    /// Sets the selected index
    pub fn set_selected(&mut self, index: usize) {
        if index < self.entries.len() {
            self.selected_index = index;
        }
    }

    /// Clears all entries
    pub fn clear(&mut self) {
        self.entries.clear();
        self.selected_index = 0;
        self.title.clear();
    }
}

impl Default for QuickfixList {
    fn default() -> Self {
        Self::new()
    }
}

/// Location list - per-window list of locations
pub type LocationList = QuickfixList;
