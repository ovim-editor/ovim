use std::path::{Path, PathBuf};
use ignore::WalkBuilder;

#[derive(Debug, Clone, PartialEq)]
pub enum PickerMode {
    FindFiles,
    LiveGrep,
    Custom,
    Completion,
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
    /// Cursor position in the query (byte offset)
    query_cursor: usize,
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
            query_cursor: 0,
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
            query_cursor: 0,
            all_results: Vec::new(),
            filtered_results: Vec::new(),
            selected_index: 0,
            base_dir,
        }
    }

    /// Creates a new picker with custom items
    pub fn new_custom(base_dir: PathBuf, items: Vec<String>) -> Self {
        let results: Vec<PickerResult> = items
            .into_iter()
            .enumerate()
            .map(|(idx, display)| PickerResult {
                display,
                location: idx.to_string(), // Use index as location identifier
                line: idx,
                col: 0,
            })
            .collect();

        Self {
            mode: PickerMode::Custom,
            query: String::new(),
            query_cursor: 0,
            all_results: results.clone(),
            filtered_results: results,
            selected_index: 0,
            base_dir,
        }
    }

    /// Creates a new completion picker with custom items
    pub fn new_completion(base_dir: PathBuf, items: Vec<String>) -> Self {
        let results: Vec<PickerResult> = items
            .into_iter()
            .enumerate()
            .map(|(idx, display)| PickerResult {
                display,
                location: idx.to_string(), // Use index as location identifier
                line: idx,
                col: 0,
            })
            .collect();

        Self {
            mode: PickerMode::Completion,
            query: String::new(),
            query_cursor: 0,
            all_results: results.clone(),
            filtered_results: results,
            selected_index: 0,
            base_dir,
        }
    }

    /// Sets the prompt for the picker
    pub fn set_prompt(&mut self, _prompt: String) {
        // Prompt display is handled by the UI layer
        // This is a placeholder for API compatibility
    }

    /// Recursively finds all files in a directory, respecting .gitignore
    fn find_files_recursive(dir: &Path) -> Vec<PickerResult> {
        let mut results = Vec::new();

        // Check if directory exists first
        if !dir.exists() || !dir.is_dir() {
            return results;
        }

        // Use ignore crate's WalkBuilder which respects .gitignore, .ignore, and other ignore files
        let walker = WalkBuilder::new(dir)
            .hidden(false)  // Don't automatically skip hidden files (let gitignore handle it)
            .git_ignore(true)  // Respect .gitignore files
            .git_global(true)  // Respect global gitignore
            .git_exclude(true)  // Respect .git/info/exclude
            .build();

        for entry in walker.filter_map(|e| e.ok()) {
            let path = entry.path();

            if path.is_file() {
                if let Ok(relative_path) = path.strip_prefix(dir) {
                    let display_path = relative_path.to_string_lossy().to_string();
                    results.push(PickerResult {
                        display: display_path,
                        location: path.to_string_lossy().to_string(),
                        line: 0,
                        col: 0,
                    });
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

                // Convert relative path to absolute path
                let abs_path = if std::path::Path::new(file).is_absolute() {
                    file.to_string()
                } else {
                    base_dir.join(file).to_string_lossy().to_string()
                };

                results.push(PickerResult {
                    display: format!("{}:{}:{}: {}", file, line_num, col_num, content.trim()),
                    location: abs_path,
                    line: line_num.saturating_sub(1), // Convert to 0-indexed
                    col: col_num.saturating_sub(1),
                });
            }
        }

        results
    }

    /// Fuzzy match scoring function
    /// Returns Some(score) if match succeeds, None otherwise
    /// Higher scores are better matches
    fn fuzzy_match(query: &str, target: &str) -> Option<i32> {
        if query.is_empty() {
            return Some(0);
        }

        let query_lower = query.to_lowercase();
        let target_lower = target.to_lowercase();

        let query_chars: Vec<char> = query_lower.chars().collect();
        let target_chars: Vec<char> = target_lower.chars().collect();

        if query_chars.is_empty() {
            return Some(0);
        }

        let mut query_idx = 0;
        let mut target_idx = 0;
        let mut score: i32 = 0;
        let mut consecutive_matches = 0;
        let mut last_match_idx: Option<usize> = None;

        while query_idx < query_chars.len() && target_idx < target_chars.len() {
            if query_chars[query_idx] == target_chars[target_idx] {
                // Base score for match
                score += 1;

                // Bonus for consecutive matches
                if let Some(last_idx) = last_match_idx {
                    if target_idx == last_idx + 1 {
                        consecutive_matches += 1;
                        score += consecutive_matches * 5; // Increasing bonus for longer sequences
                    } else {
                        consecutive_matches = 0;
                        // Penalty for gaps (but capped to not penalize too much)
                        let gap = target_idx - last_idx - 1;
                        score -= (gap as i32).min(3);
                    }
                } else {
                    consecutive_matches = 0;
                }

                // Bonus for matching at start of target
                if target_idx == 0 {
                    score += 10;
                }

                // Bonus for matching after path separator or start of word
                if target_idx > 0 {
                    let prev_char = target_chars[target_idx - 1];
                    if prev_char == '/' || prev_char == '_' || prev_char == '-' || prev_char == ' ' {
                        score += 8;
                    }
                }

                // Bonus for case match
                if query.chars().nth(query_idx) == target.chars().nth(target_idx) {
                    score += 2;
                }

                last_match_idx = Some(target_idx);
                query_idx += 1;
            }
            target_idx += 1;
        }

        // Check if we matched all query characters
        if query_idx == query_chars.len() {
            // Bonus for shorter targets (more specific matches)
            score += 100 - (target_chars.len() as i32).min(100);
            Some(score)
        } else {
            None
        }
    }

    /// Updates the query and refreshes filtered results
    pub fn set_query(&mut self, query: String) {
        self.query = query;

        match self.mode {
            PickerMode::FindFiles => {
                // Fuzzy matching with scoring
                let mut scored_results: Vec<(PickerResult, i32)> = self
                    .all_results
                    .iter()
                    .filter_map(|r| {
                        Self::fuzzy_match(&self.query, &r.display)
                            .map(|score| (r.clone(), score))
                    })
                    .collect();

                // Sort by score (descending)
                scored_results.sort_by(|a, b| b.1.cmp(&a.1));

                self.filtered_results = scored_results
                    .into_iter()
                    .map(|(result, _score)| result)
                    .collect();
            }
            PickerMode::LiveGrep => {
                // Perform live grep, then apply fuzzy filtering on results
                let grep_results = Self::live_grep(&self.query, &self.base_dir);

                // For LiveGrep, we still want to show the grep results as-is
                // since the query is used for the actual grep search
                self.filtered_results = grep_results;
            }
            PickerMode::Custom | PickerMode::Completion => {
                // Fuzzy matching for custom mode and completion
                let mut scored_results: Vec<(PickerResult, i32)> = self
                    .all_results
                    .iter()
                    .filter_map(|r| {
                        Self::fuzzy_match(&self.query, &r.display)
                            .map(|score| (r.clone(), score))
                    })
                    .collect();

                // Sort by score (descending)
                scored_results.sort_by(|a, b| b.1.cmp(&a.1));

                self.filtered_results = scored_results
                    .into_iter()
                    .map(|(result, _score)| result)
                    .collect();
            }
        }

        // Reset selection to first result
        self.selected_index = 0;
    }

    /// Inserts a character at the cursor position
    pub fn insert_char(&mut self, ch: char) {
        // Insert character at cursor position
        let byte_pos = self.char_pos_to_byte_pos(self.query_cursor);
        self.query.insert(byte_pos, ch);
        // Move cursor forward
        self.query_cursor += 1;
        self.set_query(self.query.clone());
    }

    /// Appends a character to the query (legacy method, inserts at cursor)
    pub fn append_query(&mut self, ch: char) {
        self.insert_char(ch);
    }

    /// Removes the character before the cursor
    pub fn backspace_query(&mut self) {
        if self.query_cursor > 0 {
            let byte_pos = self.char_pos_to_byte_pos(self.query_cursor - 1);
            self.query.remove(byte_pos);
            self.query_cursor -= 1;
            self.set_query(self.query.clone());
        }
    }

    /// Removes the character at the cursor (delete key)
    pub fn delete_char(&mut self) {
        let char_len = self.query.chars().count();
        if self.query_cursor < char_len {
            let byte_pos = self.char_pos_to_byte_pos(self.query_cursor);
            self.query.remove(byte_pos);
            self.set_query(self.query.clone());
        }
    }

    /// Moves cursor left in the query
    pub fn move_cursor_left(&mut self) {
        if self.query_cursor > 0 {
            self.query_cursor -= 1;
        }
    }

    /// Moves cursor right in the query
    pub fn move_cursor_right(&mut self) {
        let char_len = self.query.chars().count();
        if self.query_cursor < char_len {
            self.query_cursor += 1;
        }
    }

    /// Moves cursor to the beginning of the query
    pub fn move_cursor_home(&mut self) {
        self.query_cursor = 0;
    }

    /// Moves cursor to the end of the query
    pub fn move_cursor_end(&mut self) {
        self.query_cursor = self.query.chars().count();
    }

    /// Converts character position to byte position
    fn char_pos_to_byte_pos(&self, char_pos: usize) -> usize {
        self.query
            .char_indices()
            .nth(char_pos)
            .map(|(byte_pos, _)| byte_pos)
            .unwrap_or(self.query.len())
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

    /// Gets the query cursor position (in characters, not bytes)
    pub fn query_cursor(&self) -> usize {
        self.query_cursor
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

    /// Truncates a path in the middle if it's too long
    /// Prioritizes showing the filename and immediate parent directories
    /// Examples:
    ///   "src/buffer/mod.rs" -> "src/buffer/mod.rs" (fits)
    ///   "src/buffer/cursor/position.rs" -> "src/.../position.rs" (truncated)
    ///   "backend/services/user-state/users.ts" -> "backend/.../user-state/users.ts"
    pub fn truncate_path(path: &str, max_len: usize) -> String {
        if path.len() <= max_len {
            return path.to_string();
        }

        // Split by path separator
        let parts: Vec<&str> = path.split('/').collect();

        if parts.is_empty() {
            return path.to_string();
        }

        if parts.len() == 1 {
            // Single component, truncate with ellipsis in middle
            if max_len < 4 {
                return "...".to_string();
            }
            let start_len = (max_len - 3) / 2;
            let end_len = max_len - 3 - start_len;
            return format!("{}...{}", &path[..start_len], &path[path.len() - end_len..]);
        }

        // Always keep the last component (filename)
        let last = parts[parts.len() - 1];

        // Reserve space for ".../" (4 chars) and the last component
        let reserved = 4 + last.len();

        if reserved >= max_len {
            // Not enough space, truncate the filename itself
            if max_len < 4 {
                return "...".to_string();
            }
            let available = max_len - 3;
            return format!("...{}", &last[last.len().saturating_sub(available)..]);
        }

        // Try to include as many parts from the end as possible
        let mut included_parts = vec![last];
        let mut current_len = last.len();

        // Work backwards from the second-to-last component
        for i in (0..parts.len() - 1).rev() {
            let part = parts[i];
            let needed = part.len() + 1; // +1 for the separator

            // Check if adding this part would fit
            if current_len + needed + 4 <= max_len {
                // We have room for this part plus ".../"
                included_parts.insert(0, part);
                current_len += needed;
            } else {
                // Can't fit this part, but check if we can fit it without the leading parts
                if i > 0 && current_len + needed + 4 <= max_len {
                    included_parts.insert(0, part);
                    current_len += needed;
                }
                break;
            }
        }

        // Build the result
        if included_parts.len() == parts.len() {
            // We managed to fit everything (shouldn't happen since path.len() > max_len)
            return path.to_string();
        }

        // Check if we're missing the first part
        if included_parts.len() < parts.len() {
            // Add ellipsis at the beginning
            if included_parts[0] != parts[0] {
                // We're not showing from the start, add ".../"
                let mut result = String::from(".../");
                result.push_str(&included_parts.join("/"));
                return result;
            }
        }

        included_parts.join("/")
    }
}
