mod backend;
mod filter;
mod fuzzy_backend;
mod grep_backend;
mod nucleo_backend;
mod result;

use backend::PickerBackend;
use fuzzy_backend::FuzzyListKind;
use grep_backend::GrepState;
use nucleo_backend::NucleoState;
pub use result::{PickerAction, PickerField, PickerMode, PickerResult};

use super::fuzzy;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct Picker {
    /// Current search query
    query: String,
    /// Cursor position in the query (byte offset)
    query_cursor: usize,
    /// File filter string (for LiveGrep mode)
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
    /// Whether filtering is pending (for debouncing)
    pending_filter: bool,
    /// The last query that was actually filtered
    last_filtered_query: String,
    /// The last file filter that was actually filtered
    last_filtered_file_filter: String,
    /// Receiver for streaming grep results (LiveGrep mode)
    grep_rx: Option<mpsc::Receiver<PickerResult>>,
    /// Cancel flag for the current grep search
    grep_cancel: Option<Arc<AtomicBool>>,
    /// The query that was last sent to the grep search
    last_grep_query: String,
    /// Old results are stale and should be cleared when first new result arrives
    grep_stale: bool,
    /// Whether file loading is still in progress (grep or nucleo)
    loading: bool,
    /// Typed backend owning mode-specific state
    backend: PickerBackend,
}

impl Picker {
    /// Creates a new file finder picker
    /// Files are loaded asynchronously - use add_file_result() to populate
    /// Uses nucleo for parallel background fuzzy matching.
    pub fn new_file_finder(base_dir: PathBuf) -> Self {
        Self {
            query: String::new(),
            query_cursor: 0,
            file_filter: String::new(),
            file_filter_cursor: 0,
            active_field: PickerField::Query,
            all_results: Vec::new(),
            filtered_results: Vec::new(),
            selected_index: 0,
            base_dir,
            pending_filter: false,
            last_filtered_query: String::new(),
            last_filtered_file_filter: String::new(),
            grep_rx: None,
            grep_cancel: None,
            last_grep_query: String::new(),
            grep_stale: false,
            loading: true,
            backend: PickerBackend::Nucleo(NucleoState::new()),
        }
    }

    /// Creates a new live grep picker
    pub fn new_live_grep(base_dir: PathBuf) -> Self {
        Self {
            query: String::new(),
            query_cursor: 0,
            file_filter: String::new(),
            file_filter_cursor: 0,
            active_field: PickerField::Query,
            all_results: Vec::new(),
            filtered_results: Vec::new(),
            selected_index: 0,
            base_dir,
            pending_filter: false,
            last_filtered_query: String::new(),
            last_filtered_file_filter: String::new(),
            grep_rx: None,
            grep_cancel: None,
            last_grep_query: String::new(),
            grep_stale: false,
            loading: false,
            backend: PickerBackend::Grep(GrepState::new()),
        }
    }

    fn new_fuzzy_list(base_dir: PathBuf, results: Vec<PickerResult>, kind: FuzzyListKind) -> Self {
        Self {
            query: String::new(),
            query_cursor: 0,
            file_filter: String::new(),
            file_filter_cursor: 0,
            active_field: PickerField::Query,
            all_results: results.clone(),
            filtered_results: results,
            selected_index: 0,
            base_dir,
            pending_filter: false,
            last_filtered_query: String::new(),
            last_filtered_file_filter: String::new(),
            grep_rx: None,
            grep_cancel: None,
            last_grep_query: String::new(),
            grep_stale: false,
            loading: false,
            backend: PickerBackend::FuzzyList(kind),
        }
    }

    fn items_to_results(items: Vec<String>) -> Vec<PickerResult> {
        items
            .into_iter()
            .enumerate()
            .map(|(idx, display)| PickerResult {
                display,
                location: idx.to_string(),
                line: idx,
                col: 0,
                match_positions: Vec::new(),
                content: None,
            })
            .collect()
    }

    /// Creates a new picker with custom items
    pub fn new_custom(base_dir: PathBuf, items: Vec<String>) -> Self {
        Self::new_fuzzy_list(base_dir, Self::items_to_results(items), FuzzyListKind::Custom)
    }

    /// Creates a new completion picker with custom items
    pub fn new_completion(base_dir: PathBuf, items: Vec<String>) -> Self {
        Self::new_fuzzy_list(base_dir, Self::items_to_results(items), FuzzyListKind::Completion)
    }

    /// Creates a new LSP locations picker (for references, symbols, hierarchy, etc.)
    pub fn new_lsp_locations(base_dir: PathBuf, items: Vec<String>) -> Self {
        Self::new_fuzzy_list(base_dir, Self::items_to_results(items), FuzzyListKind::LspLocations)
    }

    /// Creates a new LSP locations picker with pre-built PickerResult items
    pub fn new_with_results(base_dir: PathBuf, results: Vec<PickerResult>) -> Self {
        Self::new_fuzzy_list(base_dir, results, FuzzyListKind::LspLocations)
    }

    /// Sets the prompt for the picker
    pub fn set_prompt(&mut self, _prompt: String) {}

    /// Starts an in-process grep search, cancelling any previous one.
    pub fn start_grep_search(&mut self) {
        self.cancel_grep();

        if self.query.is_empty() {
            self.all_results.clear();
            self.filtered_results.clear();
            self.selected_index = 0;
            return;
        }

        self.loading = true;
        self.last_grep_query = self.query.clone();

        let cancel = Arc::new(AtomicBool::new(false));
        self.grep_cancel = Some(cancel.clone());

        let rx = super::grep::spawn_grep_search(
            self.query.clone(),
            self.base_dir.clone(),
            cancel,
        );
        self.grep_rx = Some(rx);
        self.grep_stale = true;
    }

    /// Drains grep results from the channel with a 2ms budget.
    /// Returns true if any new results were added.
    pub fn drain_grep_results(&mut self) -> bool {
        let rx = match self.grep_rx.as_mut() {
            Some(rx) => rx,
            None => return false,
        };

        let start = std::time::Instant::now();
        let budget = std::time::Duration::from_millis(2);
        let mut added = false;

        loop {
            if start.elapsed() >= budget {
                break;
            }

            match rx.try_recv() {
                Ok(result) => {
                    if self.grep_stale {
                        self.all_results.clear();
                        self.filtered_results.clear();
                        self.selected_index = 0;
                        self.grep_stale = false;
                    }
                    self.all_results.push(result.clone());
                    if filter::matches_file_filter(&self.file_filter, &result.display) {
                        self.filtered_results.push(result);
                    }
                    added = true;
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    if self.grep_stale {
                        self.all_results.clear();
                        self.filtered_results.clear();
                        self.selected_index = 0;
                        self.grep_stale = false;
                    }
                    self.grep_rx = None;
                    self.loading = false;
                    break;
                }
            }
        }

        added
    }

    /// Re-applies the file filter on `all_results` to produce `filtered_results`.
    fn apply_file_filter(&mut self) {
        self.filtered_results = self
            .all_results
            .iter()
            .filter(|r| filter::matches_file_filter(&self.file_filter, &r.display))
            .cloned()
            .collect();
        self.selected_index = 0;
    }

    /// Cancels any in-flight grep search.
    pub fn cancel_grep(&mut self) {
        if let Some(cancel) = self.grep_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.grep_rx = None;
        self.loading = false;
    }

    /// Returns whether this picker uses nucleo for matching.
    pub fn uses_nucleo(&self) -> bool {
        matches!(self.backend, PickerBackend::Nucleo(_))
    }

    /// Returns the total number of results (before filtering).
    pub fn all_results_count(&self) -> usize {
        self.all_results.len()
    }

    /// Returns the number of filtered (matched) results.
    pub fn filtered_result_count(&self) -> usize {
        match &self.backend {
            PickerBackend::Nucleo(s) => s.matched_count,
            _ => self.filtered_results.len(),
        }
    }

    /// Pre-fetches visible item indices in a single nucleo snapshot.
    pub fn prefetch_visible_range(&mut self, start: usize, count: usize) {
        if let PickerBackend::Nucleo(ref mut s) = self.backend {
            s.cached_visible_indices =
                s.nucleo.get_items_in_range(start as u32, count as u32);
            s.cached_visible_start = start;
        }
    }

    /// Returns a reference to the nth filtered result (rank-ordered for nucleo).
    pub fn filtered_result(&self, idx: usize) -> Option<&PickerResult> {
        if let PickerBackend::Nucleo(ref s) = self.backend {
            // Try the prefetched cache first
            if idx >= s.cached_visible_start {
                let cache_idx = idx - s.cached_visible_start;
                if cache_idx < s.cached_visible_indices.len() {
                    let all_idx = s.cached_visible_indices[cache_idx] as usize;
                    return self.all_results.get(all_idx);
                }
            }
            let all_idx = s.nucleo.get_item_at_rank(idx as u32)?;
            self.all_results.get(all_idx as usize)
        } else {
            self.filtered_results.get(idx)
        }
    }

    /// Drives the nucleo matcher forward and updates matched count.
    /// Returns `true` if results changed.
    pub fn tick(&mut self) -> bool {
        if let PickerBackend::Nucleo(ref mut s) = self.backend {
            let changed = s.nucleo.tick();
            if changed {
                s.matched_count = s.nucleo.matched_count() as usize;
                if s.matched_count > 0 {
                    self.selected_index = self.selected_index.min(s.matched_count - 1);
                } else {
                    self.selected_index = 0;
                }
            }
            changed
        } else {
            false
        }
    }

    /// Updates the query and refreshes filtered results
    pub fn set_query(&mut self, query: String) {
        self.query = query;
        if let PickerBackend::Nucleo(ref mut s) = self.backend {
            s.nucleo.update_query(&self.query);
        } else {
            self.apply_filter_internal();
        }
    }

    /// Internal filter logic
    fn apply_filter_internal(&mut self) {
        match &self.backend {
            PickerBackend::Nucleo(_) => {
                // Nucleo handles its own filtering
                unreachable!("apply_filter_internal should not be called for Nucleo backend");
            }
            PickerBackend::FuzzyList(_) => {
                let mut scored_results: Vec<(PickerResult, i32, Vec<usize>)> = self
                    .all_results
                    .iter()
                    .filter_map(|r| {
                        fuzzy::fuzzy_score(&self.query, &r.display).map(|(score, positions)| {
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
            PickerBackend::Grep(_) => {
                self.start_grep_search();
                self.pending_filter = false;
                return;
            }
        }

        self.selected_index = 0;
        self.last_filtered_query = self.query.clone();
        self.last_filtered_file_filter = self.file_filter.clone();
        self.pending_filter = false;
    }

    /// Marks that filtering is pending (query changed but not yet filtered).
    pub fn mark_filter_pending(&mut self) {
        match &mut self.backend {
            PickerBackend::Nucleo(s) => {
                s.nucleo.update_query(&self.query);
            }
            PickerBackend::Grep(_) => {
                if self.query != self.last_grep_query {
                    self.pending_filter = true;
                } else if self.file_filter != self.last_filtered_file_filter {
                    self.apply_file_filter();
                    self.last_filtered_file_filter = self.file_filter.clone();
                }
            }
            PickerBackend::FuzzyList(_) => {
                self.pending_filter = true;
            }
        }
    }

    /// Returns true if there's a pending filter operation
    pub fn has_pending_filter(&self) -> bool {
        self.pending_filter
    }

    /// Applies the pending filter if query has changed since last filter
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
        let count = self.filtered_result_count();
        if count > 0 {
            self.selected_index = (self.selected_index + 1).min(count - 1);
        }
    }

    /// Moves selection down by n items
    pub fn move_down_n(&mut self, n: usize) {
        let count = self.filtered_result_count();
        if count > 0 {
            self.selected_index = (self.selected_index + n).min(count - 1);
        }
    }

    /// Moves selection up
    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Moves selection up by n items
    pub fn move_up_n(&mut self, n: usize) {
        self.selected_index = self.selected_index.saturating_sub(n);
    }

    /// Gets the currently selected result
    pub fn selected_result(&self) -> Option<&PickerResult> {
        self.filtered_result(self.selected_index)
    }

    /// Derives the action to execute for the currently selected result.
    pub fn selected_action(&self) -> Option<PickerAction> {
        let result = self.selected_result()?;
        match &self.backend {
            PickerBackend::FuzzyList(FuzzyListKind::Custom) => {
                Some(PickerAction::ApplyCodeAction { index: result.line })
            }
            PickerBackend::FuzzyList(FuzzyListKind::Completion) => {
                Some(PickerAction::ApplyCompletion { index: result.line })
            }
            PickerBackend::FuzzyList(FuzzyListKind::LspLocations) => {
                Some(PickerAction::OpenFileWithTag {
                    path: result.location.clone(),
                    line: result.line,
                    col: result.col,
                })
            }
            PickerBackend::Nucleo(_) | PickerBackend::Grep(_) => {
                Some(PickerAction::OpenFile {
                    path: result.location.clone(),
                    line: result.line,
                    col: result.col,
                })
            }
        }
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
        // Return a reference to a static or compute it
        // Since PickerMode is Clone + small, we can use a match
        // But we need to return &PickerMode, so use a helper
        match &self.backend {
            PickerBackend::Nucleo(_) => &PickerMode::FindFiles,
            PickerBackend::Grep(_) => &PickerMode::LiveGrep,
            PickerBackend::FuzzyList(kind) => match kind {
                FuzzyListKind::Custom => &PickerMode::Custom,
                FuzzyListKind::Completion => &PickerMode::Completion,
                FuzzyListKind::LspLocations => &PickerMode::LspLocations,
            },
        }
    }

    /// Gets the base directory for file operations
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Returns true if this picker mode supports the file filter field
    pub fn has_file_filter(&self) -> bool {
        matches!(self.backend, PickerBackend::Grep(_))
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

    /// Sets the active input field (for mouse clicks)
    pub fn set_active_field(&mut self, field: PickerField) {
        if field == PickerField::FileFilter && !self.has_file_filter() {
            return;
        }
        self.active_field = field;
    }

    /// Sets the selected index (for mouse clicks), clamped to valid range
    pub fn set_selected_index(&mut self, index: usize) {
        let count = self.filtered_result_count();
        if count > 0 {
            self.selected_index = index.min(count - 1);
        }
    }

    /// Adds a file result (for incremental loading)
    pub fn add_file_result(&mut self, result: PickerResult) {
        let idx = self.all_results.len() as u32;
        let display = result.display.clone();
        self.all_results.push(result.clone());

        if let PickerBackend::Nucleo(ref s) = self.backend {
            s.nucleo.inject(idx, &display);
        } else {
            if self.query.is_empty() {
                self.filtered_results.push(result);
            } else if fuzzy::fuzzy_score(&self.query, &result.display).is_some() {
                self.filtered_results.push(result);
                self.pending_filter = true;
            }
        }
    }

    /// Marks file loading as complete
    pub fn finish_loading(&mut self) {
        self.loading = false;
        if let PickerBackend::Nucleo(ref mut s) = self.backend {
            s.loading = false;
        }
    }

    /// Returns whether files are still being loaded
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// Returns whether file loading should be spawned
    pub fn should_spawn_file_loading(&self) -> bool {
        if let PickerBackend::Nucleo(ref s) = self.backend {
            s.loading && !s.loading_spawned
        } else {
            false
        }
    }

    /// Marks file loading as spawned
    pub fn mark_loading_spawned(&mut self) {
        if let PickerBackend::Nucleo(ref mut s) = self.backend {
            s.loading_spawned = true;
        }
    }

    /// Truncates a path in the middle if it's too long
    pub fn truncate_path(path: &str, max_len: usize) -> String {
        filter::truncate_path(path, max_len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_glob_match_star() {
        assert!(filter::glob_match("*.rs", "main.rs"));
        assert!(filter::glob_match("*.rs", "MAIN.RS"));
        assert!(!filter::glob_match("*.rs", "main.ts"));
        assert!(filter::glob_match("src/*", "src/lib.rs"));
        assert!(filter::glob_match("*test*", "my_test_file.rs"));
    }

    #[test]
    fn test_glob_match_question() {
        assert!(filter::glob_match("?.rs", "a.rs"));
        assert!(!filter::glob_match("?.rs", "ab.rs"));
        assert!(filter::glob_match("??.rs", "ab.rs"));
    }

    #[test]
    fn test_glob_match_combined() {
        assert!(filter::glob_match("*_test.?s", "my_test.rs"));
        assert!(filter::glob_match("*_test.?s", "my_test.ts"));
        assert!(!filter::glob_match("*_test.?s", "my_test.css"));
    }

    #[test]
    fn test_matches_file_filter_empty() {
        assert!(filter::matches_file_filter("", "src/main.rs"));
        assert!(filter::matches_file_filter("   ", "src/main.rs"));
    }

    #[test]
    fn test_matches_file_filter_substring() {
        assert!(filter::matches_file_filter("mod", "src/mod.rs"));
        assert!(filter::matches_file_filter("mod", "src/editor/mod.rs"));
        assert!(!filter::matches_file_filter("xyz", "src/main.rs"));
    }

    #[test]
    fn test_matches_file_filter_glob() {
        assert!(filter::matches_file_filter("*.rs", "src/main.rs"));
        assert!(!filter::matches_file_filter("*.ts", "src/main.rs"));
    }

    #[test]
    fn test_matches_file_filter_multiple_tokens() {
        assert!(filter::matches_file_filter("*.rs mod", "mod.rs"));
        assert!(!filter::matches_file_filter("*.rs xyz", "mod.rs"));
    }

    #[test]
    fn test_matches_file_filter_path_token() {
        assert!(filter::matches_file_filter("src/", "src/main.rs"));
        assert!(!filter::matches_file_filter("src/", "lib/main.rs"));
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
        assert_eq!(picker.active_field(), PickerField::Query);
    }

    #[test]
    fn test_toggle_field_no_op_for_custom() {
        let mut picker = Picker::new_custom(PathBuf::from("."), vec!["a".into(), "b".into()]);
        assert_eq!(picker.active_field(), PickerField::Query);

        picker.toggle_field();
        assert_eq!(picker.active_field(), PickerField::Query);
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

        picker.insert_text(" world");
        assert_eq!(picker.query(), "hello world");
        assert_eq!(picker.query_cursor(), 11);
    }

    #[test]
    fn test_insert_text_at_cursor_midpoint() {
        let mut picker = Picker::new_file_finder(PathBuf::from("."));
        picker.insert_text("ac");
        picker.move_cursor_left();
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
