//! "Replace in files": a reviewable, project-wide find/replace panel.
//!
//! The panel owns the find/replace/files inputs, the option toggles and the
//! grouped result list with a checkbox per match. Searching runs on a worker
//! thread (debounced while typing); applying goes through the same buffer
//! machinery as LSP workspace edits so every touched file becomes an undoable
//! change in its own buffer, including files that were not open.

use super::{Editor, SingleLineInput};
use crate::mode::Mode;
use crate::project_search::{self, FileMatches, LineMatch, SearchOptions};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long the inputs must be idle before a search starts.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(180);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchReplaceField {
    Find,
    Replace,
    Files,
    /// The result list has the keyboard (j/k, Space toggles).
    Results,
}

impl SearchReplaceField {
    pub fn as_str(self) -> &'static str {
        match self {
            SearchReplaceField::Find => "find",
            SearchReplaceField::Replace => "replace",
            SearchReplaceField::Files => "files",
            SearchReplaceField::Results => "results",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReviewMatch {
    pub found: LineMatch,
    pub checked: bool,
}

#[derive(Debug, Clone)]
pub struct ReviewFile {
    pub path: PathBuf,
    pub rel: String,
    pub matches: Vec<ReviewMatch>,
}

impl ReviewFile {
    pub fn checked_count(&self) -> usize {
        self.matches.iter().filter(|m| m.checked).count()
    }
}

/// One visible row of the result list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewRow {
    File(usize),
    Match(usize, usize),
}

struct SearchJob {
    rx: Receiver<Result<project_search::SearchOutcome, String>>,
    cancel: Arc<AtomicBool>,
}

/// What an apply did, for the status line and tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplaceReport {
    pub replaced: usize,
    pub files: usize,
    /// Matches skipped because the line no longer matches what was reviewed.
    pub stale: usize,
    /// Buffers whose edit is applied but could not be written to disk.
    pub unsaved: Vec<String>,
}

impl ReplaceReport {
    pub fn summary(&self) -> String {
        let mut text = format!(
            "Replaced {} match{} in {} file{}",
            self.replaced,
            if self.replaced == 1 { "" } else { "es" },
            self.files,
            if self.files == 1 { "" } else { "s" },
        );
        if self.stale > 0 {
            text.push_str(&format!(
                "; skipped {} stale (file changed since the search)",
                self.stale
            ));
        }
        if !self.unsaved.is_empty() {
            text.push_str(&format!(
                "; NOT saved (still modified): {}",
                self.unsaved.join(", ")
            ));
        }
        text
    }
}

pub struct SearchReplacePanel {
    pub find: SingleLineInput,
    pub replace: SingleLineInput,
    pub files: SingleLineInput,
    pub focus: SearchReplaceField,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub root: PathBuf,
    pub results: Vec<ReviewFile>,
    /// Selected row in [`Self::rows`].
    pub selected: usize,
    pub truncated: bool,
    pub error: Option<String>,
    pub searching: bool,
    /// A search has finished for the current inputs.
    pub searched: bool,
    dirty_at: Option<Instant>,
    job: Option<SearchJob>,
    /// Matches the user unchecked, keyed so the choice survives a re-search.
    unchecked: HashSet<(PathBuf, usize, usize)>,
    options: SearchOptions,
}

impl SearchReplacePanel {
    pub fn new(root: PathBuf) -> Self {
        Self {
            find: SingleLineInput::default(),
            replace: SingleLineInput::default(),
            files: SingleLineInput::default(),
            focus: SearchReplaceField::Find,
            regex: false,
            case_sensitive: false,
            whole_word: false,
            root,
            results: Vec::new(),
            selected: 0,
            truncated: false,
            error: None,
            searching: false,
            searched: false,
            dirty_at: None,
            job: None,
            unchecked: HashSet::new(),
            options: SearchOptions::default(),
        }
    }

    /// The options implied by the current inputs.
    pub fn current_options(&self) -> SearchOptions {
        SearchOptions {
            pattern: self.find.text().to_string(),
            regex: self.regex,
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            globs: self.files.text().to_string(),
        }
    }

    /// Requests a (debounced) new search after an input or toggle changed.
    pub fn mark_dirty(&mut self) {
        self.dirty_at = Some(Instant::now());
    }

    /// Runs the search right away (used by the ex command and tests).
    pub fn mark_dirty_now(&mut self) {
        self.dirty_at = Some(Instant::now() - SEARCH_DEBOUNCE * 2);
    }

    pub fn is_pending(&self) -> bool {
        self.dirty_at.is_some() || self.job.is_some()
    }

    /// Whether the inputs changed and have been quiet long enough to search.
    fn search_due(&self) -> bool {
        self.dirty_at
            .is_some_and(|started| started.elapsed() >= SEARCH_DEBOUNCE)
    }

    pub fn active_input_mut(&mut self) -> Option<&mut SingleLineInput> {
        match self.focus {
            SearchReplaceField::Find => Some(&mut self.find),
            SearchReplaceField::Replace => Some(&mut self.replace),
            SearchReplaceField::Files => Some(&mut self.files),
            SearchReplaceField::Results => None,
        }
    }

    pub fn next_field(&mut self) {
        self.focus = match self.focus {
            SearchReplaceField::Find => SearchReplaceField::Replace,
            SearchReplaceField::Replace => SearchReplaceField::Files,
            SearchReplaceField::Files => SearchReplaceField::Results,
            SearchReplaceField::Results => SearchReplaceField::Find,
        };
    }

    pub fn previous_field(&mut self) {
        self.focus = match self.focus {
            SearchReplaceField::Find => SearchReplaceField::Results,
            SearchReplaceField::Replace => SearchReplaceField::Find,
            SearchReplaceField::Files => SearchReplaceField::Replace,
            SearchReplaceField::Results => SearchReplaceField::Files,
        };
    }

    /// Visible rows: a header per file followed by its matches.
    pub fn rows(&self) -> Vec<ReviewRow> {
        let mut rows = Vec::new();
        for (file_index, file) in self.results.iter().enumerate() {
            rows.push(ReviewRow::File(file_index));
            for match_index in 0..file.matches.len() {
                rows.push(ReviewRow::Match(file_index, match_index));
            }
        }
        rows
    }

    pub fn row_count(&self) -> usize {
        self.results.iter().map(|file| 1 + file.matches.len()).sum()
    }

    pub fn selected_row(&self) -> Option<ReviewRow> {
        self.rows().get(self.selected).copied()
    }

    pub fn move_selection(&mut self, delta: isize) {
        let count = self.row_count();
        if count == 0 {
            self.selected = 0;
            return;
        }
        let next = (self.selected as isize + delta).clamp(0, count as isize - 1);
        self.selected = next as usize;
    }

    pub fn total_matches(&self) -> usize {
        self.results.iter().map(|file| file.matches.len()).sum()
    }

    pub fn checked_matches(&self) -> usize {
        self.results.iter().map(ReviewFile::checked_count).sum()
    }

    pub fn checked_files(&self) -> usize {
        self.results
            .iter()
            .filter(|file| file.checked_count() > 0)
            .count()
    }

    fn set_checked(&mut self, file: usize, m: usize, checked: bool) {
        let Some(file_entry) = self.results.get_mut(file) else {
            return;
        };
        let key = (
            file_entry.path.clone(),
            file_entry.matches[m].found.line,
            file_entry.matches[m].found.start_col,
        );
        file_entry.matches[m].checked = checked;
        if checked {
            self.unchecked.remove(&key);
        } else {
            self.unchecked.insert(key);
        }
    }

    /// Toggles the selected match, or every match of the selected file.
    pub fn toggle_selected(&mut self) {
        match self.selected_row() {
            Some(ReviewRow::Match(file, m)) => {
                let now = !self.results[file].matches[m].checked;
                self.set_checked(file, m, now);
            }
            Some(ReviewRow::File(file)) => {
                let now = self.results[file].checked_count() < self.results[file].matches.len();
                for m in 0..self.results[file].matches.len() {
                    self.set_checked(file, m, now);
                }
            }
            None => {}
        }
    }

    /// Checks everything, or unchecks everything if all are already checked.
    pub fn toggle_all(&mut self) {
        let now = self.checked_matches() < self.total_matches();
        for file in 0..self.results.len() {
            for m in 0..self.results[file].matches.len() {
                self.set_checked(file, m, now);
            }
        }
    }

    /// The text that will replace `found` (previews the capture expansion).
    pub fn replacement_for(&self, found: &LineMatch) -> String {
        match self.options.build_regex() {
            Ok(regex) => project_search::expand_replacement(
                &regex,
                &self.options,
                found,
                self.replace.text(),
            ),
            Err(_) => self.replace.text().to_string(),
        }
    }

    fn accept_outcome(&mut self, outcome: project_search::SearchOutcome) {
        let selected_key = self.selected_row().and_then(|row| match row {
            ReviewRow::Match(file, m) => {
                let file = &self.results[file];
                Some((file.path.clone(), file.matches[m].found.line))
            }
            ReviewRow::File(_) => None,
        });
        self.truncated = outcome.truncated;
        self.results = outcome
            .files
            .into_iter()
            .map(|FileMatches { path, rel, matches }| ReviewFile {
                matches: matches
                    .into_iter()
                    .map(|found| ReviewMatch {
                        checked: !self.unchecked.contains(&(
                            path.clone(),
                            found.line,
                            found.start_col,
                        )),
                        found,
                    })
                    .collect(),
                path,
                rel,
            })
            .collect();
        self.selected = 0;
        if let Some((path, line)) = selected_key {
            let rows = self.rows();
            if let Some(position) = rows.iter().position(|row| match row {
                ReviewRow::Match(file, m) => {
                    self.results[*file].path == path
                        && self.results[*file].matches[*m].found.line == line
                }
                ReviewRow::File(_) => false,
            }) {
                self.selected = position;
            }
        }
    }
}

impl Drop for SearchReplacePanel {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}

/// Char column -> UTF-16 column within `line`.
fn utf16_col(line: &str, char_col: usize) -> u32 {
    line.chars()
        .take(char_col)
        .map(|c| c.len_utf16() as u32)
        .sum()
}

impl Editor {
    pub fn search_replace_panel(&self) -> Option<&SearchReplacePanel> {
        self.ui_panels.search_replace.as_deref()
    }

    pub fn search_replace_panel_mut(&mut self) -> Option<&mut SearchReplacePanel> {
        self.ui_panels.search_replace.as_deref_mut()
    }

    /// Opens the panel (or brings back the last one) and focuses the find field.
    ///
    /// `prefill` seeds the find field (e.g. the word under the cursor).
    pub fn open_search_replace(&mut self, prefill: Option<String>) {
        let root = self.picker_dirs().0;
        if self.ui_panels.search_replace.is_none() {
            self.ui_panels.search_replace = Some(Box::new(SearchReplacePanel::new(root)));
        }
        if let Some(panel) = self.ui_panels.search_replace.as_mut() {
            if let Some(text) = prefill.filter(|text| !text.is_empty()) {
                panel.find = SingleLineInput::new(text);
                panel.find.move_end();
                panel.mark_dirty_now();
            }
            panel.focus = SearchReplaceField::Find;
        }
        self.set_mode(Mode::SearchReplace);
        self.mark_dirty();
    }

    /// Hides the panel but keeps its state for `:SearchReplace`.
    pub fn close_search_replace(&mut self) {
        if self.mode == Mode::SearchReplace {
            self.set_mode(Mode::Normal);
        }
        self.mark_dirty();
    }

    /// Discards the panel state.
    pub fn discard_search_replace(&mut self) {
        self.ui_panels.search_replace = None;
        self.close_search_replace();
    }

    pub fn is_search_replace_open(&self) -> bool {
        self.mode == Mode::SearchReplace && self.ui_panels.search_replace.is_some()
    }

    /// Text of open, file-backed buffers keyed by absolute path.
    pub(crate) fn open_buffer_overlays(&self) -> HashMap<PathBuf, String> {
        let mut overlays = HashMap::new();
        for buffer in &self.buffers {
            let Some(path) = buffer.file_path() else {
                continue;
            };
            if path.starts_with('[') {
                continue;
            }
            let path = Path::new(path);
            let Ok(canonical) = path.canonicalize() else {
                continue;
            };
            overlays.insert(canonical, buffer.rope().to_string());
        }
        overlays
    }

    /// Whether the debounce elapsed and a search starts now, with the open
    /// buffers' text for it. Copying every buffer is only worth it for a
    /// search that starts now, not on each tick while the user is typing.
    fn overlays_for_due_search(&self) -> (bool, HashMap<PathBuf, String>) {
        let due = self
            .ui_panels
            .search_replace
            .as_ref()
            .is_some_and(|panel| panel.search_due());
        let overlays = if due {
            self.open_buffer_overlays()
        } else {
            HashMap::new()
        };
        (due, overlays)
    }

    /// Drives the debounce and collects finished searches. Returns true when
    /// the panel content changed.
    pub fn poll_search_replace(&mut self) -> bool {
        let (search_due, overlays) = self.overlays_for_due_search();
        let Some(panel) = self.ui_panels.search_replace.as_mut() else {
            return false;
        };
        let mut changed = false;

        if let Some(job) = &panel.job {
            match job.rx.try_recv() {
                Ok(result) => {
                    panel.job = None;
                    panel.searching = false;
                    panel.searched = true;
                    match result {
                        Ok(outcome) if !outcome.cancelled => {
                            panel.error = None;
                            panel.accept_outcome(outcome);
                        }
                        Ok(_) => {}
                        Err(message) => {
                            panel.error = Some(message);
                            panel.results.clear();
                            panel.selected = 0;
                        }
                    }
                    changed = true;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    panel.job = None;
                    panel.searching = false;
                    changed = true;
                }
            }
        }

        if search_due {
            panel.dirty_at = None;
            if let Some(job) = panel.job.take() {
                job.cancel.store(true, Ordering::Relaxed);
            }
            panel.options = panel.current_options();
            if panel.options.pattern.is_empty() {
                panel.results.clear();
                panel.selected = 0;
                panel.error = None;
                panel.searching = false;
                panel.searched = false;
                panel.truncated = false;
            } else {
                // Compile errors show immediately, without a thread.
                match panel.options.build_regex() {
                    Err(message) => {
                        panel.error = Some(message);
                        panel.results.clear();
                        panel.selected = 0;
                        panel.searched = true;
                    }
                    Ok(_) => {
                        let cancel = Arc::new(AtomicBool::new(false));
                        let (tx, rx) = mpsc::channel();
                        let options = panel.options.clone();
                        let root = panel.root.clone();
                        let thread_cancel = cancel.clone();
                        std::thread::spawn(move || {
                            let _ = tx.send(project_search::search_project(
                                &root,
                                &options,
                                &overlays,
                                &thread_cancel,
                            ));
                        });
                        panel.job = Some(SearchJob { rx, cancel });
                        panel.searching = true;
                        panel.error = None;
                    }
                }
            }
            changed = true;
        }
        if changed {
            self.mark_dirty();
        }
        changed
    }

    /// True when the current buffer has unsaved changes (`:update`).
    pub fn current_buffer_needs_write(&self) -> bool {
        self.buffer_index_is_modified(self.current_buffer_index)
    }

    /// Runs the panel's search synchronously (ex-command form and tests).
    pub fn run_search_replace_now(&mut self) {
        let overlays = self.open_buffer_overlays();
        let Some(panel) = self.ui_panels.search_replace.as_mut() else {
            return;
        };
        if let Some(job) = panel.job.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        panel.dirty_at = None;
        panel.options = panel.current_options();
        panel.searching = false;
        panel.searched = true;
        match project_search::search_project(
            &panel.root,
            &panel.options,
            &overlays,
            &AtomicBool::new(false),
        ) {
            Ok(outcome) => {
                panel.error = None;
                panel.accept_outcome(outcome);
            }
            Err(message) => {
                panel.error = Some(message);
                panel.results.clear();
                panel.selected = 0;
            }
        }
        self.mark_dirty();
    }

    /// Applies every checked match. Each touched file is edited through its
    /// own buffer (undoable with `u`), then saved — the ex-command equivalent
    /// is `:cfdo s/x/y/ | update`. Files that were not open are loaded as
    /// hidden buffers so the change is still one undo step away.
    pub fn apply_search_replace(&mut self) -> Result<ReplaceReport, String> {
        let Some(panel) = self.ui_panels.search_replace.as_ref() else {
            return Err("No replace in files review is open".to_string());
        };
        if let Some(error) = &panel.error {
            return Err(format!("Cannot replace: {error}"));
        }
        if panel.checked_matches() == 0 {
            return Err("No matches are checked".to_string());
        }
        let options = panel.options.clone();
        let regex = options.build_regex()?;
        let template = panel.replace.text().to_string();

        struct PlannedFile {
            path: PathBuf,
            edits: Vec<(LineMatch, String)>,
        }
        let plan: Vec<PlannedFile> = panel
            .results
            .iter()
            .filter(|file| file.checked_count() > 0)
            .map(|file| PlannedFile {
                path: file.path.clone(),
                edits: file
                    .matches
                    .iter()
                    .filter(|m| m.checked)
                    .map(|m| {
                        (
                            m.found.clone(),
                            project_search::expand_replacement(
                                &regex, &options, &m.found, &template,
                            ),
                        )
                    })
                    .collect(),
            })
            .collect();

        let mut report = ReplaceReport::default();
        let mut touched: Vec<(crate::buffer::BufferId, u64)> = Vec::new();

        for file in plan {
            let Some(uri) = crate::lsp::uri_from_file_path(&file.path) else {
                report.stale += file.edits.len();
                continue;
            };
            let Some(index) = self.find_or_load_buffer_index_by_uri(&uri) else {
                report.stale += file.edits.len();
                continue;
            };

            // Re-verify against the live buffer text: only edit lines that
            // still read exactly what the reviewer saw.
            let mut edits = Vec::new();
            for (found, replacement) in &file.edits {
                let live = self.buffers[index].line_text(found.line);
                let fresh = live
                    .as_deref()
                    .map(|text| text.trim_end_matches(['\n', '\r']))
                    .is_some_and(|text| text.starts_with(found.line_text.as_str()));
                if !fresh {
                    report.stale += 1;
                    continue;
                }
                edits.push(lsp_types::TextEdit {
                    range: lsp_types::Range {
                        start: lsp_types::Position {
                            line: found.line as u32,
                            character: utf16_col(&found.line_text, found.start_col),
                        },
                        end: lsp_types::Position {
                            line: found.line as u32,
                            character: utf16_col(&found.line_text, found.end_col),
                        },
                    },
                    new_text: replacement.clone(),
                });
            }
            if edits.is_empty() {
                continue;
            }
            let count = edits.len();
            if !self.apply_lsp_edits_to_buffer_index(index, edits) {
                report.stale += count;
                continue;
            }
            report.replaced += count;
            report.files += 1;
            // Remember which undo entry is ours so `:ReplaceUndo` never undoes
            // something the user did afterwards (or already undid with `u`).
            if let Some(entry) = self.buffers[index].change_manager().undo_stack.last() {
                touched.push((self.buffers[index].id(), entry.seq));
            }

            // Save-all semantics, like `| update`. Only files the review
            // itself wrote: a buffer that already held the user's unsaved work
            // is saved too (it is the same buffer the edit went into), and a
            // failed write leaves it modified and reported.
            if self.buffer_index_is_modified(index)
                && !self.write_through_workspace_edit_buffer(index)
            {
                report.unsaved.push(
                    file.path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("?")
                        .to_string(),
                );
            }
        }

        self.ui_panels.last_replace_buffers = touched;
        self.request_diagnostics_refresh();
        self.mark_dirty();
        Ok(report)
    }

    /// Undoes the last "replace in files" in every buffer it touched.
    ///
    /// A buffer is only undone while the replacement is still the newest
    /// change in it; buffers the user has edited (or undone with `u`) since
    /// are left alone. Returns `(undone, skipped)` buffer counts.
    pub fn undo_last_search_replace(&mut self) -> Result<(usize, usize), String> {
        let entries = std::mem::take(&mut self.ui_panels.last_replace_buffers);
        if entries.is_empty() {
            return Err("No replace in files to undo".to_string());
        }
        let (mut undone, mut skipped) = (0, 0);
        for (id, seq) in entries {
            let Some(index) = self.buffers.iter().position(|b| b.id() == id) else {
                skipped += 1;
                continue;
            };
            let is_top = self.buffers[index]
                .change_manager()
                .undo_stack
                .last()
                .is_some_and(|entry| entry.seq == seq);
            if !is_top {
                skipped += 1;
                continue;
            }
            let (outcome, _) = self.buffers[index].undo();
            if outcome.is_done() {
                undone += 1;
                if self.buffer_index_is_modified(index) {
                    let _ = self.write_through_workspace_edit_buffer(index);
                }
                if let Some(path) = self.buffers[index].file_path().map(str::to_string) {
                    self.lsp
                        .state
                        .document_sync
                        .entry(path)
                        .or_default()
                        .mark_modified();
                }
            } else {
                skipped += 1;
            }
        }
        self.request_diagnostics_refresh();
        self.mark_dirty();
        Ok((undone, skipped))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An editor with a real file open (overlays skip buffers that do not
    /// exist on disk) and an empty replace-in-files panel.
    fn editor_with_panel() -> (Editor, tempfile::TempDir) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("big.txt");
        std::fs::write(&path, "needle\n").unwrap();
        let mut editor = Editor::with_content(&"needle in a haystack\n".repeat(1000));
        editor.set_file_path(path.to_string_lossy().to_string());
        editor.open_search_replace(None);
        (editor, directory)
    }

    /// The panel polled every tick during its 180 ms debounce and copied each
    /// open buffer into a string on every one of them.
    #[test]
    fn open_buffers_are_copied_only_for_a_search_that_starts() {
        let (mut editor, _directory) = editor_with_panel();
        assert_eq!(editor.overlays_for_due_search(), (false, HashMap::new()));

        let panel = editor.search_replace_panel_mut().unwrap();
        panel.find = SingleLineInput::new("needle".to_string());
        panel.mark_dirty();
        let (due, overlays) = editor.overlays_for_due_search();
        assert!(!due, "still inside the debounce");
        assert!(overlays.is_empty(), "no buffer copied while typing");

        editor.search_replace_panel_mut().unwrap().mark_dirty_now();
        let (due, overlays) = editor.overlays_for_due_search();
        assert!(due);
        assert_eq!(overlays.len(), 1, "the open file is passed to the search");
        assert!(overlays
            .values()
            .next()
            .unwrap()
            .starts_with("needle in a haystack"));
    }

    /// A due search starts once and sees text that is only in the open buffer.
    #[test]
    fn a_due_search_sees_unsaved_buffer_text() {
        let (mut editor, _directory) = editor_with_panel();
        let panel = editor.search_replace_panel_mut().unwrap();
        // The file on disk only says "needle"; "haystack" is unsaved.
        panel.find = SingleLineInput::new("haystack".to_string());
        panel.mark_dirty_now();
        assert!(editor.poll_search_replace());
        assert!(editor.search_replace_panel().unwrap().dirty_at.is_none());
        let started = Instant::now();
        while !editor.search_replace_panel().unwrap().searched {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "search never finished"
            );
            editor.poll_search_replace();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(editor.search_replace_panel().unwrap().total_matches() > 0);
    }
}
