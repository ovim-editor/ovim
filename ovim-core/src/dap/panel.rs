//! Rows of the debug panel: call stack, variables, watches, breakpoints and
//! exception filters as one flat, scrollable, keyboard-navigable list.
//!
//! The list is derived from [`DebugState`] on demand, so the TUI, the GUI
//! and the key handler always agree on what row `n` is.

use std::path::PathBuf;

use super::state::DebugState;
use super::types::{DapThread, DapVariable};

/// What a row is, and therefore what Enter / `d` / `e` do to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    /// Section title; not actionable.
    Header,
    /// Dim explanatory text (`Running...`, `No watches`); not actionable.
    Note,
    Frame {
        index: usize,
        selected: bool,
    },
    /// A variable. `var_ref` is 0 for values without children.
    Variable {
        var_ref: u64,
        expanded: bool,
    },
    Watch {
        index: usize,
        var_ref: u64,
        expanded: bool,
    },
    Breakpoint {
        path: PathBuf,
        line: u64,
        enabled: bool,
        verified: bool,
        conditional: bool,
    },
    Exception {
        index: usize,
        enabled: bool,
    },
    /// A thread of the debuggee; Enter shows its stack.
    Thread {
        id: u64,
        selected: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelRow {
    pub kind: RowKind,
    /// Tree depth (indent) of variables below their parent.
    pub depth: usize,
    pub label: String,
    pub value: Option<String>,
    pub type_: Option<String>,
    /// Row index of the parent variable/watch, for collapsing with `h`.
    pub parent: Option<usize>,
}

impl PanelRow {
    fn new(kind: RowKind, label: impl Into<String>) -> Self {
        Self {
            kind,
            depth: 0,
            label: label.into(),
            value: None,
            type_: None,
            parent: None,
        }
    }

    /// Rows the cursor may rest on.
    pub fn is_actionable(&self) -> bool {
        !matches!(self.kind, RowKind::Header | RowKind::Note)
    }
}

/// Nesting deeper than this is not shown (guards against cyclic structures
/// the adapter hands out as ever-new references).
const MAX_DEPTH: usize = 12;

fn push_variables(
    rows: &mut Vec<PanelRow>,
    state: &DebugState,
    vars: &[DapVariable],
    depth: usize,
    parent: Option<usize>,
) {
    for var in vars {
        let expanded =
            var.variables_reference > 0 && state.expanded_refs.contains(&var.variables_reference);
        let index = rows.len();
        rows.push(PanelRow {
            kind: RowKind::Variable {
                var_ref: var.variables_reference,
                expanded,
            },
            depth,
            label: var.name.clone(),
            value: Some(var.value.clone()),
            type_: var.type_.clone(),
            parent,
        });
        if expanded && depth < MAX_DEPTH {
            match state.variables.get(&var.variables_reference) {
                Some(children) => push_variables(rows, state, children, depth + 1, Some(index)),
                None => {
                    let mut loading = PanelRow::new(RowKind::Note, "loading...");
                    loading.depth = depth + 1;
                    rows.push(loading);
                }
            }
        }
    }
}

/// Threads worth listing: the JVM's own housekeeping threads are hidden
/// (unless one is the thread being inspected).
fn visible_threads(state: &DebugState) -> Vec<&DapThread> {
    const INTERNAL: &[&str] = &[
        "Reference Handler",
        "Finalizer",
        "Signal Dispatcher",
        "Common-Cleaner",
        "Notification Thread",
        "Attach Listener",
        "Service Thread",
        "Monitor Deflation Thread",
        "Monitor Ctrl-Break",
        "Sweeper thread",
        "Process reaper",
        "JDWP",
        "Cleaner-",
        "C1 CompilerThread",
        "C2 CompilerThread",
        "Compiler",
        "Notification",
    ];
    state
        .threads
        .iter()
        .filter(|t| {
            state.stopped_thread == Some(t.id)
                || !INTERNAL.iter().any(|name| t.name.starts_with(name))
        })
        .collect()
}

/// The whole panel, top to bottom.
pub fn rows(state: &DebugState) -> Vec<PanelRow> {
    let mut rows = Vec::new();

    // ---- Call stack ----
    let status = if !state.session_active {
        "no session"
    } else if state.is_running {
        "running"
    } else {
        state.stop_reason.as_deref().unwrap_or("stopped")
    };
    rows.push(PanelRow::new(
        RowKind::Header,
        format!("Call Stack ({status})"),
    ));
    if let Some(exception) = state
        .exception
        .as_deref()
        .filter(|_| !state.is_running && state.stopped_thread == state.event_thread)
    {
        rows.push(PanelRow::new(RowKind::Note, format!("! {exception}")));
    }
    if state.stack_frames.is_empty() {
        rows.push(PanelRow::new(
            RowKind::Note,
            if state.is_running {
                "Running..."
            } else if state.session_active {
                "No stack trace"
            } else {
                "No debug session"
            },
        ));
    }
    for (index, frame) in state.stack_frames.iter().enumerate() {
        let source = frame
            .source
            .as_ref()
            .and_then(|s| s.name.as_deref())
            .unwrap_or("?");
        rows.push(PanelRow::new(
            RowKind::Frame {
                index,
                selected: index == state.selected_frame,
            },
            format!("{} {}:{}", frame.name, source, frame.line),
        ));
    }

    // ---- Threads ----
    let threads = visible_threads(state);
    if threads.len() > 1 {
        rows.push(PanelRow::new(RowKind::Header, "Threads"));
        for thread in threads {
            let selected = state.stopped_thread == Some(thread.id);
            let mut row = PanelRow::new(
                RowKind::Thread {
                    id: thread.id,
                    selected,
                },
                format!("{} ({})", thread.name, thread.id),
            );
            row.depth = 1;
            if state.event_thread == Some(thread.id) {
                row.value = Some("stopped here".to_string());
            }
            rows.push(row);
        }
    }

    // ---- Variables ----
    rows.push(PanelRow::new(RowKind::Header, "Variables"));
    if state.scopes.is_empty() {
        rows.push(PanelRow::new(RowKind::Note, "No variables"));
    }
    for scope in &state.scopes {
        rows.push(PanelRow::new(RowKind::Note, format!("{}:", scope.name)));
        if let Some(vars) = state.variables.get(&scope.variables_reference) {
            push_variables(&mut rows, state, vars, 1, None);
        }
    }

    // ---- Watches ----
    rows.push(PanelRow::new(RowKind::Header, "Watch"));
    if state.watches.is_empty() {
        rows.push(PanelRow::new(RowKind::Note, "none (:DebugWatch <expr>)"));
    }
    for (index, watch) in state.watches.iter().enumerate() {
        let expanded = watch.variables_reference > 0
            && state.expanded_refs.contains(&watch.variables_reference);
        let at = rows.len();
        let mut row = PanelRow::new(
            RowKind::Watch {
                index,
                var_ref: watch.variables_reference,
                expanded,
            },
            watch.expression.clone(),
        );
        row.depth = 1;
        row.type_ = watch.type_.clone();
        row.value = Some(match &watch.result {
            Some(Ok(value)) => value.clone(),
            Some(Err(message)) => format!("<{message}>"),
            None => "<not evaluated>".to_string(),
        });
        rows.push(row);
        if expanded {
            if let Some(children) = state.variables.get(&watch.variables_reference) {
                push_variables(&mut rows, state, children, 2, Some(at));
            }
        }
    }

    // ---- Breakpoints ----
    rows.push(PanelRow::new(RowKind::Header, "Breakpoints"));
    let breakpoints = state.all_breakpoints();
    if breakpoints.is_empty() {
        rows.push(PanelRow::new(RowKind::Note, "none (F9 toggles one)"));
    }
    for (path, bp) in breakpoints {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let mut row = PanelRow::new(
            RowKind::Breakpoint {
                path: path.to_path_buf(),
                line: bp.shown_line(),
                enabled: bp.enabled,
                verified: bp.verified,
                conditional: bp.condition.is_some()
                    || bp.log_message.is_some()
                    || bp.hit_condition.is_some(),
            },
            format!("{name}:{}", bp.shown_line()),
        );
        row.depth = 1;
        let mut notes = Vec::new();
        if let Some(condition) = &bp.condition {
            notes.push(format!("if {condition}"));
        }
        if let Some(count) = &bp.hit_condition {
            notes.push(format!("hits {count}"));
        }
        if let Some(message) = &bp.log_message {
            notes.push(format!("log \"{message}\""));
        }
        row.value = (!notes.is_empty()).then(|| notes.join("  "));
        rows.push(row);
    }

    // ---- Exception filters ----
    if !state.exception_filters.is_empty() {
        rows.push(PanelRow::new(RowKind::Header, "Break on exceptions"));
        for (index, filter) in state.exception_filters.iter().enumerate() {
            let mut row = PanelRow::new(
                RowKind::Exception {
                    index,
                    enabled: filter.enabled,
                },
                filter.label.clone(),
            );
            row.depth = 1;
            rows.push(row);
        }
    }
    rows
}

/// Keeps `cursor` on an actionable row and `scroll` such that it is visible.
pub fn clamp_view(rows: &[PanelRow], cursor: &mut usize, scroll: &mut usize, height: usize) {
    if rows.is_empty() {
        *cursor = 0;
        *scroll = 0;
        return;
    }
    *cursor = (*cursor).min(rows.len() - 1);
    let height = height.max(1);
    if *cursor < *scroll {
        *scroll = *cursor;
    } else if *cursor >= *scroll + height {
        *scroll = *cursor + 1 - height;
    }
    *scroll = (*scroll).min(rows.len().saturating_sub(height));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dap::types::{DapScope, DapStackFrame};

    fn var(name: &str, value: &str, var_ref: u64) -> DapVariable {
        DapVariable {
            name: name.into(),
            value: value.into(),
            type_: None,
            variables_reference: var_ref,
        }
    }

    fn stopped() -> DebugState {
        let mut s = DebugState::new();
        s.session_active = true;
        s.stop_reason = Some("breakpoint".into());
        s.stack_frames = vec![DapStackFrame {
            id: 1,
            name: "main".into(),
            source: None,
            line: 7,
            column: 0,
        }];
        s.scopes = vec![DapScope {
            name: "Locals".into(),
            variables_reference: 10,
            expensive: false,
        }];
        s.variables
            .insert(10, vec![var("user", "User@1", 11), var("n", "3", 0)]);
        s
    }

    fn labels(rows: &[PanelRow]) -> Vec<String> {
        rows.iter().map(|r| r.label.clone()).collect()
    }

    #[test]
    fn variables_expand_and_collapse_in_place() {
        let mut s = stopped();
        s.variables.insert(11, vec![var("name", "\"Ann\"", 0)]);
        let collapsed = rows(&s);
        assert!(!labels(&collapsed).contains(&"name".to_string()));
        s.expanded_refs.insert(11);
        let expanded = rows(&s);
        let at = expanded.iter().position(|r| r.label == "name").unwrap();
        assert_eq!(expanded[at].depth, 2);
        let parent = expanded[at].parent.unwrap();
        assert_eq!(expanded[parent].label, "user");
    }

    #[test]
    fn an_expanded_variable_whose_children_are_not_loaded_shows_loading() {
        let mut s = stopped();
        s.expanded_refs.insert(11);
        assert!(labels(&rows(&s)).contains(&"loading...".to_string()));
    }

    #[test]
    fn watches_breakpoints_and_exception_filters_have_their_own_sections() {
        let mut s = stopped();
        s.watches.push(super::super::state::Watch {
            expression: "n * 2".into(),
            result: Some(Ok("6".into())),
            type_: Some("int".into()),
            variables_reference: 0,
        });
        s.toggle_breakpoint(std::path::Path::new("/p/A.java"), 12);
        s.set_breakpoint_condition(std::path::Path::new("/p/A.java"), 12, Some("n > 1".into()));
        s.exception_filters
            .push(super::super::state::ExceptionFilter {
                id: "all".into(),
                label: "All exceptions".into(),
                enabled: true,
            });
        let r = rows(&s);
        let watch = r.iter().find(|r| r.label == "n * 2").unwrap();
        assert_eq!(watch.value.as_deref(), Some("6"));
        let bp = r.iter().find(|r| r.label == "A.java:12").unwrap();
        assert_eq!(bp.value.as_deref(), Some("if n > 1"));
        assert!(matches!(
            r.last().unwrap().kind,
            RowKind::Exception { enabled: true, .. }
        ));
    }

    #[test]
    fn clamp_keeps_the_cursor_visible() {
        let s = stopped();
        let r = rows(&s);
        let (mut cursor, mut scroll) = (r.len() + 5, 0);
        clamp_view(&r, &mut cursor, &mut scroll, 3);
        assert_eq!(cursor, r.len() - 1);
        assert_eq!(scroll, r.len() - 3);
        cursor = 0;
        clamp_view(&r, &mut cursor, &mut scroll, 3);
        assert_eq!(scroll, 0);
    }

    #[test]
    fn disabled_breakpoints_are_kept_but_not_sent() {
        let mut s = DebugState::new();
        let f = std::path::Path::new("/p/A.java");
        s.toggle_breakpoint(f, 3);
        s.toggle_breakpoint(f, 9);
        assert_eq!(s.toggle_breakpoint_enabled(f, 3), Some(false));
        assert_eq!(
            s.enabled_breakpoints(f)
                .iter()
                .map(|bp| bp.line)
                .collect::<Vec<_>>(),
            vec![9]
        );
        // The adapter's answer only mentions the enabled one.
        s.update_breakpoints(
            f,
            &[9],
            &[crate::dap::types::DapBreakpoint {
                id: Some(1),
                verified: true,
                message: None,
                line: Some(9),
            }],
        );
        assert_eq!(s.breakpoint_lines(f), vec![3, 9]);
        assert!(!s.is_breakpoint_enabled(f, 3));
        assert!(s.remove_breakpoint(f, 3));
        assert_eq!(s.breakpoint_lines(f), vec![9]);
    }
}
