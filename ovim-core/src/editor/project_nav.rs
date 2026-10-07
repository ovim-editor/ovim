//! Project navigation pickers: recent files and open buffers.
//!
//! The "most recently used" order lives in a session-local list that the
//! shared tick keeps current (`track_recent_file`); when a frontend enabled
//! persistence it is also written to the per-project store so it survives
//! restarts.

use super::picker::{Picker, PickerResult};
use super::Editor;
use crate::mode::Mode;
use crate::project_root::vcs_root_or_dir;
use crate::recent_files::RecentFiles;
use std::path::{Path, PathBuf};

/// Session state for recent-file tracking.
#[derive(Default)]
pub struct RecentTracker {
    pub store: Option<RecentFiles>,
    /// The file that was current on the previous tick.
    last: Option<PathBuf>,
    /// `file_path()` string that `last` was resolved from (avoids a
    /// canonicalize per tick while nothing changed).
    last_raw: Option<String>,
    /// Files visited this session, most recent first.
    pub visits: Vec<PathBuf>,
    /// Cursor of the current file as of the last store write, and when.
    flushed: Option<((usize, usize), std::time::Instant)>,
}

/// The remembered cursor of the current file is refreshed at most this often.
const CURSOR_FLUSH: std::time::Duration = std::time::Duration::from_secs(2);

/// The columns of one workspace-symbol row: the name is `PickerResult::display`,
/// the rest is derived here so the TUI and GUI agree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRowColumns {
    /// Kind label (`class`, `method`, ...).
    pub kind: String,
    /// Nerd Font glyph for the kind.
    pub glyph: &'static str,
    /// Enclosing type/package, when the server sent one.
    pub container: Option<String>,
    /// `relative/path.java:LINE`.
    pub file: String,
}

/// Splits a workspace-symbol picker row into its columns.
pub fn symbol_row_columns(result: &PickerResult, root: &Path) -> SymbolRowColumns {
    let detail = result.content.as_deref().unwrap_or_default();
    let (kind, container) = match detail.split_once(" · ") {
        Some((kind, container)) => (kind, Some(container.to_string())),
        None => (detail, None),
    };
    SymbolRowColumns {
        glyph: symbol_kind_glyph(kind),
        kind: kind.to_string(),
        container,
        file: format!(
            "{}:{}",
            relative_display(root, Path::new(&result.location)),
            result.line + 1
        ),
    }
}

/// Codicon glyph for a symbol kind label (see `symbol_kind_str`).
fn symbol_kind_glyph(kind: &str) -> &'static str {
    match kind {
        "class" | "struct" => "\u{eb5b}",
        "interface" => "\u{eb61}",
        "enum" => "\u{ea95}",
        "enum_member" => "\u{eb5e}",
        "method" | "function" | "constructor" => "\u{ea8c}",
        "field" | "property" => "\u{eb5f}",
        "variable" => "\u{ea88}",
        "constant" => "\u{eb5d}",
        "package" | "module" | "namespace" => "\u{ea8b}",
        _ => "\u{eb63}",
    }
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

impl Editor {
    /// Opts this editor into persisting recent files (frontends only).
    pub fn enable_recent_files(&mut self) {
        self.set_recent_files(RecentFiles::discover());
    }

    pub fn set_recent_files(&mut self, store: Option<RecentFiles>) {
        self.ui_panels.recent.store = store;
    }

    fn current_file_absolute(&self) -> Option<PathBuf> {
        let path = self.buffer().file_path()?;
        if super::buffer_manager::is_scratch_path(path) {
            return None;
        }
        Path::new(path).canonicalize().ok()
    }

    /// Notes when the current file changed since the last call. Cheap enough
    /// for every tick: one path comparison when nothing changed.
    pub fn track_recent_file(&mut self) {
        let raw = self.buffer().file_path().map(str::to_string);
        if raw.is_some() && raw == self.ui_panels.recent.last_raw {
            if let Some(current) = self.ui_panels.recent.last.clone() {
                self.flush_recent_cursor(&current);
                return;
            }
        }
        let Some(current) = self.current_file_absolute() else {
            return;
        };
        self.ui_panels.recent.last_raw = raw;
        if self.ui_panels.recent.last.as_ref() == Some(&current) {
            self.flush_recent_cursor(&current);
            return;
        }
        // Remember where the cursor was in the file we are leaving.
        if let Some(previous) = self.ui_panels.recent.last.take() {
            if let Some(store) = &self.ui_panels.recent.store {
                let cursor = self
                    .find_buffer_by_path(&previous.to_string_lossy())
                    .and_then(|index| self.buffers.get(index))
                    .map(|buffer| (buffer.cursor().line(), buffer.cursor().col().0));
                if let Some(cursor) = cursor {
                    store.update_cursor(&vcs_root_or_dir(&previous), &previous, cursor);
                }
            }
        }
        if let Some(store) = &self.ui_panels.recent.store {
            store.record(&vcs_root_or_dir(&current), &current, None);
        }
        let visits = &mut self.ui_panels.recent.visits;
        visits.retain(|path| path != &current);
        visits.insert(0, current.clone());
        visits.truncate(200);
        self.ui_panels.recent.flushed = None;
        self.ui_panels.recent.last = Some(current);
    }

    /// Keeps the stored cursor of the current file fresh, so a session that
    /// ends (or crashes) without switching files still reopens at the right spot.
    fn flush_recent_cursor(&mut self, current: &Path) {
        let Some(store) = self.ui_panels.recent.store.clone() else {
            return;
        };
        let cursor = (
            self.buffer().cursor().line(),
            self.buffer().cursor().col().0,
        );
        let recent = &mut self.ui_panels.recent;
        match recent.flushed {
            Some((flushed, _)) if flushed == cursor => {}
            Some((_, at)) if at.elapsed() < CURSOR_FLUSH => {}
            _ => {
                store.update_cursor(&vcs_root_or_dir(current), current, cursor);
                recent.flushed = Some((cursor, std::time::Instant::now()));
            }
        }
    }

    fn project_root_for_pickers(&self) -> PathBuf {
        self.picker_dirs().0
    }

    /// Files the user opened in this project, most recent first.
    pub fn recent_file_results(&mut self) -> Vec<PickerResult> {
        self.track_recent_file();
        let root = self.project_root_for_pickers();
        let current = self.current_file_absolute();
        let mut entries: Vec<(PathBuf, usize, usize)> = match &self.ui_panels.recent.store {
            Some(store) => store
                .list(&root)
                .into_iter()
                .map(|entry| (PathBuf::from(entry.path), entry.line, entry.col))
                .collect(),
            None => Vec::new(),
        };
        // Files opened this session that the store has not seen (persistence
        // off, or another project's file that lives under this root).
        for visited in &self.ui_panels.recent.visits {
            if visited.starts_with(&root) && !entries.iter().any(|(path, _, _)| path == visited) {
                entries.push((visited.clone(), 0, 0));
            }
        }
        // The session order is the freshest; float this session's visits up.
        let visits = self.ui_panels.recent.visits.clone();
        entries.sort_by_key(|(path, _, _)| {
            visits
                .iter()
                .position(|visited| visited == path)
                .unwrap_or(usize::MAX)
        });
        entries
            .into_iter()
            .filter(|(path, _, _)| Some(path) != current.as_ref())
            .map(|(path, line, col)| PickerResult {
                display: relative_display(&root, &path),
                location: path.to_string_lossy().to_string(),
                line,
                col,
                match_positions: Vec::new(),
                content: None,
            })
            .collect()
    }

    /// `<Space>sh` / `:Recent` — recently opened files of this project.
    pub fn open_recent_files_picker(&mut self) {
        let items = self.recent_file_results();
        if items.is_empty() {
            self.set_status_message("No recent files in this project yet");
            return;
        }
        self.open_location_picker(items, "Recent files");
    }

    /// `<Space>sb` / `:Buffers` — open file buffers, most recently used first.
    pub fn open_buffer_picker(&mut self) {
        self.track_recent_file();
        let root = self.project_root_for_pickers();
        let current = self.current_buffer_index();
        let visits = self.ui_panels.recent.visits.clone();
        let mut rows: Vec<(usize, PathBuf, bool, bool)> = Vec::new();
        for (index, buffer) in self.buffers.iter().enumerate() {
            let Some(path) = buffer.file_path() else {
                continue;
            };
            if super::buffer_manager::is_scratch_buffer(buffer) {
                continue;
            }
            let absolute = Path::new(path)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(path));
            rows.push((
                index,
                absolute,
                index == current,
                self.buffer_index_is_modified(index),
            ));
        }
        if rows.is_empty() {
            self.set_status_message("No open file buffers");
            return;
        }
        rows.sort_by_key(|(index, path, is_current, _)| {
            (
                !*is_current,
                visits
                    .iter()
                    .position(|visited| visited == path)
                    .unwrap_or(usize::MAX),
                *index,
            )
        });
        let items = rows
            .into_iter()
            .map(|(index, path, is_current, modified)| {
                let cursor = self.buffers[index].cursor();
                PickerResult {
                    display: format!(
                        "{}{}{}",
                        relative_display(&root, &path),
                        if modified { " [+]" } else { "" },
                        if is_current { "  (current)" } else { "" },
                    ),
                    location: path.to_string_lossy().to_string(),
                    line: cursor.line(),
                    col: cursor.col().0,
                    match_positions: Vec::new(),
                    content: None,
                }
            })
            .collect();
        self.open_location_picker(items, "Open buffers");
    }

    /// Picker rows for a `workspace/symbol` answer: the name, with `kind ·
    /// container` as the detail and the file drawn by the frontends.
    pub(crate) fn workspace_symbol_items(
        &self,
        symbols: &[lsp_types::SymbolInformation],
    ) -> Vec<PickerResult> {
        let mut columns = super::lsp_columns::ColumnResolver::new(self);
        symbols
            .iter()
            .filter_map(|symbol| {
                let path = crate::lsp::uri_to_file_path(&symbol.location.uri)?;
                let line = symbol.location.range.start.line as usize;
                let col = columns.grapheme_col(&path, line, symbol.location.range.start.character);
                let kind =
                    super::lsp_integration::lsp_modules::navigation::symbol_kind_str(symbol.kind);
                let container = symbol
                    .container_name
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .map(|name| format!(" · {name}"))
                    .unwrap_or_default();
                Some(PickerResult {
                    // Name only: it is what the query matches and what stays
                    // readable however long the path is. Kind, container and
                    // file are separate columns (`symbol_row_detail`).
                    display: symbol.name.clone(),
                    location: path.to_string_lossy().to_string(),
                    line,
                    col,
                    match_positions: Vec::new(),
                    content: Some(format!("{kind}{container}")),
                })
            })
            .take(200)
            .collect()
    }

    /// `<Space>S` / `:Symbols` — workspace symbols, re-queried as you type.
    pub fn open_workspace_symbol_picker(&mut self) {
        self.lsp.state.hierarchy = None;
        let root = self.project_root_for_pickers();
        self.set_picker(Picker::new_workspace_symbols(root));
        self.set_mode(Mode::Picker);
        self.set_lsp_status("Workspace symbols: type to search".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::picker::PickerAction;
    use std::fs;

    struct Project {
        _dir: tempfile::TempDir,
        root: PathBuf,
        store: PathBuf,
    }

    fn project(files: &[&str]) -> Project {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir(root.join(".git")).unwrap();
        for file in files {
            fs::write(root.join(file), "line one\nline two\nline three\n").unwrap();
        }
        let store = root.join("recent-store.json");
        Project {
            _dir: dir,
            root,
            store,
        }
    }

    fn editor_for(project: &Project) -> Editor {
        let mut editor = Editor::default();
        editor.set_recent_files(Some(RecentFiles::new(project.store.clone())));
        editor
    }

    fn results(editor: &Editor) -> Vec<String> {
        editor
            .picker()
            .expect("picker open")
            .collect_filtered_results(100)
            .into_iter()
            .map(|result| result.display.clone())
            .collect()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn recent_files_are_mru_per_project_survive_restart_and_restore_the_cursor() {
        let project = project(&["a.txt", "b.txt", "c.txt", "d.txt"]);
        {
            let mut editor = editor_for(&project);
            for (file, line) in [("a.txt", 2), ("b.txt", 1), ("c.txt", 0)] {
                editor.load_file(project.root.join(file)).unwrap();
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(line, crate::unicode::GraphemeCol(3));
                editor.track_recent_file();
            }
        }

        // "Restart": a new editor with the same store, in the same project.
        let mut editor = editor_for(&project);
        editor.load_file(project.root.join("d.txt")).unwrap();
        editor.open_recent_files_picker();
        assert_eq!(results(&editor), vec!["c.txt", "b.txt", "a.txt"]);
        assert_eq!(
            editor.picker().unwrap().title(),
            Some("Recent files"),
            "the picker names itself"
        );

        // The cursor a file was left at is remembered (a.txt was left at 2:3).
        let target = editor
            .picker()
            .unwrap()
            .collect_filtered_results(10)
            .into_iter()
            .find(|result| result.display == "a.txt")
            .cloned()
            .unwrap();
        assert_eq!((target.line, target.col), (2, 3));

        // Selecting an entry opens it there.
        editor
            .execute_picker_action(PickerAction::OpenFileWithTag {
                path: target.location.clone(),
                line: target.line,
                col: target.col,
            })
            .unwrap();
        assert!(editor.buffer().file_path().unwrap().ends_with("a.txt"));
        assert_eq!(editor.cursor_position().line, 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn the_cursor_of_the_current_file_is_remembered_without_switching_files() {
        let project = project(&["a.txt", "b.txt"]);
        {
            let mut editor = editor_for(&project);
            editor.load_file(project.root.join("a.txt")).unwrap();
            editor.track_recent_file();
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(2, crate::unicode::GraphemeCol(5));
            // A later tick, session then ends (or crashes) in a.txt.
            editor.track_recent_file();
        }
        let mut editor = editor_for(&project);
        editor.load_file(project.root.join("b.txt")).unwrap();
        editor.open_recent_files_picker();
        let entry = editor.picker().unwrap().filtered_result(0).unwrap().clone();
        assert_eq!(entry.display, "a.txt");
        assert_eq!((entry.line, entry.col), (2, 5));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn recent_files_are_scoped_to_the_current_project() {
        let one = project(&["a.txt"]);
        let two = project(&["b.txt"]);
        let store = one.store.clone();
        let mut editor = Editor::default();
        editor.set_recent_files(Some(RecentFiles::new(store.clone())));
        editor.load_file(one.root.join("a.txt")).unwrap();
        editor.track_recent_file();
        editor.load_file(two.root.join("b.txt")).unwrap();
        editor.track_recent_file();

        let mut fresh = Editor::default();
        fresh.set_recent_files(Some(RecentFiles::new(store)));
        fs::write(one.root.join("z.txt"), "z").unwrap();
        fresh.load_file(one.root.join("z.txt")).unwrap();
        fresh.open_recent_files_picker();
        assert_eq!(
            results(&fresh),
            vec!["a.txt"],
            "b.txt belongs to another project"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn buffer_picker_lists_current_first_then_mru_and_flags_modified() {
        let project = project(&["a.txt", "b.txt", "c.txt"]);
        let mut editor = editor_for(&project);
        for file in ["a.txt", "b.txt", "c.txt"] {
            editor.load_file(project.root.join(file)).unwrap();
            editor.track_recent_file();
        }
        editor.load_file(project.root.join("a.txt")).unwrap();
        editor.track_recent_file();
        // Modify b.txt through its own buffer.
        editor.load_file(project.root.join("b.txt")).unwrap();
        editor.track_recent_file();
        editor
            .buffer_mut()
            .insert_text_at(0, crate::unicode::CharCol(0), "x");
        editor.mark_buffer_modified_force_send();

        editor.open_buffer_picker();
        assert_eq!(
            results(&editor),
            vec!["b.txt [+]  (current)", "a.txt", "c.txt"]
        );
        assert_eq!(editor.picker().unwrap().title(), Some("Open buffers"));
    }

    /// OV-00452: a symbol row is name + kind + container + file, and stays
    /// in the server's order.
    #[test]
    fn workspace_symbol_rows_carry_name_kind_container_and_file() {
        let editor = Editor::default();
        #[allow(deprecated)]
        let symbol = |name: &str, kind, container: Option<&str>, path: &str, line: u32| {
            lsp_types::SymbolInformation {
                name: name.to_string(),
                kind,
                tags: None,
                deprecated: None,
                location: lsp_types::Location {
                    uri: crate::lsp::uri_from_file_path(path).unwrap(),
                    range: lsp_types::Range::new(
                        lsp_types::Position::new(line, 13),
                        lsp_types::Position::new(line, 19),
                    ),
                },
                container_name: container.map(str::to_string),
            }
        };
        let rows = editor.workspace_symbol_items(&[
            symbol(
                "CustomerService",
                lsp_types::SymbolKind::CLASS,
                Some("com.pay.service"),
                "/proj/src/CustomerService.java",
                18,
            ),
            symbol(
                "createCustomer",
                lsp_types::SymbolKind::METHOD,
                Some("CustomerService"),
                "/proj/src/CustomerService.java",
                22,
            ),
        ]);
        // Server order kept, name is the display text.
        assert_eq!(rows[0].display, "CustomerService");
        assert_eq!(rows[1].display, "createCustomer");
        assert_eq!(rows[0].content.as_deref(), Some("class · com.pay.service"));
        assert_eq!((rows[1].line, rows[1].col), (22, 13));

        let columns = symbol_row_columns(&rows[1], Path::new("/proj"));
        assert_eq!(columns.kind, "method");
        assert_eq!(columns.container.as_deref(), Some("CustomerService"));
        assert_eq!(columns.file, "src/CustomerService.java:23");
        assert_ne!(
            columns.glyph,
            symbol_row_columns(&rows[0], Path::new("/proj")).glyph
        );
    }

    #[test]
    fn workspace_symbol_picker_asks_the_server_again_when_the_query_changes() {
        let mut picker = Picker::new_workspace_symbols(PathBuf::from("/tmp"));
        assert!(picker.is_symbol_search());
        assert_eq!(
            picker.take_symbol_query(),
            Some(String::new()),
            "initial query"
        );
        assert_eq!(picker.take_symbol_query(), None, "asked only once");

        picker.insert_char('C');
        picker.insert_char('i');
        picker.mark_filter_pending();
        assert!(picker.has_pending_filter());
        picker.apply_pending_filter();
        assert_eq!(picker.take_symbol_query(), Some("Ci".to_string()));

        // Results from the server are shown as delivered, never re-filtered.
        picker.set_results(vec![PickerResult {
            display: "Circle".to_string(),
            location: "/tmp/src/Circle.java".to_string(),
            line: 2,
            col: 13,
            match_positions: Vec::new(),
            content: Some("class · shapes".to_string()),
        }]);
        assert_eq!(picker.filtered_result_count(), 1);
        assert!(matches!(
            picker.selected_action(),
            Some(PickerAction::OpenFileWithTag {
                line: 2,
                col: 13,
                ..
            })
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn symbol_rows_show_name_kind_container_and_file() {
        let project = project(&["Circle.java"]);
        let mut editor = editor_for(&project);
        let path = project.root.join("Circle.java");
        editor.load_file(&path).unwrap();
        #[allow(deprecated)]
        let symbol = lsp_types::SymbolInformation {
            name: "area".to_string(),
            kind: lsp_types::SymbolKind::METHOD,
            tags: None,
            deprecated: None,
            location: lsp_types::Location {
                uri: crate::lsp::uri_from_file_path(&path).unwrap(),
                range: lsp_types::Range {
                    start: lsp_types::Position {
                        line: 4,
                        character: 11,
                    },
                    end: lsp_types::Position {
                        line: 4,
                        character: 15,
                    },
                },
            },
            container_name: Some("shapes.Circle".to_string()),
        };
        let items = editor.workspace_symbol_items(&[symbol]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].display, "area");
        assert_eq!(items[0].content.as_deref(), Some("method · shapes.Circle"));
        assert_eq!(
            symbol_row_columns(&items[0], &project.root).file,
            "Circle.java:5"
        );
        assert_eq!((items[0].line, items[0].col), (4, 11));
    }

    /// Every location picker (LSP references, recent files, ...) is titled
    /// and rooted at the project, not at the process working directory.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn location_pickers_keep_their_title_and_use_the_project_root() {
        let project = project(&["a.txt"]);
        fs::create_dir(project.root.join("sub")).unwrap();
        fs::write(project.root.join("sub/b.txt"), "x\n").unwrap();
        let mut editor = editor_for(&project);
        editor.load_file(project.root.join("sub/b.txt")).unwrap();
        editor.open_location_picker(Vec::new(), "References");
        let picker = editor.picker().unwrap();
        assert_eq!(picker.title(), Some("References"));
        assert_eq!(picker.base_dir(), project.root.as_path());
    }
}
