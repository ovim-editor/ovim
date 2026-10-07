//! Debug state tracking.
//!
//! Holds all debug-related state: breakpoints, stack frames, variables, output.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::types::{DapBreakpoint, DapScope, DapStackFrame, DapThread, DapVariable};

/// Per-line breakpoint state.
#[derive(Debug, Clone)]
pub struct BreakpointState {
    /// 1-based line the user asked for. This is the breakpoint's identity:
    /// it is what every sync sends, so the adapter resolves it afresh.
    pub line: u64,
    /// 1-based line the adapter put the breakpoint on when it moved it (to
    /// the next executable line, say). `None` until the adapter has answered
    /// and when it gave no line.
    pub actual_line: Option<u64>,
    /// Whether the debug adapter confirmed this breakpoint.
    pub verified: bool,
    /// DAP-assigned breakpoint ID.
    pub id: Option<u64>,
    /// Condition expression for conditional breakpoints (None = unconditional).
    pub condition: Option<String>,
    /// Logpoint: print this message (with `{expression}` interpolation)
    /// instead of stopping.
    pub log_message: Option<String>,
    /// Hit-count condition (`5`, `>3`, `%2`): stop only when it holds.
    pub hit_condition: Option<String>,
    /// Disabled breakpoints stay in the list (and the gutter, hollow) but are
    /// not sent to the adapter.
    pub enabled: bool,
}

impl BreakpointState {
    fn new(line: u64) -> Self {
        Self {
            line,
            actual_line: None,
            verified: false,
            id: None,
            condition: None,
            log_message: None,
            hit_condition: None,
            enabled: true,
        }
    }

    /// The line the breakpoint is drawn on and found by: where the adapter
    /// put it, else where the user did.
    pub fn shown_line(&self) -> u64 {
        self.actual_line.unwrap_or(self.line)
    }
}

/// A watch expression, re-evaluated at every stop.
#[derive(Debug, Clone)]
pub struct Watch {
    pub expression: String,
    /// `Ok(value)` or `Err(message)` from the last evaluation while stopped.
    pub result: Option<Result<String, String>>,
    pub type_: Option<String>,
    /// Non-zero when the value has children that can be expanded.
    pub variables_reference: u64,
}

/// An exception category the adapter can break on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptionFilter {
    pub id: String,
    pub label: String,
    pub enabled: bool,
}

/// Cursor and scroll state of the focusable debug panel.
#[derive(Debug, Clone, Default)]
pub struct PanelUi {
    /// Highlighted row (index into [`panel::rows`](super::panel::rows)).
    pub cursor: usize,
    /// First visible row (written by the renderer, which knows the height).
    pub scroll: std::cell::Cell<usize>,
    /// Rows that fit on screen; set by the renderer for paging.
    pub view_height: std::cell::Cell<usize>,
    /// Columns added to (or taken from) the default panel width.
    pub width_delta: i16,
}

/// All debug state for the editor.
pub struct DebugState {
    /// Whether a debug session is active.
    pub session_active: bool,
    /// Whether the debuggee is currently running (not stopped).
    pub is_running: bool,

    // ---- Stop state ----
    /// Thread that is currently stopped (if any).
    pub stopped_thread: Option<u64>,
    /// The thread the adapter reported the stop on (`stopped_thread` is the
    /// one being inspected, which the user can change).
    pub event_thread: Option<u64>,
    /// Threads of the debuggee as of the last stop.
    pub threads: Vec<DapThread>,
    /// Reason for the stop (e.g., "breakpoint", "step", "exception").
    pub stop_reason: Option<String>,
    /// What was thrown (`Type: message`) when the stop reason is "exception".
    pub exception: Option<String>,

    // ---- Breakpoints ----
    /// Breakpoints per file path.
    pub breakpoints: HashMap<PathBuf, Vec<BreakpointState>>,

    // ---- Stack trace ----
    /// Stack frames from the last stop.
    pub stack_frames: Vec<DapStackFrame>,
    /// Currently selected frame index.
    pub selected_frame: usize,

    // ---- Variables ----
    /// Scopes for the selected frame.
    pub scopes: Vec<DapScope>,
    /// Variables by reference ID.
    pub variables: HashMap<u64, Vec<DapVariable>>,
    /// Expanded variable references (for tree view).
    pub expanded_refs: HashSet<u64>,

    // ---- Output ----
    /// Debuggee output lines.
    pub output_lines: Vec<String>,

    // ---- Watches and exceptions ----
    pub watches: Vec<Watch>,
    /// Exception filters the adapter offered (kept across sessions so the
    /// user's choice survives a restart).
    pub exception_filters: Vec<ExceptionFilter>,

    // ---- UI ----
    /// Whether debug panels are visible.
    pub panels_visible: bool,
    /// The user opened the panel themselves (`<Space>dv` / `<Space>df`): it
    /// stays when a session ends instead of disappearing with it.
    pub panel_pinned: bool,
    pub panel: PanelUi,

    // ---- Execution line tracking ----
    /// Current execution file path (for gutter indicator).
    pub execution_file: Option<PathBuf>,
    /// Current execution line (1-based, for gutter indicator).
    pub execution_line: Option<u64>,
}

impl Default for DebugState {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugState {
    pub fn new() -> Self {
        Self {
            session_active: false,
            is_running: false,
            stopped_thread: None,
            event_thread: None,
            threads: Vec::new(),
            stop_reason: None,
            exception: None,
            breakpoints: HashMap::new(),
            stack_frames: Vec::new(),
            selected_frame: 0,
            scopes: Vec::new(),
            variables: HashMap::new(),
            expanded_refs: HashSet::new(),
            output_lines: Vec::new(),
            watches: Vec::new(),
            exception_filters: Vec::new(),
            panels_visible: false,
            panel_pinned: false,
            panel: PanelUi::default(),
            execution_file: None,
            execution_line: None,
        }
    }

    /// Toggle a breakpoint at the given line in the given file.
    /// Returns the new set of breakpoint lines for that file.
    pub fn toggle_breakpoint(&mut self, path: &Path, line: u64) -> Vec<u64> {
        let entry = self.breakpoints.entry(path.to_path_buf()).or_default();

        if entry.iter().any(|bp| bp.shown_line() == line) {
            entry.retain(|bp| bp.shown_line() != line);
        } else {
            entry.push(BreakpointState::new(line));
            entry.sort_by_key(BreakpointState::shown_line);
        }

        entry.iter().map(BreakpointState::shown_line).collect()
    }

    /// Lines to draw breakpoints on for a file.
    pub fn breakpoint_lines(&self, path: &Path) -> Vec<u64> {
        self.breakpoints
            .get(path)
            .map(|bps| bps.iter().map(BreakpointState::shown_line).collect())
            .unwrap_or_default()
    }

    /// The enabled breakpoints the adapter should know about, in the order
    /// they are sent.
    pub fn enabled_breakpoints(&self, path: &Path) -> Vec<BreakpointState> {
        self.breakpoints
            .get(path)
            .map(|bps| bps.iter().filter(|bp| bp.enabled).cloned().collect())
            .unwrap_or_default()
    }

    /// Whether the breakpoint at `line` exists and is enabled.
    pub fn is_breakpoint_enabled(&self, path: &Path, line: u64) -> bool {
        self.breakpoint_at(path, line).is_some_and(|bp| bp.enabled)
    }

    /// Removes a breakpoint. Returns whether one existed.
    pub fn remove_breakpoint(&mut self, path: &Path, line: u64) -> bool {
        let Some(entry) = self.breakpoints.get_mut(path) else {
            return false;
        };
        let before = entry.len();
        entry.retain(|bp| bp.shown_line() != line);
        // The (now empty) entry stays so the next sync tells the adapter
        // that the file has no breakpoints any more.
        entry.len() != before
    }

    /// Enables or disables a breakpoint. Returns the new state, or `None`
    /// when there is no breakpoint at that line.
    pub fn toggle_breakpoint_enabled(&mut self, path: &Path, line: u64) -> Option<bool> {
        let bp = self
            .breakpoints
            .get_mut(path)?
            .iter_mut()
            .find(|bp| bp.shown_line() == line)?;
        bp.enabled = !bp.enabled;
        Some(bp.enabled)
    }

    /// Every breakpoint, ordered by file and line.
    pub fn all_breakpoints(&self) -> Vec<(&Path, &BreakpointState)> {
        let mut all: Vec<(&Path, &BreakpointState)> = self
            .breakpoints
            .iter()
            .flat_map(|(path, bps)| bps.iter().map(move |bp| (path.as_path(), bp)))
            .collect();
        all.sort_by(|a, b| a.0.cmp(b.0).then(a.1.shown_line().cmp(&b.1.shown_line())));
        all
    }

    /// Check if a line has a breakpoint.
    pub fn has_breakpoint(&self, path: &Path, line: u64) -> bool {
        self.breakpoint_at(path, line).is_some()
    }

    /// Applies the adapter's answer to a `setBreakpoints` request.
    ///
    /// `sent` holds the requested line of each breakpoint in the order they
    /// were sent. DAP answers with one entry per request, in the same order,
    /// so entries are matched by position: the line in an answer is where the
    /// adapter *put* the breakpoint (it may have moved it), and an unverified
    /// entry may leave it out. What the user attached to a breakpoint
    /// (condition, logpoint, hit count) never depends on the answer.
    pub fn update_breakpoints(&mut self, path: &Path, sent: &[u64], reply: &[DapBreakpoint]) {
        let Some(entry) = self.breakpoints.get_mut(path) else {
            return;
        };
        let mut answered = vec![false; entry.len()];
        for (requested, dap_bp) in sent.iter().zip(reply) {
            // The user may have changed breakpoints while the request was in
            // flight; one that is gone has nothing to update. Duplicates of a
            // requested line are answered in order.
            let Some(index) = entry
                .iter()
                .enumerate()
                .position(|(i, bp)| bp.line == *requested && bp.enabled && !answered[i])
            else {
                continue;
            };
            answered[index] = true;
            let bp = &mut entry[index];
            bp.verified = dap_bp.verified;
            bp.id = dap_bp.id;
            bp.actual_line = dap_bp.line.filter(|line| *line != bp.line);
        }
        entry.sort_by_key(BreakpointState::shown_line);
    }

    /// Marks the breakpoints requested at `lines` as not known to the adapter.
    pub fn mark_breakpoints_unverified(&mut self, path: &Path, lines: &[u64]) {
        let Some(entry) = self.breakpoints.get_mut(path) else {
            return;
        };
        for bp in entry.iter_mut().filter(|bp| lines.contains(&bp.line)) {
            bp.verified = false;
            bp.actual_line = None;
        }
    }

    /// Update the execution position from the selected stack frame.
    pub fn update_execution_position(&mut self) {
        if let Some(frame) = self.stack_frames.get(self.selected_frame) {
            self.execution_line = Some(frame.line);
            self.execution_file = frame
                .source
                .as_ref()
                .and_then(|s| s.path.as_ref())
                .map(PathBuf::from);
        } else {
            self.execution_line = None;
            self.execution_file = None;
        }
    }

    /// Set a condition on a breakpoint. If the breakpoint doesn't exist, creates it.
    pub fn set_breakpoint_condition(&mut self, path: &Path, line: u64, condition: Option<String>) {
        self.edit_breakpoint(path, line, |bp| bp.condition = condition);
    }

    /// Makes the breakpoint at `line` a logpoint (`None` makes it a plain
    /// breakpoint again). Creates the breakpoint when there is none.
    pub fn set_breakpoint_log_message(&mut self, path: &Path, line: u64, message: Option<String>) {
        self.edit_breakpoint(path, line, |bp| bp.log_message = message);
    }

    /// Sets the hit-count condition of the breakpoint at `line`.
    pub fn set_breakpoint_hit_condition(
        &mut self,
        path: &Path,
        line: u64,
        hit_condition: Option<String>,
    ) {
        self.edit_breakpoint(path, line, |bp| bp.hit_condition = hit_condition);
    }

    fn edit_breakpoint(&mut self, path: &Path, line: u64, edit: impl FnOnce(&mut BreakpointState)) {
        let entry = self.breakpoints.entry(path.to_path_buf()).or_default();
        if let Some(bp) = entry.iter_mut().find(|bp| bp.shown_line() == line) {
            edit(bp);
        } else {
            let mut bp = BreakpointState::new(line);
            edit(&mut bp);
            entry.push(bp);
            entry.sort_by_key(BreakpointState::shown_line);
        }
    }

    /// The breakpoint drawn at `line`, if any.
    pub fn breakpoint_at(&self, path: &Path, line: u64) -> Option<&BreakpointState> {
        self.breakpoints
            .get(path)?
            .iter()
            .find(|bp| bp.shown_line() == line)
    }

    /// Get the condition for a breakpoint at a given line, if any.
    pub fn breakpoint_condition(&self, path: &Path, line: u64) -> Option<&str> {
        self.breakpoint_at(path, line)
            .and_then(|bp| bp.condition.as_deref())
    }

    /// Check if a breakpoint at the given line is conditional.
    pub fn is_conditional_breakpoint(&self, path: &Path, line: u64) -> bool {
        self.breakpoint_condition(path, line).is_some()
    }

    /// Clear all live debug state (on session end).
    ///
    /// Output lines are deliberately kept: they are the only record of why a
    /// program ended. They are reset when the next session starts.
    pub fn clear(&mut self) {
        self.end_session_keep_output();
        // Keep breakpoints — they persist across sessions.
    }

    /// Everything tied to a live debuggee goes: session flag, stop state,
    /// frames, variables and the execution marker.
    pub fn end_session_keep_output(&mut self) {
        self.session_active = false;
        self.is_running = false;
        self.stopped_thread = None;
        self.event_thread = None;
        self.threads.clear();
        self.stop_reason = None;
        self.exception = None;
        self.stack_frames.clear();
        self.selected_frame = 0;
        self.scopes.clear();
        self.variables.clear();
        self.expanded_refs.clear();
        self.execution_file = None;
        self.execution_line = None;
        self.clear_watch_values();
    }

    /// Forgets watch results (they belong to a stop that no longer exists);
    /// the expressions stay.
    pub fn clear_watch_values(&mut self) {
        for watch in &mut self.watches {
            watch.result = None;
            watch.type_ = None;
            watch.variables_reference = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "/p/A.java";

    fn file() -> &'static Path {
        Path::new(FILE)
    }

    fn reply(line: Option<u64>, verified: bool) -> DapBreakpoint {
        DapBreakpoint {
            id: Some(7),
            verified,
            message: None,
            line,
        }
    }

    #[test]
    fn a_breakpoint_the_adapter_moves_keeps_its_condition_and_requested_line() {
        let mut s = DebugState::new();
        s.set_breakpoint_condition(file(), 10, Some("n > 1".into()));
        s.set_breakpoint_log_message(file(), 20, Some("hit {n}".into()));
        s.set_breakpoint_hit_condition(file(), 30, Some("%2".into()));

        // The adapter moves each to the next executable line.
        s.update_breakpoints(
            file(),
            &[10, 20, 30],
            &[
                reply(Some(12), true),
                reply(Some(22), true),
                reply(Some(33), true),
            ],
        );

        let sent = s.enabled_breakpoints(file());
        assert_eq!(
            sent.iter().map(|bp| bp.line).collect::<Vec<_>>(),
            vec![10, 20, 30],
            "the next sync asks for the lines the user chose"
        );
        assert_eq!(sent[0].condition.as_deref(), Some("n > 1"));
        assert_eq!(sent[1].log_message.as_deref(), Some("hit {n}"));
        assert_eq!(sent[2].hit_condition.as_deref(), Some("%2"));
        // Drawn and found where the adapter put them.
        assert_eq!(s.breakpoint_lines(file()), vec![12, 22, 33]);
        assert!(s.is_conditional_breakpoint(file(), 12));
        assert!(!s.has_breakpoint(file(), 10));
        assert!(sent.iter().all(|bp| bp.verified));
    }

    #[test]
    fn an_unverified_answer_without_a_line_keeps_the_breakpoint() {
        let mut s = DebugState::new();
        s.set_breakpoint_condition(file(), 10, Some("n > 1".into()));
        s.toggle_breakpoint(file(), 20);

        s.update_breakpoints(
            file(),
            &[10, 20],
            &[reply(None, false), reply(Some(20), true)],
        );

        assert_eq!(s.breakpoint_lines(file()), vec![10, 20]);
        let first = s.breakpoint_at(file(), 10).unwrap();
        assert!(!first.verified);
        assert_eq!(first.condition.as_deref(), Some("n > 1"));
        assert!(s.breakpoint_at(file(), 20).unwrap().verified);
    }

    #[test]
    fn a_short_answer_does_not_delete_the_breakpoints_it_leaves_out() {
        let mut s = DebugState::new();
        s.toggle_breakpoint(file(), 10);
        s.toggle_breakpoint(file(), 20);

        s.update_breakpoints(file(), &[10, 20], &[reply(Some(10), true)]);

        assert_eq!(s.breakpoint_lines(file()), vec![10, 20]);
    }

    #[test]
    fn answers_for_breakpoints_removed_in_the_meantime_are_ignored() {
        let mut s = DebugState::new();
        s.toggle_breakpoint(file(), 10);
        s.toggle_breakpoint(file(), 20);
        s.remove_breakpoint(file(), 10);

        s.update_breakpoints(
            file(),
            &[10, 20],
            &[reply(Some(11), true), reply(Some(21), true)],
        );

        assert_eq!(s.breakpoint_lines(file()), vec![21]);
    }

    #[test]
    fn toggling_where_a_moved_breakpoint_is_drawn_removes_it() {
        let mut s = DebugState::new();
        s.toggle_breakpoint(file(), 10);
        s.update_breakpoints(file(), &[10], &[reply(Some(12), true)]);

        assert_eq!(s.toggle_breakpoint(file(), 12), Vec::<u64>::new());
        assert!(s.breakpoints[file()].is_empty());
    }
}
