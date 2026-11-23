//! Document outline: the cached symbol tree behind breadcrumbs and the
//! outline picker.
//!
//! The tree comes from the language server (`textDocument/documentSymbol`)
//! and falls back to tree-sitter for the languages that have a node table
//! (Rust, TypeScript/JavaScript, Python, Java, Kotlin), so breadcrumbs work
//! before the server is ready and without one. Like code lenses the outline is
//! refreshed once edits settle, from the shared tick, and never waits on the
//! server.

use std::time::{Duration, Instant};

use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::picker::{Picker, PickerResult};
use super::Editor;
use crate::mode::Mode;
use crate::navigation_types::OutlineSymbol;

/// Edits must be quiet for this long before the outline is recomputed.
const DEBOUNCE: Duration = Duration::from_millis(250);
/// After a failed or empty server answer, wait this long before asking again.
const RETRY: Duration = Duration::from_secs(2);

/// One crumb of the enclosing-symbol path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreadcrumbItem {
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutlineSource {
    #[default]
    None,
    TreeSitter,
    Lsp,
}

type SymbolResponse = anyhow::Result<Vec<lsp_types::DocumentSymbol>>;

struct Inflight {
    _task: JoinHandle<()>,
    rx: oneshot::Receiver<SymbolResponse>,
    file_path: String,
    version: usize,
}

#[derive(Default)]
pub struct OutlineState {
    file_path: Option<String>,
    /// Buffer version the symbols describe.
    version: usize,
    symbols: Vec<OutlineSymbol>,
    source: OutlineSource,
    /// Buffer version the tree-sitter pass last ran for.
    checked_version: Option<usize>,
    settling: Option<(usize, Instant)>,
    /// Version of the last server request, and when it was sent.
    requested: Option<(usize, Instant)>,
    /// Version the server answered for (an empty answer counts).
    answered: Option<usize>,
    inflight: Option<Inflight>,
}

/// Kinds that make sense as a "where am I" crumb.
fn is_crumb_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class"
            | "interface"
            | "enum"
            | "struct"
            | "module"
            | "namespace"
            | "package"
            | "impl"
            | "object"
            | "method"
            | "function"
            | "constructor"
            | "property"
    )
}

/// The symbols containing 0-based `line`, outermost first (of every kind).
pub(crate) fn enclosing_path(symbols: &[OutlineSymbol], line: usize) -> Vec<&OutlineSymbol> {
    let mut path = Vec::new();
    let mut level = symbols;
    while let Some(symbol) = level
        .iter()
        .filter(|symbol| {
            symbol.start_line >= 1 && symbol.start_line - 1 <= line && line < symbol.end_line
        })
        // Innermost wins when siblings overlap (e.g. one-line members).
        .min_by_key(|symbol| symbol.end_line - symbol.start_line)
    {
        path.push(symbol);
        level = &symbol.children;
    }
    path
}

/// Chain of "where am I" symbols containing 0-based `line`, outermost first.
fn enclosing_chain(symbols: &[OutlineSymbol], line: usize) -> Vec<BreadcrumbItem> {
    enclosing_path(symbols, line)
        .into_iter()
        .filter(|symbol| is_crumb_kind(&symbol.kind))
        .map(|symbol| BreadcrumbItem {
            name: symbol.name.clone(),
            kind: symbol.kind.clone(),
        })
        .collect()
}

/// Rows of the outline picker: depth-indented, with kind and line.
fn outline_rows(symbols: &[OutlineSymbol], file: &str) -> Vec<PickerResult> {
    fn walk(symbols: &[OutlineSymbol], depth: usize, file: &str, out: &mut Vec<PickerResult>) {
        for symbol in symbols {
            let (line, col) = symbol
                .selection
                .unwrap_or((symbol.start_line.saturating_sub(1), 0));
            out.push(PickerResult {
                display: format!(
                    "{}{}  {} :{}",
                    "  ".repeat(depth),
                    symbol.name,
                    symbol.kind,
                    line + 1
                ),
                location: file.to_string(),
                line,
                col,
                match_positions: Vec::new(),
                content: None,
            });
            walk(&symbol.children, depth + 1, file, out);
        }
    }
    let mut rows = Vec::new();
    walk(symbols, 0, file, &mut rows);
    rows
}

impl Editor {
    /// The enclosing symbols of the cursor, outermost first ("Circle", "area()").
    pub fn breadcrumbs(&self) -> Vec<BreadcrumbItem> {
        enclosing_chain(self.outline_symbols(), self.buffer().cursor().line())
    }

    /// The symbol tree of the current buffer: from the language server, or
    /// tree-sitter before it answers (empty until either has run).
    pub(crate) fn outline_symbols(&self) -> &[OutlineSymbol] {
        let state = &self.ui_panels.outline;
        if state.source == OutlineSource::None
            || self.buffer().file_path() != state.file_path.as_deref()
        {
            return &[];
        }
        &state.symbols
    }

    /// Breadcrumbs as one line: `Circle › area()`.
    pub fn breadcrumb_text(&self) -> String {
        self.breadcrumbs()
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>()
            .join(" › ")
    }

    /// Where the current outline came from (for the status/diagnostics).
    pub fn outline_source(&self) -> OutlineSource {
        self.ui_panels.outline.source
    }

    fn current_outline_file(&self) -> Option<String> {
        let path = self.buffer().file_path()?;
        if super::buffer_manager::is_scratch_path(path) {
            return None;
        }
        Some(path.to_string())
    }

    /// Recomputes the outline when the buffer changed and settled. Called from
    /// the tick; the tree-sitter pass is immediate, the server pass is polled.
    pub async fn request_outline_if_needed(&mut self) {
        let Some(path) = self.current_outline_file() else {
            *self.ui_panels.outline = OutlineState::default();
            return;
        };
        let version = self.buffer().version();
        if self.ui_panels.outline.file_path.as_deref() != Some(path.as_str()) {
            *self.ui_panels.outline = OutlineState {
                file_path: Some(path.clone()),
                ..OutlineState::default()
            };
        }

        // Tree-sitter pass: once per buffer version, after edits go quiet.
        if self.ui_panels.outline.checked_version != Some(version) {
            let state = &mut self.ui_panels.outline;
            if state.source != OutlineSource::None {
                match &state.settling {
                    Some((seen, since)) if *seen == version => {
                        if since.elapsed() < DEBOUNCE {
                            return;
                        }
                    }
                    _ => {
                        state.settling = Some((version, Instant::now()));
                        return;
                    }
                }
            }
            let symbols = self.treesitter_outline();
            let state = &mut self.ui_panels.outline;
            state.settling = None;
            state.checked_version = Some(version);
            if !symbols.is_empty() {
                state.symbols = symbols;
                state.source = OutlineSource::TreeSitter;
                state.version = version;
            }
            self.mark_dirty();
        }

        // Server pass: at most one request per version, retried after errors.
        let state = &self.ui_panels.outline;
        if state.inflight.is_some() || state.answered == Some(version) {
            return;
        }
        if let Some((requested_version, at)) = &state.requested {
            if *requested_version == version && at.elapsed() < RETRY {
                return;
            }
        }
        let Some(manager) = self.lsp_manager() else {
            return;
        };
        let Some(language_id) = self.language_id_for_path(&path) else {
            return;
        };
        let Some(uri) = crate::lsp::uri_from_file_path(&path) else {
            return;
        };
        self.ensure_lsp_document_synced().await;
        let state = &mut self.ui_panels.outline;
        state.requested = Some((version, Instant::now()));
        let (tx, rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = tx.send(manager.document_symbols(&uri, &language_id).await);
        });
        state.inflight = Some(Inflight {
            _task: task,
            rx,
            file_path: path,
            version,
        });
    }

    /// Applies a finished server answer. Returns true when the outline changed.
    pub fn poll_outline(&mut self) -> bool {
        let Some(mut inflight) = self.ui_panels.outline.inflight.take() else {
            return false;
        };
        let response = match inflight.rx.try_recv() {
            Ok(response) => response,
            Err(oneshot::error::TryRecvError::Empty) => {
                self.ui_panels.outline.inflight = Some(inflight);
                return false;
            }
            Err(oneshot::error::TryRecvError::Closed) => return false,
        };
        if self.buffer().file_path() != Some(inflight.file_path.as_str())
            || self.buffer().version() != inflight.version
        {
            // Computed for text that is gone; the next tick asks again.
            self.ui_panels.outline.requested = None;
            return false;
        }
        if response.is_ok() {
            self.ui_panels.outline.answered = Some(inflight.version);
        }
        match response {
            Ok(symbols) if !symbols.is_empty() => {
                let converted = symbols
                    .iter()
                    .map(super::lsp_integration::lsp_modules::navigation::convert_document_symbol)
                    .collect();
                let state = &mut self.ui_panels.outline;
                state.symbols = converted;
                state.source = OutlineSource::Lsp;
                state.version = inflight.version;
                self.mark_dirty();
                true
            }
            _ => false,
        }
    }

    /// `<Space>o` / `:Outline` — the file's symbol tree, indented, in a picker.
    ///
    /// Uses the freshest tree available: the cached outline when it describes
    /// the current text, otherwise a tree-sitter pass; the server tree replaces
    /// it as soon as `poll_outline` receives it.
    pub fn open_outline_picker(&mut self) {
        let Some(path) = self.current_outline_file() else {
            self.set_status_message("Save the file first to see its outline");
            return;
        };
        let version = self.buffer().version();
        let fresh = {
            let state = &self.ui_panels.outline;
            state.file_path.as_deref() == Some(path.as_str())
                && state.version == version
                && state.source != OutlineSource::None
        };
        let symbols = if fresh {
            self.ui_panels.outline.symbols.clone()
        } else {
            self.treesitter_outline()
        };
        let rows = outline_rows(&symbols, &path);
        if rows.is_empty() {
            self.set_status_message("No symbols found");
            return;
        }
        let base_dir = self.picker_dirs().0;
        self.lsp.state.hierarchy = None;
        let picker = Picker::new_with_results(base_dir, rows).with_title("Outline");
        self.set_picker(picker);
        self.set_mode(Mode::Picker);
        self.mark_picker_selection_changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(
        name: &str,
        kind: &str,
        start: usize,
        end: usize,
        children: Vec<OutlineSymbol>,
    ) -> OutlineSymbol {
        OutlineSymbol {
            name: name.to_string(),
            kind: kind.to_string(),
            detail: None,
            start_line: start,
            end_line: end,
            children,
            selection: Some((start - 1, 4)),
        }
    }

    fn tree() -> Vec<OutlineSymbol> {
        vec![symbol(
            "Circle",
            "class",
            1,
            20,
            vec![
                symbol("radius", "field", 2, 2, vec![]),
                symbol("area()", "method", 4, 8, vec![]),
                symbol(
                    "Builder",
                    "class",
                    10,
                    18,
                    vec![symbol("build()", "method", 12, 16, vec![])],
                ),
            ],
        )]
    }

    fn names(chain: Vec<BreadcrumbItem>) -> Vec<String> {
        chain.into_iter().map(|item| item.name).collect()
    }

    #[test]
    fn chain_lists_the_enclosing_classes_and_method_outermost_first() {
        // Lines are 0-based here; symbol lines are 1-based.
        assert_eq!(names(enclosing_chain(&tree(), 5)), vec!["Circle", "area()"]);
        assert_eq!(
            names(enclosing_chain(&tree(), 13)),
            vec!["Circle", "Builder", "build()"]
        );
        assert_eq!(
            names(enclosing_chain(&tree(), 9)),
            vec!["Circle", "Builder"]
        );
    }

    #[test]
    fn chain_skips_fields_and_stops_outside_any_symbol() {
        assert_eq!(names(enclosing_chain(&tree(), 1)), vec!["Circle"]);
        assert!(enclosing_chain(&tree(), 30).is_empty());
    }

    #[test]
    fn outline_rows_indent_by_depth_and_point_at_the_name() {
        let rows = outline_rows(&tree(), "/p/Circle.java");
        assert_eq!(rows[0].display, "Circle  class :1");
        assert_eq!(rows[2].display, "  area()  method :4");
        assert_eq!(rows[4].display, "    build()  method :12");
        assert_eq!((rows[2].line, rows[2].col), (3, 4));
        assert_eq!(rows[2].location, "/p/Circle.java");
    }
}
