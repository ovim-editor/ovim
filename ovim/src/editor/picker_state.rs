//! Picker state management
//!
//! Groups all picker-related fields into a single struct, extracted from Editor.

use super::picker::{Picker, PickerResult};
use super::PickerLayout;
use super::PreviewCache;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

/// Consolidated picker state, grouping all picker-related fields
/// previously scattered across the Editor struct.
pub struct PickerState {
    /// Picker for fuzzy finding files/grep
    pub picker: Option<Picker>,
    /// Preview cache for picker (file_path -> (content, syntax highlights))
    pub preview_cache: HashMap<String, PreviewCache>,
    /// Last time picker query changed (for debouncing preview loading and filtering)
    pub last_query_change: Option<Instant>,
    /// Last time picker selection moved (for debouncing preview loading)
    pub last_selection_change: Option<Instant>,
    /// Previous picker selection change time (for detecting rapid scrolling vs single navigation)
    pub prev_selection_change: Option<Instant>,
    /// Currently loading preview path (to avoid duplicate requests)
    pub loading_preview: Option<String>,
    /// Last successfully shown preview path (to show while new one loads)
    pub last_shown_preview: Option<String>,
    /// Cached file list for picker: (root_path, files, timestamp)
    /// Speeds up repeated picker opens by reusing file discovery results
    pub file_list_cache: Option<(PathBuf, Vec<PickerResult>, Instant)>,
    /// Cached picker layout rects from last render (for mouse hit-testing)
    pub last_layout: Option<PickerLayout>,
    /// Whether the last render was during rapid scrolling (to detect transition → not-rapid)
    pub was_scrolling_rapidly: bool,
}

impl PickerState {
    pub fn new() -> Self {
        Self {
            picker: None,
            preview_cache: HashMap::new(),
            last_query_change: None,
            last_selection_change: None,
            prev_selection_change: None,
            loading_preview: None,
            last_shown_preview: None,
            file_list_cache: None,
            last_layout: None,
            was_scrolling_rapidly: false,
        }
    }
}

impl Default for PickerState {
    fn default() -> Self {
        Self::new()
    }
}
