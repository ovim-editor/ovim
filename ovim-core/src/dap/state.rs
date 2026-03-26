//! Debug state tracking.
//!
//! Holds all debug-related state: breakpoints, stack frames, variables, output.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::types::{DapBreakpoint, DapScope, DapStackFrame, DapVariable};

/// Per-line breakpoint state.
#[derive(Debug, Clone)]
pub struct BreakpointState {
    /// 1-based line number.
    pub line: u64,
    /// Whether the debug adapter confirmed this breakpoint.
    pub verified: bool,
    /// DAP-assigned breakpoint ID.
    pub id: Option<u64>,
    /// Condition expression for conditional breakpoints (None = unconditional).
    pub condition: Option<String>,
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
    /// Reason for the stop (e.g., "breakpoint", "step", "exception").
    pub stop_reason: Option<String>,

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

    // ---- UI ----
    /// Whether debug panels are visible.
    pub panels_visible: bool,

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
            stop_reason: None,
            breakpoints: HashMap::new(),
            stack_frames: Vec::new(),
            selected_frame: 0,
            scopes: Vec::new(),
            variables: HashMap::new(),
            expanded_refs: HashSet::new(),
            output_lines: Vec::new(),
            panels_visible: false,
            execution_file: None,
            execution_line: None,
        }
    }

    /// Toggle a breakpoint at the given line in the given file.
    /// Returns the new set of breakpoint lines for that file.
    pub fn toggle_breakpoint(&mut self, path: &Path, line: u64) -> Vec<u64> {
        let entry = self.breakpoints.entry(path.to_path_buf()).or_default();

        if let Some(idx) = entry.iter().position(|bp| bp.line == line) {
            entry.remove(idx);
        } else {
            entry.push(BreakpointState {
                line,
                verified: false,
                id: None,
                condition: None,
            });
        }

        entry.iter().map(|bp| bp.line).collect()
    }

    /// Get breakpoint lines for a file.
    pub fn breakpoint_lines(&self, path: &Path) -> Vec<u64> {
        self.breakpoints
            .get(path)
            .map(|bps| bps.iter().map(|bp| bp.line).collect())
            .unwrap_or_default()
    }

    /// Check if a line has a breakpoint.
    pub fn has_breakpoint(&self, path: &Path, line: u64) -> bool {
        self.breakpoints
            .get(path)
            .is_some_and(|bps| bps.iter().any(|bp| bp.line == line))
    }

    /// Update breakpoints with responses from the debug adapter.
    pub fn update_breakpoints(&mut self, path: &Path, dap_bps: &[DapBreakpoint]) {
        let entry = self.breakpoints.entry(path.to_path_buf()).or_default();
        let old_entries = entry.clone();
        entry.clear();
        for bp in dap_bps {
            if let Some(line) = bp.line {
                // Preserve existing condition if the breakpoint was already there.
                let existing_condition = old_entries
                    .iter()
                    .find(|old| old.line == line)
                    .and_then(|old| old.condition.clone());
                entry.push(BreakpointState {
                    line,
                    verified: bp.verified,
                    id: bp.id,
                    condition: existing_condition,
                });
            }
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
        let entry = self.breakpoints.entry(path.to_path_buf()).or_default();
        if let Some(bp) = entry.iter_mut().find(|bp| bp.line == line) {
            bp.condition = condition;
        } else {
            entry.push(BreakpointState {
                line,
                verified: false,
                id: None,
                condition,
            });
        }
    }

    /// Get the condition for a breakpoint at a given line, if any.
    pub fn breakpoint_condition(&self, path: &Path, line: u64) -> Option<&str> {
        self.breakpoints
            .get(path)
            .and_then(|bps| bps.iter().find(|bp| bp.line == line))
            .and_then(|bp| bp.condition.as_deref())
    }

    /// Check if a breakpoint at the given line is conditional.
    pub fn is_conditional_breakpoint(&self, path: &Path, line: u64) -> bool {
        self.breakpoint_condition(path, line).is_some()
    }

    /// Clear all debug state (on session end).
    pub fn clear(&mut self) {
        self.session_active = false;
        self.is_running = false;
        self.stopped_thread = None;
        self.stop_reason = None;
        self.stack_frames.clear();
        self.selected_frame = 0;
        self.scopes.clear();
        self.variables.clear();
        self.expanded_refs.clear();
        self.output_lines.clear();
        self.execution_file = None;
        self.execution_line = None;
        // Keep breakpoints — they persist across sessions.
    }
}
