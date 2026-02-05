use super::backend::PickerBackend;
use super::fuzzy_backend::FuzzyListKind;
use super::grep_backend::GrepState;
use super::nucleo_backend::NucleoState;
use super::result::{PickerField, PickerResult};
use super::Picker;
use std::path::PathBuf;

impl Picker {
    /// Creates a new file finder picker
    pub fn new_file_finder(base_dir: PathBuf, preferred_dir: PathBuf) -> Self {
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
            preferred_dir,
            pending_filter: false,
            backend: PickerBackend::Nucleo(NucleoState::new()),
        }
    }

    /// Creates a new live grep picker
    pub fn new_live_grep(base_dir: PathBuf, preferred_dir: PathBuf) -> Self {
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
            preferred_dir,
            pending_filter: false,
            backend: PickerBackend::Grep(GrepState::new()),
        }
    }

    pub(super) fn new_fuzzy_list(
        base_dir: PathBuf,
        preferred_dir: PathBuf,
        results: Vec<PickerResult>,
        kind: FuzzyListKind,
    ) -> Self {
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
            preferred_dir,
            pending_filter: false,
            backend: PickerBackend::FuzzyList(kind),
        }
    }

    pub(super) fn items_to_results(items: Vec<String>) -> Vec<PickerResult> {
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
        let preferred_dir = base_dir.clone();
        Self::new_fuzzy_list(
            base_dir,
            preferred_dir,
            Self::items_to_results(items),
            FuzzyListKind::Custom,
        )
    }

    /// Creates a new completion picker with custom items
    pub fn new_completion(base_dir: PathBuf, items: Vec<String>) -> Self {
        let preferred_dir = base_dir.clone();
        Self::new_fuzzy_list(
            base_dir,
            preferred_dir,
            Self::items_to_results(items),
            FuzzyListKind::Completion,
        )
    }

    /// Creates a new LSP locations picker
    pub fn new_lsp_locations(base_dir: PathBuf, items: Vec<String>) -> Self {
        let preferred_dir = base_dir.clone();
        Self::new_fuzzy_list(
            base_dir,
            preferred_dir,
            Self::items_to_results(items),
            FuzzyListKind::LspLocations,
        )
    }

    /// Creates a new LSP locations picker with pre-built PickerResult items
    pub fn new_with_results(base_dir: PathBuf, results: Vec<PickerResult>) -> Self {
        let preferred_dir = base_dir.clone();
        Self::new_fuzzy_list(base_dir, preferred_dir, results, FuzzyListKind::LspLocations)
    }

    /// Sets the prompt for the picker
    pub fn set_prompt(&mut self, _prompt: String) {}
}
