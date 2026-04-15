//! Picker management and preview cache

use super::picker_state::PickerState;
use super::{Editor, Picker, PreviewCache};
use std::collections::HashMap;

// ==================== PickerState methods ====================

impl PickerState {
    /// Sets the picker
    pub fn set_picker(&mut self, picker: Picker) {
        self.picker = Some(picker);
    }

    /// Gets a reference to the picker
    pub fn picker(&self) -> Option<&Picker> {
        self.picker.as_ref()
    }

    /// Gets a mutable reference to the picker
    pub fn picker_mut(&mut self) -> Option<&mut Picker> {
        self.picker.as_mut()
    }

    /// Marks that picker query was just changed (for debouncing preview loading and filtering)
    pub fn mark_query_changed(&mut self) {
        self.last_query_change = Some(std::time::Instant::now());
        // Clear loading flag since query changed
        self.loading_preview = None;
    }

    /// Checks if enough time has elapsed since picker query changed (for debouncing filtering)
    /// Returns true if we should apply the pending filter now
    pub fn should_apply_filter(&self, debounce_ms: u64) -> bool {
        match self.last_query_change {
            None => true, // No recent change, apply immediately
            Some(last_change) => last_change.elapsed().as_millis() >= debounce_ms as u128,
        }
    }

    /// Applies pending filter if debounce period has elapsed
    /// Returns true if filter was applied
    pub fn apply_pending_filter(&mut self, debounce_ms: u64) -> bool {
        if !self.should_apply_filter(debounce_ms) {
            return false;
        }

        if let Some(picker) = self.picker.as_mut() {
            if picker.has_pending_filter() {
                picker.apply_pending_filter();
                return true;
            }
        }
        false
    }

    /// Marks that the picker selection moved (for debouncing preview loading)
    pub fn mark_selection_changed(&mut self) {
        self.prev_selection_change = self.last_selection_change;
        self.last_selection_change = Some(std::time::Instant::now());
        // Allow new preview to load for the freshly selected entry
        self.loading_preview = None;
    }

    /// Returns true if user is scrolling rapidly through picker results.
    /// Used to skip expensive preview rendering during rapid navigation.
    /// Detects rapid scrolling by checking if two consecutive selection changes
    /// happened within 80ms of each other (i.e., holding j/k or fast repeated presses).
    /// A single keypress won't trigger this — only sustained rapid navigation.
    /// Also requires that the last change was recent (< 150ms ago) so that
    /// rapid scrolling "expires" once the user stops — otherwise the stale
    /// timestamps would keep returning true forever.
    pub fn is_scrolling_rapidly(&self) -> bool {
        match (self.prev_selection_change, self.last_selection_change) {
            (Some(prev), Some(last)) => {
                last.duration_since(prev).as_millis() < 80 && last.elapsed().as_millis() < 150
            }
            _ => false,
        }
    }

    /// Returns true if rapid scrolling just stopped (was rapid, now isn't).
    /// Used to trigger a re-render so syntax highlighting gets applied.
    pub fn rapid_scrolling_just_stopped(&mut self) -> bool {
        let rapid_now = self.is_scrolling_rapidly();
        let was_rapid = self.was_scrolling_rapidly;
        self.was_scrolling_rapidly = rapid_now;
        was_rapid && !rapid_now
    }

    /// Checks if enough time has elapsed since picker query changed (for debouncing)
    /// Returns true if we should load preview now
    pub fn should_load_preview(&self, debounce_ms: u64) -> bool {
        let mut last_change = self.last_query_change;

        if let Some(selection_change) = self.last_selection_change {
            last_change = match last_change {
                Some(existing) => Some(std::cmp::max(existing, selection_change)),
                None => Some(selection_change),
            };
        }

        match last_change {
            None => true, // No recent change, load immediately
            Some(last_change) => last_change.elapsed().as_millis() >= debounce_ms as u128,
        }
    }

    /// Gets the path that should be loaded for preview (if any)
    /// Returns None if already cached or already loading
    pub fn get_preview_to_load(&mut self) -> Option<String> {
        if let Some(picker) = self.picker() {
            if let Some(result) = picker.selected_result() {
                // Skip modes that don't have file paths
                if *picker.mode() == crate::editor::PickerMode::Custom
                    || *picker.mode() == crate::editor::PickerMode::Completion
                {
                    return None;
                }

                let file_path = result.location.clone();

                // Skip if already cached
                if self.preview_cache.contains_key(&file_path) {
                    return None;
                }

                // Skip if currently loading
                if self.loading_preview.as_ref() == Some(&file_path) {
                    return None;
                }

                // Mark as loading
                self.loading_preview = Some(file_path.clone());
                return Some(file_path);
            }
        }
        None
    }

    /// Inserts a loaded preview into the cache
    pub fn insert_preview(&mut self, file_path: String, cache: PreviewCache) {
        self.preview_cache.insert(file_path.clone(), cache);
        // Clear loading flag
        if self.loading_preview.as_ref() == Some(&file_path) {
            self.loading_preview = None;
        }
        // Trim cache
        self.trim_preview_cache(50);
    }

    /// Closes the picker
    pub fn close_picker(&mut self) {
        // Cancel any in-flight grep search before dropping
        if let Some(picker) = self.picker.as_mut() {
            picker.cancel_grep();
        }
        self.picker = None;
        // Clear preview cache when closing picker to free memory
        self.preview_cache.clear();
        self.last_selection_change = None;
        self.prev_selection_change = None;
        self.was_scrolling_rapidly = false;
        self.last_layout = None;
    }

    /// Gets preview from cache or loads it (async version)
    pub async fn get_or_load_preview_async(&mut self, file_path: &str) -> Option<&PreviewCache> {
        // Check if already cached
        if self.preview_cache.contains_key(file_path) {
            return self.preview_cache.get(file_path);
        }

        // Check file size before loading (max 1MB for preview)
        const MAX_PREVIEW_SIZE: u64 = 1024 * 1024;
        if let Ok(metadata) = tokio::fs::metadata(file_path).await {
            if metadata.len() > MAX_PREVIEW_SIZE {
                // File too large, create a placeholder cache entry
                let cache = PreviewCache {
                    content: format!("File too large for preview ({} bytes)", metadata.len()),
                    highlighted_lines: std::cell::RefCell::new(HashMap::new()),
                    language: None,
                };
                self.preview_cache.insert(file_path.to_string(), cache);
                // Track as last shown so fallback works correctly
                self.last_shown_preview = Some(file_path.to_string());
                return self.preview_cache.get(file_path);
            }
        }

        // Load the file
        let content = match tokio::fs::read_to_string(file_path).await {
            Ok(c) => c,
            Err(_) => return None,
        };

        // Detect language
        let language = crate::syntax::LanguageRegistry::detect_from_path(file_path);

        // Create cache entry
        let cache = PreviewCache {
            content,
            highlighted_lines: std::cell::RefCell::new(HashMap::new()),
            language,
        };

        self.preview_cache.insert(file_path.to_string(), cache);
        // Track as last shown so fallback works correctly
        self.last_shown_preview = Some(file_path.to_string());
        self.preview_cache.get(file_path)
    }

    /// Gets preview from cache or loads it (blocking wrapper)
    pub fn get_or_load_preview(&mut self, file_path: &str) -> Option<&PreviewCache> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.get_or_load_preview_async(file_path))
        })
    }

    /// Gets cached preview if available
    pub fn get_preview_cache(&self, file_path: &str) -> Option<&PreviewCache> {
        self.preview_cache.get(file_path)
    }

    /// Gets preview with fallback - prefers current file, but shows last preview if loading
    /// This provides smooth transitions without "Loading..." flicker
    /// Returns (preview, actual_path, is_stale) where is_stale indicates fallback was used
    pub fn get_preview_with_fallback(&self, file_path: &str) -> Option<(&PreviewCache, bool)> {
        // Try to get the requested preview
        if let Some(preview) = self.preview_cache.get(file_path) {
            return Some((preview, false));
        }

        // Fall back to last shown preview while new one loads
        if let Some(last_path) = &self.last_shown_preview {
            if let Some(preview) = self.preview_cache.get(last_path) {
                // Return the old preview (marked as stale)
                return Some((preview, true));
            }
        }

        None
    }

    /// Update the last shown preview path (called after successful render)
    pub fn set_last_shown_preview(&mut self, file_path: &str) {
        self.last_shown_preview = Some(file_path.to_string());
    }

    /// Limits preview cache size to prevent memory bloat
    pub fn trim_preview_cache(&mut self, max_entries: usize) {
        if self.preview_cache.len() > max_entries {
            // Keep only the most recent entries
            // Simple strategy: clear half when limit is exceeded
            let to_remove = self.preview_cache.len() - max_entries / 2;
            let keys_to_remove: Vec<String> =
                self.preview_cache.keys().take(to_remove).cloned().collect();
            for key in keys_to_remove {
                self.preview_cache.remove(&key);
            }
        }
    }

    // ==================== File List Cache Methods ====================

    /// File list cache TTL (5 minutes)
    const FILE_LIST_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

    /// Gets cached file list if available and fresh (less than 5 minutes old)
    pub fn get_cached_file_list(
        &self,
        base_dir: &std::path::Path,
        preferred_dir: &std::path::Path,
    ) -> Option<&[super::PickerResult]> {
        let (cached_base, cached_preferred, files, timestamp) = self.file_list_cache.as_ref()?;
        if cached_base == base_dir
            && cached_preferred == preferred_dir
            && timestamp.elapsed() < Self::FILE_LIST_CACHE_TTL
        {
            Some(files.as_slice())
        } else {
            None
        }
    }

    /// Stores the file list in cache with current timestamp
    pub fn update_file_list_cache(
        &mut self,
        base_dir: std::path::PathBuf,
        preferred_dir: std::path::PathBuf,
        files: Vec<super::PickerResult>,
    ) {
        self.file_list_cache = Some((base_dir, preferred_dir, files, std::time::Instant::now()));
    }

    /// Invalidates the file list cache (called on file save/create/delete)
    pub fn invalidate_file_list_cache(&mut self) {
        self.file_list_cache = None;
    }
}

// ==================== Editor delegation methods ====================

impl Editor {
    /// Returns the best base directory for file picker / grep operations.
    ///
    /// Priority:
    /// 1. Git root of the current file (walk up looking for `.git`)
    /// 2. Parent directory of the current file
    /// 3. `current_dir()` as last resort
    ///
    /// This prevents scanning the user's entire home directory (and triggering
    /// macOS iCloud Drive / TCC permission dialogs) when ovim is launched from ~.
    pub fn picker_base_dir(&self) -> std::path::PathBuf {
        // Try to find git root from current file
        if let Some(file_path) = self.buffer().file_path() {
            let path = std::path::Path::new(file_path);

            // Make path absolute for reliable parent traversal
            let abs_path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(path))
                    .unwrap_or_else(|_| path.to_path_buf())
            };

            // Walk up looking for .git
            let mut current = abs_path.parent();
            while let Some(dir) = current {
                if dir.join(".git").exists() {
                    return dir.to_path_buf();
                }
                current = dir.parent();
            }

            // No git root found — use file's parent directory
            if let Some(parent) = abs_path.parent() {
                return parent.to_path_buf();
            }
        }

        // Fallback: current working directory
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    }

    /// Returns `(base_dir, preferred_dir)` for picker operations.
    ///
    /// - `base_dir`: the broader search root (git root if available)
    /// - `preferred_dir`: the current file's parent directory (local-first ranking)
    pub fn picker_dirs(&self) -> (std::path::PathBuf, std::path::PathBuf) {
        // Default both to CWD
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

        let Some(file_path) = self.buffer().file_path() else {
            let base = self.picker_base_dir();
            return (base.clone(), base);
        };

        let path = std::path::Path::new(file_path);
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        };

        let preferred = abs_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| cwd.clone());

        let base = self.picker_base_dir();
        (base, preferred)
    }

    /// Sets the picker
    pub fn set_picker(&mut self, picker: Picker) {
        self.picker_state.set_picker(picker);
    }

    /// Gets a reference to the picker
    pub fn picker(&self) -> Option<&Picker> {
        self.picker_state.picker()
    }

    /// Gets a mutable reference to the picker
    pub fn picker_mut(&mut self) -> Option<&mut Picker> {
        self.picker_state.picker_mut()
    }

    /// Closes the picker
    pub fn close_picker(&mut self) {
        self.picker_state.close_picker();
    }

    /// Marks that picker query was just changed (for debouncing preview loading and filtering)
    pub fn mark_picker_query_changed(&mut self) {
        self.picker_state.mark_query_changed();
    }

    /// Marks that the picker selection moved (for debouncing preview loading)
    pub fn mark_picker_selection_changed(&mut self) {
        self.picker_state.mark_selection_changed();
    }

    /// Returns true if user is scrolling rapidly through picker results
    pub fn is_picker_scrolling_rapidly(&self) -> bool {
        self.picker_state.is_scrolling_rapidly()
    }

    /// Returns true if rapid scrolling just stopped (needs re-render for syntax highlighting)
    pub fn picker_rapid_scrolling_just_stopped(&mut self) -> bool {
        self.picker_state.rapid_scrolling_just_stopped()
    }

    /// Checks if enough time has elapsed since picker query changed (for debouncing filtering)
    pub fn should_apply_picker_filter(&self, debounce_ms: u64) -> bool {
        self.picker_state.should_apply_filter(debounce_ms)
    }

    /// Applies pending filter if debounce period has elapsed
    pub fn apply_pending_picker_filter(&mut self, debounce_ms: u64) -> bool {
        self.picker_state.apply_pending_filter(debounce_ms)
    }

    /// Checks if enough time has elapsed since picker query changed (for debouncing)
    pub fn should_load_picker_preview(&self, debounce_ms: u64) -> bool {
        self.picker_state.should_load_preview(debounce_ms)
    }

    /// Gets the path that should be loaded for preview (if any)
    pub fn get_preview_to_load(&mut self) -> Option<String> {
        self.picker_state.get_preview_to_load()
    }

    /// Inserts a loaded preview into the cache
    pub fn insert_preview(&mut self, file_path: String, cache: PreviewCache) {
        self.picker_state.insert_preview(file_path, cache);
    }

    /// Gets preview from cache or loads it (async version)
    pub async fn get_or_load_preview_async(&mut self, file_path: &str) -> Option<&PreviewCache> {
        self.picker_state.get_or_load_preview_async(file_path).await
    }

    /// Gets preview from cache or loads it (blocking wrapper)
    pub fn get_or_load_preview(&mut self, file_path: &str) -> Option<&PreviewCache> {
        self.picker_state.get_or_load_preview(file_path)
    }

    /// Gets cached preview if available
    pub fn get_preview_cache(&self, file_path: &str) -> Option<&PreviewCache> {
        self.picker_state.get_preview_cache(file_path)
    }

    /// Gets preview with fallback
    pub fn get_preview_with_fallback(&self, file_path: &str) -> Option<(&PreviewCache, bool)> {
        self.picker_state.get_preview_with_fallback(file_path)
    }

    /// Update the last shown preview path
    pub fn set_last_shown_preview(&mut self, file_path: &str) {
        self.picker_state.set_last_shown_preview(file_path);
    }

    /// Limits preview cache size to prevent memory bloat
    pub fn trim_preview_cache(&mut self, max_entries: usize) {
        self.picker_state.trim_preview_cache(max_entries);
    }

    /// Gets cached file list if available and fresh
    pub fn get_cached_file_list(
        &self,
        base_dir: &std::path::Path,
        preferred_dir: &std::path::Path,
    ) -> Option<&[super::PickerResult]> {
        self.picker_state
            .get_cached_file_list(base_dir, preferred_dir)
    }

    /// Stores the file list in cache with current timestamp
    pub fn update_file_list_cache(
        &mut self,
        base_dir: std::path::PathBuf,
        preferred_dir: std::path::PathBuf,
        files: Vec<super::PickerResult>,
    ) {
        self.picker_state
            .update_file_list_cache(base_dir, preferred_dir, files);
    }

    /// Invalidates the file list cache
    pub fn invalidate_file_list_cache(&mut self) {
        self.picker_state.invalidate_file_list_cache();
    }

    /// Executes a picker action (called after the picker is closed)
    pub fn execute_picker_action(&mut self, action: super::PickerAction) -> anyhow::Result<()> {
        use super::PickerAction;
        match action {
            PickerAction::OpenFile { path, line, col } => {
                if let Err(e) = self.load_file(&path) {
                    self.set_lsp_status(format!("Failed to load file {}: {}", path, e));
                    return Ok(());
                }
                self.buffer_mut().cursor_mut().set_position(line, crate::unicode::GraphemeCol(col));
                self.buffer_mut().validate_cursor_position();
                self.center_cursor_in_viewport();
            }
            PickerAction::OpenFileWithTag { path, line, col } => {
                self.push_tag();
                if let Err(e) = self.load_file(&path) {
                    self.set_lsp_status(format!("Failed to load file {}: {}", path, e));
                    return Ok(());
                }
                self.buffer_mut().cursor_mut().set_position(line, crate::unicode::GraphemeCol(col));
                self.buffer_mut().validate_cursor_position();
                self.center_cursor_in_viewport();
            }
            PickerAction::ApplyCodeAction { index } => {
                self.apply_code_action(index);
            }
            PickerAction::ApplyCompletion { index } => {
                self.accept_completion_at(index);
            }
            PickerAction::SelectDebugConfig { index } => {
                self.select_debug_config(index);
            }
        }
        Ok(())
    }

    /// Select a debug run config by index and start the debug session.
    fn select_debug_config(&mut self, index: usize) {
        let configs = &self.dap_manager.available_debug_configs;
        if index >= configs.len() {
            self.set_lsp_status("Invalid debug config index".to_string());
            return;
        }
        let config = configs[index].clone();
        self.dap_manager.available_debug_configs.clear();

        // Detect the DAP adapter command from the current file's language config.
        let dap_start = self
            .buffer()
            .file_path()
            .and_then(|fp| {
                crate::language_config::LanguageRegistry::try_get().and_then(|reg| reg.detect(fp))
            })
            .and_then(|lang| lang.dap.as_ref())
            .and_then(|dap_config| {
                crate::language_config::find_dap_command(dap_config)
                    .map(|cmd| (cmd, dap_config.args.clone()))
            });

        if let Some((command, args)) = dap_start {
            self.dap_manager.pending_action = Some(crate::dap::PendingDebugAction::Start {
                command,
                args,
                run_config: Some(config),
            });
        } else {
            self.set_lsp_status("No DAP adapter configured for this language".to_string());
        }
    }
}
