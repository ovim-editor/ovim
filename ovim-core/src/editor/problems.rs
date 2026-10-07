//! Problems view: every diagnostic the language servers have published, for
//! all files (not just the current buffer), grouped by file, with a severity
//! filter, and jump to the location.

use std::path::{Path, PathBuf};

use lsp_types::DiagnosticSeverity;

use super::picker::{Picker, PickerResult, PickerRole};
use super::Editor;
use crate::mode::Mode;

/// Which severities the view shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProblemFilter {
    #[default]
    All,
    /// Errors and warnings (no information / hints).
    WarningsUp,
    Errors,
}

impl ProblemFilter {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "" | "all" | "a" => Some(Self::All),
            "warnings" | "warning" | "warn" | "w" => Some(Self::WarningsUp),
            "errors" | "error" | "e" => Some(Self::Errors),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::WarningsUp => "errors and warnings",
            Self::Errors => "errors",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::All => Self::WarningsUp,
            Self::WarningsUp => Self::Errors,
            Self::Errors => Self::All,
        }
    }

    fn admits(self, severity: u8) -> bool {
        match self {
            Self::All => true,
            Self::WarningsUp => severity <= 2,
            Self::Errors => severity == 1,
        }
    }
}

/// One published diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProblemItem {
    pub path: PathBuf,
    /// 0-based.
    pub line: usize,
    pub col: usize,
    /// 1 error, 2 warning, 3 information, 4 hint.
    pub severity: u8,
    pub message: String,
    pub source: Option<String>,
}

/// The picker's current data, kept so the filter can change without asking
/// the servers again.
pub struct ProblemsState {
    pub items: Vec<ProblemItem>,
    pub filter: ProblemFilter,
    pub root: PathBuf,
}

fn severity_number(severity: Option<DiagnosticSeverity>) -> u8 {
    match severity {
        Some(DiagnosticSeverity::ERROR) | None => 1,
        Some(DiagnosticSeverity::WARNING) => 2,
        Some(DiagnosticSeverity::INFORMATION) => 3,
        _ => 4,
    }
}

fn severity_tag(severity: u8) -> &'static str {
    match severity {
        1 => "error",
        2 => "warn ",
        3 => "info ",
        _ => "hint ",
    }
}

/// Flattens the server answer into sorted items (by file, then position).
pub fn problems_from(
    all: Vec<(lsp_types::Uri, Vec<lsp_types::Diagnostic>, Vec<i32>)>,
) -> Vec<ProblemItem> {
    let mut items = Vec::new();
    for (uri, diagnostics, _) in all {
        let Some(path) = crate::lsp::uri_to_file_path(&uri) else {
            continue;
        };
        for diagnostic in diagnostics {
            items.push(ProblemItem {
                path: path.clone(),
                line: diagnostic.range.start.line as usize,
                col: diagnostic.range.start.character as usize,
                severity: severity_number(diagnostic.severity),
                message: diagnostic
                    .message
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                source: diagnostic.source,
            });
        }
    }
    items.sort_by(|a, b| {
        (&a.path, a.line, a.col, a.severity).cmp(&(&b.path, b.line, b.col, b.severity))
    });
    items
}

/// Picker rows: a header per file, then its problems indented below it.
/// Returns the rows and a `N errors, M warnings in K files` summary.
pub fn problem_rows(
    items: &[ProblemItem],
    filter: ProblemFilter,
    root: &Path,
) -> (Vec<PickerResult>, String) {
    let shown: Vec<&ProblemItem> = items
        .iter()
        .filter(|item| filter.admits(item.severity))
        .collect();
    let mut rows = Vec::new();
    let mut files = 0;
    let mut index = 0;
    while index < shown.len() {
        let path = &shown[index].path;
        let end = shown[index..]
            .iter()
            .position(|item| &item.path != path)
            .map(|offset| index + offset)
            .unwrap_or(shown.len());
        let group = &shown[index..end];
        files += 1;
        let errors = group.iter().filter(|item| item.severity == 1).count();
        let warnings = group.iter().filter(|item| item.severity == 2).count();
        let relative = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
        rows.push(PickerResult {
            display: format!(
                "{relative}  {} ({errors} errors, {warnings} warnings)",
                group.len()
            ),
            location: path.to_string_lossy().to_string(),
            line: group[0].line,
            col: group[0].col,
            match_positions: Vec::new(),
            content: None,
        });
        for item in group {
            rows.push(PickerResult {
                display: format!(
                    "    {}  {}:{}  {}{}",
                    severity_tag(item.severity),
                    item.line + 1,
                    item.col + 1,
                    item.message,
                    item.source
                        .as_deref()
                        .map(|source| format!("  [{source}]"))
                        .unwrap_or_default(),
                ),
                location: item.path.to_string_lossy().to_string(),
                line: item.line,
                col: item.col,
                match_positions: Vec::new(),
                content: None,
            });
        }
        index = end;
    }
    let errors = shown.iter().filter(|item| item.severity == 1).count();
    let warnings = shown.iter().filter(|item| item.severity == 2).count();
    let summary = format!(
        "{errors} error{}, {warnings} warning{} in {files} file{}",
        if errors == 1 { "" } else { "s" },
        if warnings == 1 { "" } else { "s" },
        if files == 1 { "" } else { "s" },
    );
    (rows, summary)
}

impl Editor {
    /// Reads every published diagnostic from the language servers.
    fn collect_problems(&self) -> Vec<ProblemItem> {
        let Some(manager) = self.lsp_manager() else {
            return Vec::new();
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return Vec::new();
        };
        let all = tokio::task::block_in_place(|| {
            handle.block_on(async { manager.list_all_diagnostics().await })
        });
        let mut items = problems_from(all);
        let mut columns = super::lsp_columns::ColumnResolver::new(self);
        for item in &mut items {
            item.col = columns.grapheme_col(&item.path, item.line, item.col as u32);
        }
        items
    }

    /// `<Space>sd` / `:Problems [all|warnings|errors]`.
    pub fn open_problems_picker(&mut self, filter: ProblemFilter) {
        let root = self.picker_dirs().0;
        let items = self.collect_problems();
        if items.is_empty() {
            self.set_status_message(if self.lsp_manager().is_none() {
                "Problems: no language server is running"
            } else {
                "Problems: the language servers have not published any diagnostics"
            });
            return;
        }
        let (rows, summary) = problem_rows(&items, filter, &root);
        if rows.is_empty() {
            self.set_status_message(format!("No {} problems", filter.label()));
            return;
        }
        let picker = Picker::new_with_results(root.clone(), rows)
            .with_title(format!("Problems ({})", filter.label()))
            .with_role(PickerRole::Problems);
        self.ui_panels.problems = Some(Box::new(ProblemsState {
            items,
            filter,
            root,
        }));
        self.set_picker(picker);
        self.set_mode(Mode::Picker);
        self.mark_picker_selection_changed();
        self.set_status_message(summary);
    }

    /// `Ctrl-T` in the problems list: all -> errors and warnings -> errors.
    pub fn cycle_problems_filter(&mut self) {
        let Some(state) = self.ui_panels.problems.as_mut() else {
            return;
        };
        state.filter = state.filter.next();
        let (rows, summary) = problem_rows(&state.items, state.filter, &state.root);
        let title = format!("Problems ({})", state.filter.label());
        if let Some(picker) = self.picker_mut() {
            picker.set_title(title);
            picker.replace_results_keeping_selection(rows);
        }
        self.set_status_message(summary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Diagnostic, Position, Range};

    fn diagnostic(line: u32, severity: DiagnosticSeverity, message: &str) -> Diagnostic {
        Diagnostic {
            range: Range {
                start: Position { line, character: 4 },
                end: Position { line, character: 8 },
            },
            severity: Some(severity),
            message: message.to_string(),
            source: Some("javac".to_string()),
            ..Diagnostic::default()
        }
    }

    fn uri(path: &str) -> lsp_types::Uri {
        crate::lsp::uri_from_file_path(Path::new(path)).unwrap()
    }

    fn items() -> Vec<ProblemItem> {
        problems_from(vec![
            (
                uri("/p/src/B.java"),
                vec![
                    diagnostic(9, DiagnosticSeverity::WARNING, "unused import"),
                    diagnostic(
                        2,
                        DiagnosticSeverity::ERROR,
                        "cannot find symbol: Foo\nsecond line",
                    ),
                ],
                vec![],
            ),
            (
                uri("/p/src/A.java"),
                vec![diagnostic(0, DiagnosticSeverity::HINT, "consider final")],
                vec![],
            ),
        ])
    }

    #[test]
    fn items_are_sorted_by_file_then_position_and_keep_the_first_message_line() {
        let items = items();
        let order: Vec<(String, usize)> = items
            .iter()
            .map(|item| (item.path.to_string_lossy().to_string(), item.line))
            .collect();
        assert_eq!(
            order,
            vec![
                ("/p/src/A.java".to_string(), 0),
                ("/p/src/B.java".to_string(), 2),
                ("/p/src/B.java".to_string(), 9)
            ]
        );
        assert_eq!(items[1].message, "cannot find symbol: Foo");
    }

    #[test]
    fn rows_group_by_file_with_counts_and_indented_problems() {
        let (rows, summary) = problem_rows(&items(), ProblemFilter::All, Path::new("/p"));
        let text: Vec<&str> = rows.iter().map(|row| row.display.as_str()).collect();
        assert_eq!(text[0], "src/A.java  1 (0 errors, 0 warnings)");
        assert_eq!(text[1], "    hint   1:5  consider final  [javac]");
        assert_eq!(text[2], "src/B.java  2 (1 errors, 1 warnings)");
        assert_eq!(text[3], "    error  3:5  cannot find symbol: Foo  [javac]");
        assert_eq!(text[4], "    warn   10:5  unused import  [javac]");
        assert_eq!(summary, "1 error, 1 warning in 2 files");
        // Enter on a problem jumps to its position; on a header, to the first one.
        assert_eq!((rows[3].line, rows[3].col), (2, 4));
        assert_eq!(rows[2].location, "/p/src/B.java");
        assert_eq!(rows[2].line, 2);
    }

    #[test]
    fn severity_filter_hides_lesser_problems_and_empty_files() {
        let (rows, summary) = problem_rows(&items(), ProblemFilter::Errors, Path::new("/p"));
        assert_eq!(rows.len(), 2, "one file header and its one error: {rows:?}");
        assert_eq!(summary, "1 error, 0 warnings in 1 file");
        let (rows, _) = problem_rows(&items(), ProblemFilter::WarningsUp, Path::new("/p"));
        assert_eq!(rows.len(), 3, "B.java header, its error and its warning");
    }

    #[test]
    fn filter_parses_words_and_cycles() {
        assert_eq!(ProblemFilter::parse("errors"), Some(ProblemFilter::Errors));
        assert_eq!(ProblemFilter::parse(""), Some(ProblemFilter::All));
        assert_eq!(ProblemFilter::parse("nope"), None);
        assert_eq!(ProblemFilter::All.next().next().next(), ProblemFilter::All);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn picker_lists_the_diagnostics_of_every_file_the_server_published_for() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        for name in ["A.java", "B.java", "C.java"] {
            std::fs::write(root.join(name), "class X {}\n").unwrap();
        }
        let mut editor = Editor::default();
        editor.enable_lsp();
        editor.load_file(root.join("A.java")).unwrap();
        let manager = editor.lsp_manager().unwrap();
        manager
            .set_diagnostics(
                crate::lsp::uri_from_file_path(root.join("A.java")).unwrap(),
                "java",
                vec![diagnostic(0, DiagnosticSeverity::ERROR, "boom in A")],
                None,
            )
            .await;
        // B is not the current buffer, C has no diagnostics.
        manager
            .set_diagnostics(
                crate::lsp::uri_from_file_path(root.join("B.java")).unwrap(),
                "java",
                vec![diagnostic(0, DiagnosticSeverity::WARNING, "careful in B")],
                None,
            )
            .await;

        editor.open_problems_picker(ProblemFilter::All);
        assert_eq!(editor.mode(), Mode::Picker);
        let picker = editor.picker().unwrap();
        assert_eq!(picker.title(), Some("Problems (all)"));
        let rows: Vec<String> = picker
            .collect_filtered_results(20)
            .into_iter()
            .map(|row| row.display.clone())
            .collect();
        assert_eq!(rows.len(), 4, "{rows:?}");
        assert!(rows[0].starts_with("A.java"));
        assert!(rows[1].contains("boom in A"));
        assert!(rows[2].starts_with("B.java"));
        assert!(rows[3].contains("careful in B"));

        editor.cycle_problems_filter();
        assert_eq!(
            editor.picker().unwrap().title(),
            Some("Problems (errors and warnings)")
        );
        editor.cycle_problems_filter();
        let picker = editor.picker().unwrap();
        assert_eq!(picker.title(), Some("Problems (errors)"));
        assert_eq!(picker.filtered_result_count(), 2, "only A.java's error");
    }
}
