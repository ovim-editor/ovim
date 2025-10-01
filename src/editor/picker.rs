use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub enum PickerMode {
    FindFiles,
    LiveGrep,
}

#[derive(Debug, Clone)]
pub struct PickerResult {
    /// Display text for the result
    pub display: String,
    /// File path (for FindFiles) or file:line:col (for LiveGrep)
    pub location: String,
    /// Line number (for LiveGrep, 0 for FindFiles)
    pub line: usize,
    /// Column number (for LiveGrep, 0 for FindFiles)
    pub col: usize,
}

pub struct Picker {
    /// Current picker mode
    mode: PickerMode,
    /// Current search query
    query: String,
    /// All available results (unfiltered)
    all_results: Vec<PickerResult>,
    /// Filtered results based on query
    filtered_results: Vec<PickerResult>,
    /// Currently selected index in filtered_results
    selected_index: usize,
    /// Base directory for file search
    base_dir: PathBuf,
}

impl Picker {
    /// Creates a new file finder picker
    pub fn new_file_finder(base_dir: PathBuf) -> Self {
        let mut picker = Self {
            mode: PickerMode::FindFiles,
            query: String::new(),
            all_results: Vec::new(),
            filtered_results: Vec::new(),
            selected_index: 0,
            base_dir: base_dir.clone(),
        };

        // Load all files from base directory
        picker.all_results = Self::find_files_recursive(&base_dir);
        picker.filtered_results = picker.all_results.clone();

        picker
    }

    /// Creates a new live grep picker
    pub fn new_live_grep(base_dir: PathBuf) -> Self {
        Self {
            mode: PickerMode::LiveGrep,
            query: String::new(),
            all_results: Vec::new(),
            filtered_results: Vec::new(),
            selected_index: 0,
            base_dir,
        }
    }

    /// Recursively finds all files in a directory
    fn find_files_recursive(dir: &Path) -> Vec<PickerResult> {
        let mut results = Vec::new();

        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();

                // Skip hidden files and directories
                if let Some(name) = path.file_name() {
                    if name.to_string_lossy().starts_with('.') {
                        continue;
                    }
                }

                // Skip common ignore patterns
                if let Some(name) = path.file_name() {
                    let name_str = name.to_string_lossy();
                    if name_str == "target" || name_str == "node_modules" || name_str == ".git" {
                        continue;
                    }
                }

                if path.is_file() {
                    if let Ok(relative_path) = path.strip_prefix(dir) {
                        results.push(PickerResult {
                            display: relative_path.to_string_lossy().to_string(),
                            location: path.to_string_lossy().to_string(),
                            line: 0,
                            col: 0,
                        });
                    }
                } else if path.is_dir() {
                    results.extend(Self::find_files_recursive(&path));
                }
            }
        }

        results
    }

    /// Performs live grep using ripgrep or grep
    fn live_grep(query: &str, base_dir: &Path) -> Vec<PickerResult> {
        use std::process::Command;

        if query.is_empty() {
            return Vec::new();
        }

        // Try ripgrep first, fall back to grep
        let output = Command::new("rg")
            .args(&[
                "--line-number",
                "--column",
                "--no-heading",
                "--color=never",
                query,
            ])
            .current_dir(base_dir)
            .output();

        let output = match output {
            Ok(out) => out,
            Err(_) => {
                // Fall back to grep
                return Vec::new();
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut results = Vec::new();

        for line in stdout.lines() {
            // Parse rg output: file:line:col:content
            let parts: Vec<&str> = line.splitn(4, ':').collect();
            if parts.len() >= 4 {
                let file = parts[0];
                let line_num = parts[1].parse::<usize>().unwrap_or(1);
                let col_num = parts[2].parse::<usize>().unwrap_or(1);
                let content = parts[3];

                results.push(PickerResult {
                    display: format!("{}:{}:{}: {}", file, line_num, col_num, content.trim()),
                    location: file.to_string(),
                    line: line_num.saturating_sub(1), // Convert to 0-indexed
                    col: col_num.saturating_sub(1),
                });
            }
        }

        results
    }

    /// Updates the query and refreshes filtered results
    pub fn set_query(&mut self, query: String) {
        self.query = query;

        match self.mode {
            PickerMode::FindFiles => {
                // Simple substring matching (case-insensitive)
                let query_lower = self.query.to_lowercase();
                self.filtered_results = self
                    .all_results
                    .iter()
                    .filter(|r| r.display.to_lowercase().contains(&query_lower))
                    .cloned()
                    .collect();
            }
            PickerMode::LiveGrep => {
                // Perform live grep
                self.filtered_results = Self::live_grep(&self.query, &self.base_dir);
            }
        }

        // Reset selection to first result
        self.selected_index = 0;
    }

    /// Appends a character to the query
    pub fn append_query(&mut self, ch: char) {
        self.query.push(ch);
        self.set_query(self.query.clone());
    }

    /// Removes the last character from the query
    pub fn backspace_query(&mut self) {
        self.query.pop();
        self.set_query(self.query.clone());
    }

    /// Moves selection down
    pub fn move_down(&mut self) {
        if !self.filtered_results.is_empty() {
            self.selected_index = (self.selected_index + 1).min(self.filtered_results.len() - 1);
        }
    }

    /// Moves selection up
    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Gets the currently selected result
    pub fn selected_result(&self) -> Option<&PickerResult> {
        self.filtered_results.get(self.selected_index)
    }

    /// Gets the current query
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Gets filtered results
    pub fn filtered_results(&self) -> &[PickerResult] {
        &self.filtered_results
    }

    /// Gets selected index
    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    /// Gets picker mode
    pub fn mode(&self) -> &PickerMode {
        &self.mode
    }
}
