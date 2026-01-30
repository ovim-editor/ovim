use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub enum PickerMode {
    FindFiles,
    LiveGrep,
    Custom,
    Completion,
    LspLocations,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PickerField {
    Query,
    FileFilter,
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
    /// Character indices in `display` that matched the query
    pub match_positions: Vec<usize>,
    /// Matched content (for LiveGrep) — displayed separately from the location
    pub content: Option<String>,
}

pub struct Picker {
    /// Current picker mode
    mode: PickerMode,
    /// Current search query
    query: String,
    /// Cursor position in the query (byte offset)
    query_cursor: usize,
    /// File filter string (for FindFiles/LiveGrep modes)
    file_filter: String,
    /// Cursor position in the file filter (char offset)
    file_filter_cursor: usize,
    /// Which input field is currently active
    active_field: PickerField,
    /// All available results (unfiltered)
    all_results: Vec<PickerResult>,
    /// Filtered results based on query
    filtered_results: Vec<PickerResult>,
    /// Currently selected index in filtered_results
    selected_index: usize,
    /// Base directory for file search
    base_dir: PathBuf,
    /// Whether file loading is still in progress
    loading: bool,
    /// Whether file loading task has been spawned
    loading_spawned: bool,
    /// Whether filtering is pending (for debouncing)
    pending_filter: bool,
    /// The last query that was actually filtered
    last_filtered_query: String,
    /// The last file filter that was actually filtered
    last_filtered_file_filter: String,
}

impl Picker {
    /// Creates a new file finder picker
    /// Files are loaded asynchronously - use add_file_result() to populate
    pub fn new_file_finder(base_dir: PathBuf) -> Self {
        Self {
            mode: PickerMode::FindFiles,
            query: String::new(),
            query_cursor: 0,
            file_filter: String::new(),
            file_filter_cursor: 0,
            active_field: PickerField::Query,
            all_results: Vec::new(),
            filtered_results: Vec::new(),
            selected_index: 0,
            base_dir,
            loading: true,
            loading_spawned: false,
            pending_filter: false,
            last_filtered_query: String::new(),
            last_filtered_file_filter: String::new(),
        }
    }

    /// Creates a new live grep picker
    pub fn new_live_grep(base_dir: PathBuf) -> Self {
        Self {
            mode: PickerMode::LiveGrep,
            query: String::new(),
            query_cursor: 0,
            file_filter: String::new(),
            file_filter_cursor: 0,
            active_field: PickerField::Query,
            all_results: Vec::new(),
            filtered_results: Vec::new(),
            selected_index: 0,
            base_dir,
            loading: false,
            loading_spawned: false,
            pending_filter: false,
            last_filtered_query: String::new(),
            last_filtered_file_filter: String::new(),
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
                match_positions: Vec::new(),
                content: None,
            })
            .collect();

        Self {
            mode: PickerMode::Custom,
            query: String::new(),
            query_cursor: 0,
            file_filter: String::new(),
            file_filter_cursor: 0,
            active_field: PickerField::Query,
            all_results: results.clone(),
            filtered_results: results,
            selected_index: 0,
            base_dir,
            loading: false,
            loading_spawned: false,
            pending_filter: false,
            last_filtered_query: String::new(),
            last_filtered_file_filter: String::new(),
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
                match_positions: Vec::new(),
                content: None,
            })
            .collect();

        Self {
            mode: PickerMode::Completion,
            query: String::new(),
            query_cursor: 0,
            file_filter: String::new(),
            file_filter_cursor: 0,
            active_field: PickerField::Query,
            all_results: results.clone(),
            filtered_results: results,
            selected_index: 0,
            base_dir,
            loading: false,
            loading_spawned: false,
            pending_filter: false,
            last_filtered_query: String::new(),
            last_filtered_file_filter: String::new(),
        }
    }

    /// Creates a new LSP locations picker (for references, symbols, hierarchy, etc.)
    /// Items are display strings, and indices map to editor's LSP storage vectors
    pub fn new_lsp_locations(base_dir: PathBuf, items: Vec<String>) -> Self {
        let results: Vec<PickerResult> = items
            .into_iter()
            .enumerate()
            .map(|(idx, display)| PickerResult {
                display,
                location: idx.to_string(), // Index into editor's LSP storage vectors
                line: idx,
                col: 0,
                match_positions: Vec::new(),
                content: None,
            })
            .collect();

        Self {
            mode: PickerMode::LspLocations,
            query: String::new(),
            query_cursor: 0,
            file_filter: String::new(),
            file_filter_cursor: 0,
            active_field: PickerField::Query,
            all_results: results.clone(),
            filtered_results: results,
            selected_index: 0,
            base_dir,
            loading: false,
            loading_spawned: false,
            pending_filter: false,
            last_filtered_query: String::new(),
            last_filtered_file_filter: String::new(),
        }
    }

    /// Creates a new LSP locations picker with pre-built PickerResult items
    /// This preserves the actual file paths in location field for preview loading
    pub fn new_with_results(base_dir: PathBuf, results: Vec<PickerResult>) -> Self {
        Self {
            mode: PickerMode::LspLocations,
            query: String::new(),
            query_cursor: 0,
            file_filter: String::new(),
            file_filter_cursor: 0,
            active_field: PickerField::Query,
            all_results: results.clone(),
            filtered_results: results,
            selected_index: 0,
            base_dir,
            loading: false,
            loading_spawned: false,
            pending_filter: false,
            last_filtered_query: String::new(),
            last_filtered_file_filter: String::new(),
        }
    }

    /// Sets the prompt for the picker
    pub fn set_prompt(&mut self, _prompt: String) {
        // Prompt display is handled by the UI layer
        // This is a placeholder for API compatibility
    }

    /// Simple glob pattern matching supporting `*` and `?` wildcards.
    /// Matches against the given string case-insensitively.
    #[cfg(test)]
    fn glob_match(pattern: &str, text: &str) -> bool {
        let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
        let text: Vec<char> = text.to_lowercase().chars().collect();
        Self::glob_match_inner(&pattern, &text)
    }

    #[cfg(test)]
    fn glob_match_inner(pattern: &[char], text: &[char]) -> bool {
        let mut pi = 0;
        let mut ti = 0;
        let mut star_pi = None;
        let mut star_ti = 0;

        while ti < text.len() {
            if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == text[ti]) {
                pi += 1;
                ti += 1;
            } else if pi < pattern.len() && pattern[pi] == '*' {
                star_pi = Some(pi);
                star_ti = ti;
                pi += 1;
            } else if let Some(sp) = star_pi {
                pi = sp + 1;
                star_ti += 1;
                ti = star_ti;
            } else {
                return false;
            }
        }

        while pi < pattern.len() && pattern[pi] == '*' {
            pi += 1;
        }

        pi == pattern.len()
    }

    /// Checks if a result's file path matches the file filter.
    /// The filter is space-separated tokens; all must match.
    /// Tokens containing `*` or `?` are glob-matched against the basename
    /// (or full path if token contains `/`). Otherwise, substring match (case-insensitive).
    #[cfg(test)]
    fn matches_file_filter(filter: &str, path: &str) -> bool {
        if filter.is_empty() {
            return true;
        }

        let tokens: Vec<&str> = filter.split_whitespace().collect();
        if tokens.is_empty() {
            return true;
        }

        let basename = std::path::Path::new(path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string());

        let path_lower = path.to_lowercase();
        let basename_lower = basename.to_lowercase();

        for token in &tokens {
            let is_glob = token.contains('*') || token.contains('?');
            let has_slash = token.contains('/');

            if is_glob {
                let target = if has_slash { &path_lower } else { &basename_lower };
                if !Self::glob_match(token, target) {
                    return false;
                }
            } else {
                let token_lower = token.to_lowercase();
                let target = if has_slash { &path_lower } else { &basename_lower };
                if !target.contains(&token_lower) {
                    return false;
                }
            }
        }

        true
    }

    /// Performs live grep using ripgrep or grep
    fn live_grep(query: &str, base_dir: &Path, file_filter: &str) -> Vec<PickerResult> {
        use std::process::Command;

        if query.is_empty() {
            return Vec::new();
        }

        let mut args = vec![
            "--line-number".to_string(),
            "--column".to_string(),
            "--no-heading".to_string(),
            "--color=never".to_string(),
        ];

        // Add glob filters from file_filter
        for token in file_filter.split_whitespace() {
            if !token.is_empty() {
                let is_glob = token.contains('*') || token.contains('?');
                args.push("--glob".to_string());
                if is_glob {
                    args.push(token.to_string());
                } else {
                    // Wrap plain substring as glob: "editor" -> "*editor*"
                    args.push(format!("*{}*", token));
                }
            }
        }

        args.push(query.to_string());

        // Try ripgrep first, fall back to grep
        let output = Command::new("rg")
            .args(&args)
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
                    display: format!("{}:{}:{}", file, line_num, col_num),
                    location: abs_path,
                    line: line_num.saturating_sub(1), // Convert to 0-indexed
                    col: col_num.saturating_sub(1),
                    match_positions: Vec::new(),
                    content: Some(content.trim().to_string()),
                });
            }
        }

        results
    }

    /// Fuzzy match that also returns the matched character positions in the target
    fn fuzzy_match_with_positions(query: &str, target: &str) -> Option<(i32, Vec<usize>)> {
        if query.is_empty() {
            return Some((0, Vec::new()));
        }

        let query_lower = query.to_lowercase();
        let target_lower = target.to_lowercase();

        let query_chars: Vec<char> = query_lower.chars().collect();
        let target_chars: Vec<char> = target_lower.chars().collect();

        if query_chars.is_empty() {
            return Some((0, Vec::new()));
        }

        // Prefer exact substring matches — find the best occurrence
        if let Some(result) = Self::exact_substring_match(&query_chars, &target_chars) {
            return Some(result);
        }

        // Fall back to fuzzy matching
        let mut query_idx = 0;
        let mut target_idx = 0;
        let mut score: i32 = 0;
        let mut consecutive_matches = 0;
        let mut last_match_idx: Option<usize> = None;
        let mut positions = Vec::with_capacity(query_chars.len());

        while query_idx < query_chars.len() && target_idx < target_chars.len() {
            if query_chars[query_idx] == target_chars[target_idx] {
                // Base score for match
                score += 1;

                // Bonus for consecutive matches
                if let Some(last_idx) = last_match_idx {
                    if target_idx == last_idx + 1 {
                        consecutive_matches += 1;
                        score += consecutive_matches * 5;
                    } else {
                        consecutive_matches = 0;
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
                    if prev_char == '/' || prev_char == '_' || prev_char == '-' || prev_char == ' '
                    {
                        score += 8;
                    }
                }

                positions.push(target_idx);
                last_match_idx = Some(target_idx);
                query_idx += 1;
            }
            target_idx += 1;
        }

        if query_idx == query_chars.len() {
            score += 100 - (target_chars.len() as i32).min(100);
            Some((score, positions))
        } else {
            None
        }
    }

    /// Find the best exact substring match, preferring word boundaries and start of string.
    /// Returns a high score so exact matches always rank above fuzzy matches.
    fn exact_substring_match(
        query_chars: &[char],
        target_chars: &[char],
    ) -> Option<(i32, Vec<usize>)> {
        let query_len = query_chars.len();
        if query_len == 0 || target_chars.len() < query_len {
            return None;
        }

        let mut best: Option<(i32, usize)> = None;

        for start in 0..=(target_chars.len() - query_len) {
            let matches = (0..query_len).all(|i| target_chars[start + i] == query_chars[i]);
            if !matches {
                continue;
            }

            // Base: large bonus for being an exact substring
            let mut score: i32 = 200;

            // Bonus for matching at start of target
            if start == 0 {
                score += 20;
            }

            // Bonus for matching at a word boundary
            if start > 0 {
                let prev = target_chars[start - 1];
                if prev == '/' || prev == '_' || prev == '-' || prev == '.' || prev == ' ' {
                    score += 15;
                }
            }

            // Prefer shorter targets (more specific match)
            score += 100 - (target_chars.len() as i32).min(100);

            match best {
                Some((best_score, _)) if score <= best_score => {}
                _ => best = Some((score, start)),
            }
        }

        best.map(|(score, start)| {
            let positions: Vec<usize> = (start..start + query_len).collect();
            (score, positions)
        })
    }

    /// Filename-preferential fuzzy scoring
    /// Splits query on whitespace (all tokens must match), prefers filename matches.
    /// Returns (total_score, matched_positions_in_full_path).
    fn fuzzy_score(query: &str, target: &str) -> Option<(i32, Vec<usize>)> {
        if query.is_empty() {
            return Some((0, Vec::new()));
        }

        let tokens: Vec<&str> = query.split_whitespace().collect();
        if tokens.is_empty() {
            return Some((0, Vec::new()));
        }

        // Extract filename and its char offset in the full path
        let filename_start = target.rfind('/').map(|i| i + 1).unwrap_or(0);
        let filename = &target[filename_start..];
        // Convert byte offset to char offset
        let filename_char_offset = target[..filename_start].chars().count();

        let mut total_score: i32 = 0;
        let mut all_positions = Vec::new();

        for token in &tokens {
            // Try matching against filename first (with bonus)
            if let Some((score, positions)) = Self::fuzzy_match_with_positions(token, filename) {
                total_score += score + 50; // Filename match bonus
                // Offset positions to full-path indices
                for pos in positions {
                    all_positions.push(pos + filename_char_offset);
                }
            } else if let Some((score, positions)) = Self::fuzzy_match_with_positions(token, target)
            {
                // Fall back to full path match (no bonus)
                total_score += score;
                all_positions.extend(positions);
            } else {
                // Token didn't match at all — entire query fails
                return None;
            }
        }

        Some((total_score, all_positions))
    }

    /// Returns the total number of results (before filtering)
    pub fn all_results_count(&self) -> usize {
        self.all_results.len()
    }

    /// Updates the query and refreshes filtered results
    /// Note: For incremental typing, use mark_filter_pending() and apply_pending_filter()
    /// to debounce the expensive filtering operation
    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.apply_filter_internal();
    }

    /// Internal filter logic - called by both set_query and apply_pending_filter
    fn apply_filter_internal(&mut self) {
        match self.mode {
            PickerMode::FindFiles => {
                // No file_filter in FindFiles mode — query is the sole filter
                let mut scored_results: Vec<(PickerResult, i32, Vec<usize>)> = self
                    .all_results
                    .iter()
                    .filter_map(|r| {
                        Self::fuzzy_score(&self.query, &r.display).map(|(score, positions)| {
                            (r.clone(), score, positions)
                        })
                    })
                    .collect();

                scored_results.sort_by(|a, b| b.1.cmp(&a.1));

                self.filtered_results = scored_results
                    .into_iter()
                    .map(|(mut result, _score, positions)| {
                        result.match_positions = positions;
                        result
                    })
                    .collect();
            }
            PickerMode::Custom | PickerMode::Completion | PickerMode::LspLocations => {
                let mut scored_results: Vec<(PickerResult, i32, Vec<usize>)> = self
                    .all_results
                    .iter()
                    .filter_map(|r| {
                        Self::fuzzy_score(&self.query, &r.display).map(|(score, positions)| {
                            (r.clone(), score, positions)
                        })
                    })
                    .collect();

                scored_results.sort_by(|a, b| b.1.cmp(&a.1));

                self.filtered_results = scored_results
                    .into_iter()
                    .map(|(mut result, _score, positions)| {
                        result.match_positions = positions;
                        result
                    })
                    .collect();
            }
            PickerMode::LiveGrep => {
                let grep_results = Self::live_grep(&self.query, &self.base_dir, &self.file_filter);
                self.filtered_results = grep_results;
            }
        }

        // Reset selection to first result
        self.selected_index = 0;
        // Track what query was filtered
        self.last_filtered_query = self.query.clone();
        self.last_filtered_file_filter = self.file_filter.clone();
        self.pending_filter = false;
    }

    /// Marks that filtering is pending (query changed but not yet filtered)
    pub fn mark_filter_pending(&mut self) {
        self.pending_filter = true;
    }

    /// Returns true if there's a pending filter operation
    pub fn has_pending_filter(&self) -> bool {
        self.pending_filter
    }

    /// Applies the pending filter if query has changed since last filter
    /// Call this from the event loop after debounce period
    pub fn apply_pending_filter(&mut self) {
        if self.pending_filter {
            self.apply_filter_internal();
        }
    }

    /// Returns mutable references to the active field's text and cursor
    fn active_field_mut(&mut self) -> (&mut String, &mut usize) {
        match self.active_field {
            PickerField::Query => (&mut self.query, &mut self.query_cursor),
            PickerField::FileFilter => (&mut self.file_filter, &mut self.file_filter_cursor),
        }
    }

    /// Converts character position to byte position in a string
    fn char_pos_to_byte_pos_in(s: &str, char_pos: usize) -> usize {
        s.char_indices()
            .nth(char_pos)
            .map(|(byte_pos, _)| byte_pos)
            .unwrap_or(s.len())
    }

    /// Inserts a character at the cursor position in the active field
    pub fn insert_char(&mut self, ch: char) {
        let (text, cursor) = self.active_field_mut();
        let byte_pos = Self::char_pos_to_byte_pos_in(text, *cursor);
        text.insert(byte_pos, ch);
        *cursor += 1;
        self.mark_filter_pending();
    }

    /// Inserts a string at the cursor position in the active field
    pub fn insert_text(&mut self, s: &str) {
        let (text, cursor) = self.active_field_mut();
        let byte_pos = Self::char_pos_to_byte_pos_in(text, *cursor);
        text.insert_str(byte_pos, s);
        *cursor += s.chars().count();
        self.mark_filter_pending();
    }

    /// Appends a character to the query (legacy method, inserts at cursor)
    pub fn append_query(&mut self, ch: char) {
        self.insert_char(ch);
    }

    /// Removes the character before the cursor in the active field
    pub fn backspace_query(&mut self) {
        let (text, cursor) = self.active_field_mut();
        if *cursor > 0 {
            let byte_pos = Self::char_pos_to_byte_pos_in(text, *cursor - 1);
            text.remove(byte_pos);
            *cursor -= 1;
        } else {
            return;
        }
        self.mark_filter_pending();
    }

    /// Removes the character at the cursor in the active field (delete key)
    pub fn delete_char(&mut self) {
        let (text, cursor) = self.active_field_mut();
        let char_len = text.chars().count();
        if *cursor < char_len {
            let byte_pos = Self::char_pos_to_byte_pos_in(text, *cursor);
            text.remove(byte_pos);
        } else {
            return;
        }
        self.mark_filter_pending();
    }

    /// Moves cursor left in the active field
    pub fn move_cursor_left(&mut self) {
        let (_text, cursor) = self.active_field_mut();
        if *cursor > 0 {
            *cursor -= 1;
        }
    }

    /// Moves cursor right in the active field
    pub fn move_cursor_right(&mut self) {
        let (text, cursor) = self.active_field_mut();
        let char_len = text.chars().count();
        if *cursor < char_len {
            *cursor += 1;
        }
    }

    /// Moves cursor to the beginning of the active field
    pub fn move_cursor_home(&mut self) {
        let (_text, cursor) = self.active_field_mut();
        *cursor = 0;
    }

    /// Moves cursor to the end of the active field
    pub fn move_cursor_end(&mut self) {
        let (text, cursor) = self.active_field_mut();
        *cursor = text.chars().count();
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

    /// Gets the base directory for file operations
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Returns true if this picker mode supports the file filter field
    pub fn has_file_filter(&self) -> bool {
        matches!(self.mode, PickerMode::LiveGrep)
    }

    /// Switches the active input field (only for modes with file filter)
    pub fn toggle_field(&mut self) {
        if self.has_file_filter() {
            self.active_field = match self.active_field {
                PickerField::Query => PickerField::FileFilter,
                PickerField::FileFilter => PickerField::Query,
            };
        }
    }

    /// Gets the current file filter string
    pub fn file_filter(&self) -> &str {
        &self.file_filter
    }

    /// Gets the file filter cursor position
    pub fn file_filter_cursor(&self) -> usize {
        self.file_filter_cursor
    }

    /// Gets the currently active field
    pub fn active_field(&self) -> PickerField {
        self.active_field
    }

    /// Adds a file result (for incremental loading)
    pub fn add_file_result(&mut self, result: PickerResult) {
        self.all_results.push(result.clone());

        // If query is empty, add to filtered results too
        if self.query.is_empty() {
            self.filtered_results.push(result);
        } else {
            // Just check if it matches and append - don't try to maintain sort order
            // The filter will be re-applied with proper sorting when the user stops typing
            // This avoids O(n²) re-scoring of all existing results on each file addition
            if Self::fuzzy_score(&self.query, &result.display).is_some() {
                self.filtered_results.push(result);
                // Mark that results need re-sorting (will happen on next filter apply)
                self.pending_filter = true;
            }
        }
    }

    /// Marks file loading as complete
    pub fn finish_loading(&mut self) {
        self.loading = false;
    }

    /// Returns whether files are still being loaded
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// Returns whether file loading should be spawned
    pub fn should_spawn_file_loading(&self) -> bool {
        self.mode == PickerMode::FindFiles && self.loading && !self.loading_spawned
    }

    /// Marks file loading as spawned
    pub fn mark_loading_spawned(&mut self) {
        self.loading_spawned = true;
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
            let chars: Vec<char> = path.chars().collect();
            let start_len = (max_len - 3) / 2;
            let end_len = max_len - 3 - start_len;
            let start: String = chars.iter().take(start_len).collect();
            let end: String = chars
                .iter()
                .skip(chars.len().saturating_sub(end_len))
                .collect();
            return format!("{}...{}", start, end);
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
            let chars: Vec<char> = last.chars().collect();
            let skip_count = chars.len().saturating_sub(available);
            let suffix: String = chars.iter().skip(skip_count).collect();
            return format!("...{}", suffix);
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
                    let current_len = current_len + needed;
                    let _ = current_len; // Suppress warning for now
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_glob_match_star() {
        assert!(Picker::glob_match("*.rs", "main.rs"));
        assert!(Picker::glob_match("*.rs", "MAIN.RS"));
        assert!(!Picker::glob_match("*.rs", "main.ts"));
        assert!(Picker::glob_match("src/*", "src/lib.rs"));
        assert!(Picker::glob_match("*test*", "my_test_file.rs"));
    }

    #[test]
    fn test_glob_match_question() {
        assert!(Picker::glob_match("?.rs", "a.rs"));
        assert!(!Picker::glob_match("?.rs", "ab.rs"));
        assert!(Picker::glob_match("??.rs", "ab.rs"));
    }

    #[test]
    fn test_glob_match_combined() {
        assert!(Picker::glob_match("*_test.?s", "my_test.rs"));
        assert!(Picker::glob_match("*_test.?s", "my_test.ts"));
        assert!(!Picker::glob_match("*_test.?s", "my_test.css"));
    }

    #[test]
    fn test_matches_file_filter_empty() {
        assert!(Picker::matches_file_filter("", "src/main.rs"));
        assert!(Picker::matches_file_filter("   ", "src/main.rs"));
    }

    #[test]
    fn test_matches_file_filter_substring() {
        assert!(Picker::matches_file_filter("mod", "src/mod.rs"));
        assert!(Picker::matches_file_filter("mod", "src/editor/mod.rs"));
        assert!(!Picker::matches_file_filter("xyz", "src/main.rs"));
    }

    #[test]
    fn test_matches_file_filter_glob() {
        assert!(Picker::matches_file_filter("*.rs", "src/main.rs"));
        assert!(!Picker::matches_file_filter("*.ts", "src/main.rs"));
    }

    #[test]
    fn test_matches_file_filter_multiple_tokens() {
        // All tokens must match
        assert!(Picker::matches_file_filter("*.rs mod", "mod.rs"));
        assert!(!Picker::matches_file_filter("*.rs xyz", "mod.rs"));
    }

    #[test]
    fn test_matches_file_filter_path_token() {
        // Token with `/` matches against full path
        assert!(Picker::matches_file_filter("src/", "src/main.rs"));
        assert!(!Picker::matches_file_filter("src/", "lib/main.rs"));
    }

    #[test]
    fn test_toggle_field() {
        let mut picker = Picker::new_live_grep(PathBuf::from("."));
        assert_eq!(picker.active_field(), PickerField::Query);

        picker.toggle_field();
        assert_eq!(picker.active_field(), PickerField::FileFilter);

        picker.toggle_field();
        assert_eq!(picker.active_field(), PickerField::Query);
    }

    #[test]
    fn test_toggle_field_no_op_for_find_files() {
        let mut picker = Picker::new_file_finder(PathBuf::from("."));
        assert_eq!(picker.active_field(), PickerField::Query);

        picker.toggle_field();
        assert_eq!(picker.active_field(), PickerField::Query); // No change
    }

    #[test]
    fn test_toggle_field_no_op_for_custom() {
        let mut picker = Picker::new_custom(PathBuf::from("."), vec!["a".into(), "b".into()]);
        assert_eq!(picker.active_field(), PickerField::Query);

        picker.toggle_field();
        assert_eq!(picker.active_field(), PickerField::Query); // No change
    }

    #[test]
    fn test_has_file_filter() {
        assert!(!Picker::new_file_finder(PathBuf::from(".")).has_file_filter());
        assert!(Picker::new_live_grep(PathBuf::from(".")).has_file_filter());
        assert!(!Picker::new_custom(PathBuf::from("."), vec![]).has_file_filter());
        assert!(!Picker::new_completion(PathBuf::from("."), vec![]).has_file_filter());
        assert!(!Picker::new_lsp_locations(PathBuf::from("."), vec![]).has_file_filter());
    }

    #[test]
    fn test_active_field_mut_delegates_to_query() {
        let mut picker = Picker::new_file_finder(PathBuf::from("."));
        // Active field is Query by default
        picker.insert_char('a');
        picker.insert_char('b');
        assert_eq!(picker.query(), "ab");
        assert_eq!(picker.file_filter(), "");
    }

    #[test]
    fn test_active_field_mut_delegates_to_filter() {
        let mut picker = Picker::new_live_grep(PathBuf::from("."));
        picker.toggle_field();

        picker.insert_char('*');
        picker.insert_char('.');
        picker.insert_char('r');
        picker.insert_char('s');
        assert_eq!(picker.file_filter(), "*.rs");
        assert_eq!(picker.query(), "");
    }

    #[test]
    fn test_backspace_in_filter_field() {
        let mut picker = Picker::new_live_grep(PathBuf::from("."));
        picker.toggle_field();
        picker.insert_char('a');
        picker.insert_char('b');
        picker.backspace_query();
        assert_eq!(picker.file_filter(), "a");
        assert_eq!(picker.file_filter_cursor(), 1);
    }

    #[test]
    fn test_insert_text_into_query() {
        let mut picker = Picker::new_file_finder(PathBuf::from("."));
        picker.insert_text("hello");
        assert_eq!(picker.query(), "hello");
        assert_eq!(picker.query_cursor(), 5);

        // Insert more text at cursor
        picker.insert_text(" world");
        assert_eq!(picker.query(), "hello world");
        assert_eq!(picker.query_cursor(), 11);
    }

    #[test]
    fn test_insert_text_at_cursor_midpoint() {
        let mut picker = Picker::new_file_finder(PathBuf::from("."));
        picker.insert_text("ac");
        picker.move_cursor_left(); // cursor before 'c'
        picker.insert_text("b");
        assert_eq!(picker.query(), "abc");
        assert_eq!(picker.query_cursor(), 2);
    }

    #[test]
    fn test_insert_text_into_file_filter() {
        let mut picker = Picker::new_live_grep(PathBuf::from("."));
        picker.toggle_field();
        picker.insert_text("*.rs");
        assert_eq!(picker.file_filter(), "*.rs");
        assert_eq!(picker.file_filter_cursor(), 4);
        assert_eq!(picker.query(), "");
    }

    #[test]
    fn test_cursor_movement_in_filter_field() {
        let mut picker = Picker::new_live_grep(PathBuf::from("."));
        picker.toggle_field();
        picker.insert_char('a');
        picker.insert_char('b');
        picker.insert_char('c');
        assert_eq!(picker.file_filter_cursor(), 3);

        picker.move_cursor_left();
        assert_eq!(picker.file_filter_cursor(), 2);

        picker.move_cursor_home();
        assert_eq!(picker.file_filter_cursor(), 0);

        picker.move_cursor_end();
        assert_eq!(picker.file_filter_cursor(), 3);
    }
}
