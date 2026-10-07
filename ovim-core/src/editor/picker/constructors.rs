use super::backend::PickerBackend;
use super::fuzzy_backend::FuzzyListKind;
use super::grep_backend::GrepState;
use super::nucleo_backend::NucleoState;
use super::result::{GitPick, PickerField, PickerResult};
use super::Picker;
use crate::editor::SingleLineInput;
use std::path::PathBuf;

impl Picker {
    /// Creates a new file finder picker
    pub fn new_file_finder(base_dir: PathBuf, preferred_dir: PathBuf) -> Self {
        Self {
            query: SingleLineInput::default(),
            file_filter: SingleLineInput::default(),
            active_field: PickerField::Query,
            all_results: Vec::new(),
            filtered_results: Vec::new(),
            selected_index: 0,
            base_dir,
            preferred_dir,
            pending_filter: false,
            backend: PickerBackend::Nucleo(Box::new(NucleoState::new())),
            title: None,
            symbol_query_pending: false,
            role: None,
        }
    }

    /// Creates a new live grep picker
    pub fn new_live_grep(base_dir: PathBuf, preferred_dir: PathBuf) -> Self {
        Self {
            query: SingleLineInput::default(),
            file_filter: SingleLineInput::default(),
            active_field: PickerField::Query,
            all_results: Vec::new(),
            filtered_results: Vec::new(),
            selected_index: 0,
            base_dir,
            preferred_dir,
            pending_filter: false,
            backend: PickerBackend::Grep(GrepState::new()),
            title: None,
            symbol_query_pending: false,
            role: None,
        }
    }

    pub(super) fn new_fuzzy_list(
        base_dir: PathBuf,
        preferred_dir: PathBuf,
        results: Vec<PickerResult>,
        kind: FuzzyListKind,
    ) -> Self {
        Self {
            query: SingleLineInput::default(),
            file_filter: SingleLineInput::default(),
            active_field: PickerField::Query,
            all_results: results.clone(),
            filtered_results: results,
            selected_index: 0,
            base_dir,
            preferred_dir,
            pending_filter: false,
            backend: PickerBackend::FuzzyList(kind),
            title: None,
            symbol_query_pending: false,
            role: None,
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
        Self::new_fuzzy_list(
            base_dir,
            preferred_dir,
            results,
            FuzzyListKind::LspLocations,
        )
    }

    /// Creates a picker whose rows each open a git view when selected.
    pub fn new_git(base_dir: PathBuf, rows: Vec<(String, GitPick)>, title: &str) -> Self {
        let preferred_dir = base_dir.clone();
        let (results, picks) = Self::git_results(rows);
        Self::new_fuzzy_list(base_dir, preferred_dir, results, FuzzyListKind::Git(picks))
            .with_title(title)
    }

    /// Display rows for `rows`, each pointing at its pick by index.
    pub(super) fn git_results(rows: Vec<(String, GitPick)>) -> (Vec<PickerResult>, Vec<GitPick>) {
        rows.into_iter()
            .enumerate()
            .map(|(index, (display, pick))| {
                let result = PickerResult {
                    display,
                    location: String::new(),
                    line: index,
                    col: 0,
                    match_positions: Vec::new(),
                    content: None,
                };
                (result, pick)
            })
            .unzip()
    }

    /// Creates the live workspace-symbol picker; results arrive from the server.
    pub fn new_workspace_symbols(base_dir: PathBuf) -> Self {
        let preferred_dir = base_dir.clone();
        let mut picker = Self::new_fuzzy_list(
            base_dir,
            preferred_dir,
            Vec::new(),
            FuzzyListKind::WorkspaceSymbols,
        )
        .with_title("Workspace symbols");
        picker.symbol_query_pending = true;
        picker
    }

    /// Creates a new debug config picker
    pub fn new_debug_config(base_dir: PathBuf, items: Vec<String>) -> Self {
        let preferred_dir = base_dir.clone();
        Self::new_fuzzy_list(
            base_dir,
            preferred_dir,
            Self::items_to_results(items),
            FuzzyListKind::DebugConfig,
        )
    }

    /// Creates the action picker for a server `window/showMessageRequest`
    pub fn new_message_actions(base_dir: PathBuf, items: Vec<String>) -> Self {
        let preferred_dir = base_dir.clone();
        Self::new_fuzzy_list(
            base_dir,
            preferred_dir,
            Self::items_to_results(items),
            FuzzyListKind::MessageAction,
        )
    }

    /// Sets the prompt for the picker
    pub fn set_prompt(&mut self, _prompt: String) {}
}
