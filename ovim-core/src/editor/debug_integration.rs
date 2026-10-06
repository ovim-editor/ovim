//! Debug (DAP) integration for the Editor.
//!
//! Provides methods on `Editor` for breakpoint management, debug session
//! lifecycle, and stepping through code. Mirrors `lsp_integration.rs`.

use super::*;
use crate::dap::state::DebugState;
use crate::dap::DapManager;
use std::path::Path;

/// What `:DebugLogpoint` / `:DebugHitCount` attach to a breakpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointExtra {
    Logpoint,
    HitCount,
}

/// How a breakpoint is drawn in the gutter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointMarker {
    Enabled,
    Conditional,
    Disabled,
}

impl Editor {
    /// Returns a reference to the DAP manager.
    pub fn dap_manager(&self) -> &DapManager {
        &self.dap_manager
    }

    /// Returns a mutable reference to the DAP manager.
    pub fn dap_manager_mut(&mut self) -> &mut DapManager {
        &mut self.dap_manager
    }

    /// Returns a reference to the debug state.
    pub fn debug_state(&self) -> &DebugState {
        &self.dap_manager.state
    }

    /// Whether a debug session is currently active.
    pub fn is_debug_active(&self) -> bool {
        self.dap_manager.is_active()
    }

    /// Toggle a breakpoint at the cursor line in the current file.
    /// Returns the updated list of breakpoint lines for that file, or `None`
    /// if the current buffer has no file path.
    pub fn toggle_breakpoint(&mut self) -> Option<Vec<u64>> {
        let file_path = self.buffer().file_path()?.to_string();
        let line = self.buffer().cursor().line() as u64 + 1; // DAP uses 1-based lines
        let path = std::path::PathBuf::from(&file_path);
        let lines = self.dap_manager.state.toggle_breakpoint(&path, line);
        if self.dap_manager.is_active() {
            // A live session must learn about the change immediately.
            self.dap_manager.request_breakpoint_sync();
        }
        self.mark_dirty();
        Some(lines)
    }

    /// Toggles a breakpoint at a 1-based line of `path` (gutter click).
    pub fn toggle_breakpoint_at(&mut self, path: &Path, line: u64) {
        self.dap_manager.state.toggle_breakpoint(path, line);
        self.after_breakpoint_change();
    }

    /// Get breakpoint lines for the current file (1-based).
    pub fn current_file_breakpoint_lines(&self) -> Vec<u64> {
        let Some(file_path) = self.buffer().file_path() else {
            return Vec::new();
        };
        let path = std::path::PathBuf::from(file_path);
        self.dap_manager.state.breakpoint_lines(&path)
    }

    /// Check if a line (1-based) has a breakpoint in the current file.
    pub fn has_breakpoint_at(&self, line_1based: u64) -> bool {
        let Some(file_path) = self.buffer().file_path() else {
            return false;
        };
        let path = std::path::PathBuf::from(file_path);
        self.dap_manager.state.has_breakpoint(&path, line_1based)
    }

    /// How the breakpoint on a line (1-based) of the current file should be
    /// drawn: `None` when there is none.
    pub fn breakpoint_marker_at(&self, line_1based: u64) -> Option<BreakpointMarker> {
        let file_path = self.buffer().file_path()?;
        self.breakpoint_marker_in(Path::new(file_path), line_1based)
    }

    /// Like [`breakpoint_marker_at`](Self::breakpoint_marker_at) for any file.
    pub fn breakpoint_marker_in(&self, path: &Path, line_1based: u64) -> Option<BreakpointMarker> {
        let bp = self
            .dap_manager
            .state
            .breakpoints
            .get(path)?
            .iter()
            .find(|bp| bp.line == line_1based)?;
        Some(if !bp.enabled {
            BreakpointMarker::Disabled
        } else if bp.condition.is_some() || bp.log_message.is_some() || bp.hit_condition.is_some() {
            BreakpointMarker::Conditional
        } else {
            BreakpointMarker::Enabled
        })
    }

    /// Process DAP events. Returns the number of events processed.
    pub fn process_dap_events(&mut self) -> usize {
        self.dap_manager.process_events()
    }

    /// Start a debug session by spawning a debug adapter and initialising
    /// it. `attach` is sent once the adapter reports `initialized`. A failure
    /// leaves nothing behind: the adapter is killed and state is reset.
    pub async fn start_debug_session(
        &mut self,
        command: &str,
        args: &[String],
        attach: serde_json::Value,
    ) -> anyhow::Result<()> {
        self.dap_manager.attach_request = Some(attach);
        let result = async {
            self.dap_manager.start(command, args).await?;
            self.dap_manager.initialize().await
        }
        .await;
        if let Err(e) = result {
            let _ = self.dap_manager.disconnect().await;
            self.mark_dirty();
            return Err(e);
        }
        self.mark_dirty();
        Ok(())
    }

    /// Stop the current debug session.
    pub async fn stop_debug_session(&mut self) -> anyhow::Result<()> {
        self.dap_manager.disconnect().await?;
        self.mark_dirty();
        Ok(())
    }

    /// Continue execution (resume from stopped state).
    pub async fn debug_continue(&mut self) -> anyhow::Result<()> {
        let thread_id = self.dap_manager.state.stopped_thread.unwrap_or(1);
        self.dap_manager.continue_(thread_id).await?;
        self.dap_manager.state.is_running = true;
        self.mark_dirty();
        Ok(())
    }

    /// Step over (next line).
    pub async fn debug_step_over(&mut self) -> anyhow::Result<()> {
        let thread_id = self.dap_manager.state.stopped_thread.unwrap_or(1);
        self.dap_manager.next(thread_id).await?;
        self.mark_dirty();
        Ok(())
    }

    /// Step into.
    pub async fn debug_step_in(&mut self) -> anyhow::Result<()> {
        let thread_id = self.dap_manager.state.stopped_thread.unwrap_or(1);
        self.dap_manager.step_in(thread_id).await?;
        self.mark_dirty();
        Ok(())
    }

    /// Step out.
    pub async fn debug_step_out(&mut self) -> anyhow::Result<()> {
        let thread_id = self.dap_manager.state.stopped_thread.unwrap_or(1);
        self.dap_manager.step_out(thread_id).await?;
        self.mark_dirty();
        Ok(())
    }

    /// Fetch and store the stack trace for the stopped thread.
    pub async fn debug_fetch_stack_trace(&mut self) -> anyhow::Result<()> {
        let thread_id = self.dap_manager.state.stopped_thread.unwrap_or(1);
        let frames = self.dap_manager.stack_trace(thread_id).await?;
        self.dap_manager.state.stack_frames = frames;
        self.dap_manager.state.selected_frame = 0;
        self.dap_manager.state.update_execution_position();
        // Show where the debuggee stopped, even in a file that is not open.
        self.show_frame_source(0);
        self.mark_dirty();
        Ok(())
    }

    /// Lists the debuggee's threads for the panel (best effort).
    pub async fn debug_fetch_threads(&mut self) {
        if let Ok(threads) = self.dap_manager.threads().await {
            // Not if the debuggee resumed in the meantime.
            if !self.dap_manager.state.is_running {
                self.dap_manager.state.threads = threads;
            }
        }
        self.mark_dirty();
    }

    /// When the debuggee stopped on an exception: ask the adapter what was
    /// thrown and show it in the panel, the status line and the console.
    /// The stop event's own description is the fallback.
    pub async fn debug_fetch_exception_info(&mut self) {
        if self.dap_manager.state.stop_reason.as_deref() != Some("exception") {
            return;
        }
        let thread_id = self.dap_manager.state.event_thread.unwrap_or(1);
        if self.dap_manager.state.stopped_thread.unwrap_or(thread_id) != thread_id {
            return;
        }
        let summary = match self.dap_manager.exception_info(thread_id).await {
            Ok(info) => Some(info.summary()),
            Err(_) => self.dap_manager.state.exception.clone(),
        };
        // The debuggee may have been resumed while the request was in flight.
        if self.dap_manager.state.stop_reason.as_deref() != Some("exception") {
            return;
        }
        if let Some(summary) = summary {
            self.set_status_message(format!("Exception: {summary}"));
            self.dap_manager
                .log_console(format!("Stopped on exception: {summary}"));
            self.dap_manager.state.exception = Some(summary);
        }
        self.mark_dirty();
    }

    /// Fetch and store scopes for the currently selected frame.
    pub async fn debug_fetch_scopes(&mut self) -> anyhow::Result<()> {
        let frame_id = self
            .dap_manager
            .state
            .stack_frames
            .get(self.dap_manager.state.selected_frame)
            .map(|f| f.id)
            .unwrap_or(0);
        let scopes = self.dap_manager.scopes(frame_id).await?;
        self.dap_manager.state.scopes = scopes;
        self.mark_dirty();
        Ok(())
    }

    /// Fetch and store variables for a given reference.
    pub async fn debug_fetch_variables(&mut self, variables_reference: u64) -> anyhow::Result<()> {
        let vars = self.dap_manager.variables(variables_reference).await?;
        self.dap_manager
            .state
            .variables
            .insert(variables_reference, vars);
        self.mark_dirty();
        Ok(())
    }

    /// Send breakpoints for a file to the debug adapter.
    pub async fn debug_sync_breakpoints(&mut self, path: &Path) -> anyhow::Result<()> {
        let lines = self.dap_manager.state.enabled_breakpoint_lines(path);
        self.dap_manager.set_breakpoints(path, &lines).await?;
        self.mark_dirty();
        Ok(())
    }

    /// Toggle debug panels visibility. An explicit toggle pins the panel: it
    /// then stays across sessions (breakpoints, watches) until toggled again.
    pub fn toggle_debug_panels(&mut self) {
        let state = &mut self.dap_manager.state;
        state.panels_visible = !state.panels_visible;
        state.panel_pinned = state.panels_visible;
        if !state.panels_visible && self.mode == crate::mode::Mode::DebugPanel {
            self.mode = crate::mode::Mode::Normal;
        }
        self.mark_dirty();
    }

    // ------------------------------------------------------------------
    // The focusable debug panel (`<Space>df`)
    // ------------------------------------------------------------------

    /// Called by the renderer: how many rows are visible and where the view starts.
    pub fn debug_panel_view(&self, height: usize, scroll: usize) {
        let panel = &self.dap_manager.state.panel;
        panel.view_height.set(height);
        panel.scroll.set(scroll);
    }

    /// The panel's rows, top to bottom.
    pub fn debug_panel_rows(&self) -> Vec<crate::dap::panel::PanelRow> {
        crate::dap::panel::rows(&self.dap_manager.state)
    }

    /// Show the panel and give it keyboard focus (`<Space>df`).
    pub fn focus_debug_panel(&mut self) {
        let state = &mut self.dap_manager.state;
        state.panels_visible = true;
        state.panel_pinned = true;
        let rows = crate::dap::panel::rows(state);
        // Start on the selected frame, else the first actionable row.
        state.panel.cursor = rows
            .iter()
            .position(|r| {
                matches!(
                    r.kind,
                    crate::dap::panel::RowKind::Frame { selected: true, .. }
                )
            })
            .or_else(|| rows.iter().position(|r| r.is_actionable()))
            .unwrap_or(0);
        self.mode = crate::mode::Mode::DebugPanel;
        self.mark_dirty();
    }

    /// Moves the panel cursor by `delta` actionable rows.
    pub fn debug_panel_move(&mut self, delta: isize) {
        let rows = self.debug_panel_rows();
        let mut cursor = self
            .dap_manager
            .state
            .panel
            .cursor
            .min(rows.len().saturating_sub(1));
        let step = delta.signum();
        let mut remaining = delta.unsigned_abs();
        while remaining > 0 {
            let mut next = cursor as isize + step;
            // Skip headers and notes.
            while next >= 0 && (next as usize) < rows.len() && !rows[next as usize].is_actionable()
            {
                next += step;
            }
            if next < 0 || next as usize >= rows.len() {
                break;
            }
            cursor = next as usize;
            remaining -= 1;
        }
        self.dap_manager.state.panel.cursor = cursor;
        self.mark_dirty();
    }

    /// Jumps to the first / last actionable row.
    pub fn debug_panel_edge(&mut self, last: bool) {
        let rows = self.debug_panel_rows();
        let found = if last {
            rows.iter().rposition(|r| r.is_actionable())
        } else {
            rows.iter().position(|r| r.is_actionable())
        };
        if let Some(index) = found {
            self.dap_manager.state.panel.cursor = index;
        }
        self.mark_dirty();
    }

    fn debug_panel_current(&self) -> Option<(usize, crate::dap::panel::PanelRow)> {
        let rows = self.debug_panel_rows();
        let index = self
            .dap_manager
            .state
            .panel
            .cursor
            .min(rows.len().checked_sub(1)?);
        rows.get(index).cloned().map(|row| (index, row))
    }

    /// Expands or collapses a variable / watch value; fetches children on demand.
    pub fn toggle_variable_expansion(&mut self, var_ref: u64) {
        if var_ref == 0 {
            return;
        }
        let state = &mut self.dap_manager.state;
        if !state.expanded_refs.remove(&var_ref) {
            state.expanded_refs.insert(var_ref);
            if !state.variables.contains_key(&var_ref) {
                self.dap_manager.pending_action =
                    Some(crate::dap::PendingDebugAction::FetchVariables { var_ref });
            }
        }
        self.mark_dirty();
    }

    /// Enter / Space / `l` on the current row.
    pub fn debug_panel_activate(&mut self) {
        use crate::dap::panel::RowKind;
        let Some((_, row)) = self.debug_panel_current() else {
            return;
        };
        match row.kind {
            RowKind::Frame { index, .. } => self.select_stack_frame(index),
            RowKind::Variable { var_ref, .. } | RowKind::Watch { var_ref, .. } => {
                self.toggle_variable_expansion(var_ref)
            }
            RowKind::Breakpoint { path, line, .. } => {
                self.open_location(&path, line as usize, 1);
            }
            RowKind::Exception { index, .. } => {
                self.toggle_exception_filter_at(index);
            }
            RowKind::Thread { id, .. } => self.select_debug_thread(id),
            RowKind::Header | RowKind::Note => {}
        }
        self.mark_dirty();
    }

    /// Inspects another thread: its stack and variables replace the shown ones.
    pub fn select_debug_thread(&mut self, id: u64) {
        let state = &mut self.dap_manager.state;
        if state.is_running || state.stopped_thread == Some(id) {
            return;
        }
        state.stopped_thread = Some(id);
        state.stack_frames.clear();
        state.scopes.clear();
        state.variables.clear();
        state.clear_watch_values();
        self.dap_manager.pending_action = Some(crate::dap::PendingDebugAction::FetchState);
        self.mark_dirty();
    }

    /// `h` on the current row: collapse it, or move to its parent.
    pub fn debug_panel_collapse(&mut self) {
        use crate::dap::panel::RowKind;
        let Some((_, row)) = self.debug_panel_current() else {
            return;
        };
        match row.kind {
            RowKind::Variable {
                var_ref,
                expanded: true,
            }
            | RowKind::Watch {
                var_ref,
                expanded: true,
                ..
            } => self.toggle_variable_expansion(var_ref),
            _ => {
                if let Some(parent) = row.parent {
                    self.dap_manager.state.panel.cursor = parent;
                }
            }
        }
        self.mark_dirty();
    }

    /// `d` / `x`: delete the breakpoint or watch under the cursor.
    pub fn debug_panel_delete(&mut self) {
        use crate::dap::panel::RowKind;
        let Some((_, row)) = self.debug_panel_current() else {
            return;
        };
        match row.kind {
            RowKind::Breakpoint { path, line, .. } => {
                self.dap_manager.state.remove_breakpoint(&path, line);
                self.after_breakpoint_change();
            }
            RowKind::Watch { index, .. } => {
                self.remove_watch(index);
            }
            _ => self.set_status_message("Nothing to delete here (breakpoints and watches only)"),
        }
        self.mark_dirty();
    }

    /// `e` / `t`: enable or disable the breakpoint (or exception filter) under the cursor.
    pub fn debug_panel_toggle_enabled(&mut self) {
        use crate::dap::panel::RowKind;
        let Some((_, row)) = self.debug_panel_current() else {
            return;
        };
        match row.kind {
            RowKind::Breakpoint { path, line, .. } => {
                self.dap_manager
                    .state
                    .toggle_breakpoint_enabled(&path, line);
                self.after_breakpoint_change();
            }
            RowKind::Exception { index, .. } => self.toggle_exception_filter_at(index),
            _ => self.set_status_message("Only breakpoints and exception filters can be disabled"),
        }
        self.mark_dirty();
    }

    /// Widens (`delta > 0`) or narrows the debug panel.
    pub fn debug_panel_resize(&mut self, delta: i16) {
        let panel = &mut self.dap_manager.state.panel;
        panel.width_delta = (panel.width_delta + delta).clamp(-20, 80);
        self.mark_dirty();
    }

    /// `:PanelSize test|debug|console <+N|-N|N|reset>`: widens or narrows
    /// (or heightens, for the console) a bottom/side panel. Returns what to
    /// tell the user.
    pub fn resize_panel(&mut self, panel: &str, amount: &str) -> Result<String, String> {
        let amount = amount.trim();
        let parse = |current: i16| -> Result<i16, String> {
            if amount == "reset" || amount.is_empty() {
                return Ok(0);
            }
            let n: i16 = amount
                .trim_start_matches('+')
                .parse()
                .map_err(|_| format!("Not a number: '{amount}' (use +N, -N or reset)"))?;
            Ok(if amount.starts_with('+') || amount.starts_with('-') {
                current.saturating_add(n)
            } else {
                n
            })
        };
        let message = match panel {
            "test" | "tests" => {
                let delta = parse(self.test_panel().width_delta)?;
                self.set_test_panel_width_delta(i32::from(delta));
                format!("Test panel width offset {}", self.test_panel().width_delta)
            }
            "debug" => {
                let p = &mut self.dap_manager.state.panel;
                p.width_delta = parse(p.width_delta)?.clamp(-20, 80);
                format!("Debug panel width offset {}", p.width_delta)
            }
            "console" | "run" => {
                let c = &mut self.launch.console;
                c.height_delta = parse(c.height_delta)?.clamp(-10, 40);
                format!("Run console height offset {}", c.height_delta)
            }
            other => return Err(format!("Unknown panel '{other}' (test, debug or console)")),
        };
        self.mark_dirty();
        Ok(message)
    }

    /// Tells a live session about changed breakpoints.
    fn after_breakpoint_change(&mut self) {
        if self.dap_manager.is_active() {
            self.dap_manager.request_breakpoint_sync();
        }
        self.mark_dirty();
    }

    // ---- Breakpoint list commands ----

    /// Enables or disables every breakpoint (`:DebugBreakpoints on|off`).
    pub fn set_all_breakpoints_enabled(&mut self, enabled: bool) {
        for bps in self.dap_manager.state.breakpoints.values_mut() {
            for bp in bps {
                bp.enabled = enabled;
            }
        }
        self.after_breakpoint_change();
    }

    /// Removes every breakpoint (`:DebugBreakpoints clear`).
    pub fn clear_all_breakpoints(&mut self) {
        for bps in self.dap_manager.state.breakpoints.values_mut() {
            bps.clear();
        }
        self.after_breakpoint_change();
    }

    // ---- Watch expressions ----

    /// Adds a watch expression (evaluated at the current stop, and at every
    /// later one).
    pub fn add_watch(&mut self, expression: String) {
        let expression = expression.trim().to_string();
        if expression.is_empty()
            || self
                .dap_manager
                .state
                .watches
                .iter()
                .any(|w| w.expression == expression)
        {
            return;
        }
        self.dap_manager
            .state
            .watches
            .push(crate::dap::state::Watch {
                expression,
                result: None,
                type_: None,
                variables_reference: 0,
            });
        if self.is_debug_stopped() {
            self.dap_manager.pending_action = Some(crate::dap::PendingDebugAction::RefreshWatches);
        }
        self.mark_dirty();
    }

    /// Removes the watch at `index`.
    pub fn remove_watch(&mut self, index: usize) {
        let watches = &mut self.dap_manager.state.watches;
        if index < watches.len() {
            watches.remove(index);
        }
        self.mark_dirty();
    }

    /// Re-evaluates every watch in the selected frame.
    pub async fn debug_refresh_watches(&mut self) {
        let frame_id = self.selected_frame_id();
        let expressions: Vec<String> = self
            .dap_manager
            .state
            .watches
            .iter()
            .map(|w| w.expression.clone())
            .collect();
        for (index, expression) in expressions.into_iter().enumerate() {
            let result = self
                .dap_manager
                .evaluate(&expression, frame_id, Some("watch"))
                .await;
            if let Some(watch) = self.dap_manager.state.watches.get_mut(index) {
                if watch.expression != expression {
                    continue;
                }
                match result {
                    Ok((value, type_, var_ref)) => {
                        watch.result = Some(Ok(value));
                        watch.type_ = type_;
                        watch.variables_reference = var_ref;
                    }
                    Err(e) => {
                        watch.result = Some(Err(e.to_string()));
                        watch.type_ = None;
                        watch.variables_reference = 0;
                    }
                }
            }
        }
        self.mark_dirty();
    }

    // ---- Exception breakpoints ----

    fn toggle_exception_filter_at(&mut self, index: usize) {
        let Some(filter) = self.dap_manager.state.exception_filters.get_mut(index) else {
            return;
        };
        filter.enabled = !filter.enabled;
        let message = format!(
            "Break on {}: {}",
            filter.label,
            if filter.enabled { "on" } else { "off" }
        );
        self.set_status_message(message);
        self.after_breakpoint_change();
    }

    /// `:DebugException [name]` — toggles the exception filter whose id or
    /// label contains `name` (no name: the first filter, e.g. "All exceptions").
    pub fn toggle_exception_filter(&mut self, name: &str) -> Result<(), String> {
        let filters = &self.dap_manager.state.exception_filters;
        if filters.is_empty() {
            return Err(
                "No exception filters yet: the debug adapter offers them once a session has started"
                    .to_string(),
            );
        }
        let needle = name.trim().to_lowercase();
        let index = if needle.is_empty() {
            Some(0)
        } else {
            filters.iter().position(|f| {
                f.id.to_lowercase().contains(&needle) || f.label.to_lowercase().contains(&needle)
            })
        };
        match index {
            Some(index) => {
                self.toggle_exception_filter_at(index);
                Ok(())
            }
            None => Err(format!(
                "No exception filter matches '{name}' (have: {})",
                filters
                    .iter()
                    .map(|f| f.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// The expression under the cursor for hover-evaluate: the visual
    /// selection, or the identifier with the member chain before it
    /// (`user.address.city` when the cursor is on `city`).
    pub fn debug_expression_at_cursor(&self) -> Option<String> {
        let line = self.buffer().line_text(self.buffer().cursor().line())?;
        let col = self.buffer().cursor().col().0;
        let chars: Vec<char> = line.chars().collect();
        let is_ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
        let mut at = col.min(chars.len().saturating_sub(1));
        if chars.is_empty() || !is_ident(chars[at]) {
            return None;
        }
        let mut end = at;
        while end + 1 < chars.len() && is_ident(chars[end + 1]) {
            end += 1;
        }
        while at > 0 && is_ident(chars[at - 1]) {
            at -= 1;
        }
        // Extend left over `ident.` pairs (and `this.`).
        let mut start = at;
        while start > 0 && chars[start - 1] == '.' {
            let mut s = start - 1;
            if s == 0 || !is_ident(chars[s - 1]) {
                break;
            }
            while s > 0 && is_ident(chars[s - 1]) {
                s -= 1;
            }
            start = s;
        }
        Some(chars[start..=end].iter().collect())
    }

    /// Select a stack frame by index. Queues fetch of scopes/variables for the new frame
    /// and navigates the editor to the frame's source location.
    pub fn select_stack_frame(&mut self, index: usize) {
        if index < self.dap_manager.state.stack_frames.len() {
            self.dap_manager.state.selected_frame = index;
            self.dap_manager.state.update_execution_position();

            self.show_frame_source(index);

            // Queue scopes + variables refresh for the new frame.
            self.dap_manager.pending_action =
                Some(crate::dap::PendingDebugAction::SelectFrame { index });
            self.mark_dirty();
        }
    }

    /// Opens the frame's source file (when it has one) and puts the cursor on
    /// its line.
    fn show_frame_source(&mut self, index: usize) {
        let Some(frame) = self.dap_manager.state.stack_frames.get(index) else {
            return;
        };
        let line = frame.line.saturating_sub(1) as usize;
        let Some(path) = frame.source.as_ref().and_then(|s| s.path.clone()) else {
            return;
        };
        if self.load_file(&path).is_ok() {
            self.buffer_mut()
                .cursor_mut()
                .set_position(line, crate::unicode::GraphemeCol::ZERO);
            self.buffer_mut().validate_cursor_position();
        }
    }

    /// Select the next frame up (caller).
    pub fn select_frame_up(&mut self) {
        let current = self.dap_manager.state.selected_frame;
        let max = self.dap_manager.state.stack_frames.len();
        if max > 0 && current + 1 < max {
            self.select_stack_frame(current + 1);
        }
    }

    /// Select the next frame down (callee).
    pub fn select_frame_down(&mut self) {
        let current = self.dap_manager.state.selected_frame;
        if current > 0 {
            self.select_stack_frame(current - 1);
        }
    }

    /// Whether the debugger is stopped (at a breakpoint/step, not running).
    pub fn is_debug_stopped(&self) -> bool {
        self.dap_manager.is_active()
            && !self.dap_manager.state.is_running
            && self.dap_manager.state.stopped_thread.is_some()
    }

    /// Get the selected frame's DAP frame ID (for evaluate context).
    pub fn selected_frame_id(&self) -> Option<u64> {
        self.dap_manager
            .state
            .stack_frames
            .get(self.dap_manager.state.selected_frame)
            .map(|f| f.id)
    }

    /// Toggle a conditional breakpoint — prompts for condition via command line.
    pub fn toggle_conditional_breakpoint(&mut self, condition: String) {
        let Some(file_path) = self.buffer().file_path().map(|s| s.to_string()) else {
            return;
        };
        let line = self.buffer().cursor().line() as u64 + 1;
        let path = std::path::PathBuf::from(&file_path);
        self.dap_manager
            .state
            .set_breakpoint_condition(&path, line, Some(condition));
        self.mark_dirty();
    }

    /// `:DebugLogpoint <message>` / `:DebugHitCount <n>`: attaches a log
    /// message or a hit condition to the breakpoint at the cursor (creating
    /// it); an empty value removes it. Returns what to tell the user.
    pub fn set_cursor_breakpoint_extra(&mut self, kind: BreakpointExtra, value: &str) -> String {
        let Some(file_path) = self.buffer().file_path().map(|s| s.to_string()) else {
            return "Save the buffer to a file first".to_string();
        };
        let line = self.buffer().cursor().line() as u64 + 1;
        let path = std::path::PathBuf::from(&file_path);
        let value = Some(value.trim().to_string()).filter(|v| !v.is_empty());
        let supported = match kind {
            BreakpointExtra::Logpoint => self.dap_manager.supports_log_points(),
            BreakpointExtra::HitCount => self.dap_manager.supports_hit_conditions(),
        };
        let state = &mut self.dap_manager.state;
        let removed = value.is_none();
        match kind {
            BreakpointExtra::Logpoint => state.set_breakpoint_log_message(&path, line, value),
            BreakpointExtra::HitCount => state.set_breakpoint_hit_condition(&path, line, value),
        }
        self.after_breakpoint_change();
        let what = match kind {
            BreakpointExtra::Logpoint => "Logpoint",
            BreakpointExtra::HitCount => "Hit count",
        };
        match (removed, supported) {
            (true, _) => format!("{what} removed"),
            (false, Some(false)) => format!(
                "{what} set, but the running debug adapter does not support it (the breakpoint is not set until it does)"
            ),
            (false, _) => format!("{what} set at line {line}"),
        }
    }

    /// The 1-based execution line when the debuggee is stopped in the file of
    /// the buffer being shown (the marker belongs to that buffer only).
    pub fn execution_line_in_current_buffer(&self) -> Option<u64> {
        let (file, line) = self.execution_position()?;
        let current = Path::new(self.buffer().file_path()?);
        (file == current).then_some(line)
    }

    /// Returns the current execution file and line (1-based), if any.
    pub fn execution_position(&self) -> Option<(&Path, u64)> {
        let file = self.dap_manager.state.execution_file.as_deref()?;
        let line = self.dap_manager.state.execution_line?;
        Some((file, line))
    }
}
