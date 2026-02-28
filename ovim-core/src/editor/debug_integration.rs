//! Debug (DAP) integration for the Editor.
//!
//! Provides methods on `Editor` for breakpoint management, debug session
//! lifecycle, and stepping through code. Mirrors `lsp_integration.rs`.

use super::*;
use crate::dap::state::DebugState;
use crate::dap::DapManager;
use std::path::Path;

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
        self.mark_dirty();
        Some(lines)
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

    /// Process DAP events. Returns the number of events processed.
    pub fn process_dap_events(&mut self) -> usize {
        self.dap_manager.process_events()
    }

    /// Start a debug session by spawning a debug adapter.
    pub async fn start_debug_session(
        &mut self,
        command: &str,
        args: &[String],
    ) -> anyhow::Result<()> {
        self.dap_manager.start(command, args).await?;
        self.dap_manager.initialize().await?;
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
        self.mark_dirty();
        Ok(())
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
    pub async fn debug_fetch_variables(
        &mut self,
        variables_reference: u64,
    ) -> anyhow::Result<()> {
        let vars = self.dap_manager.variables(variables_reference).await?;
        self.dap_manager.state.variables.insert(variables_reference, vars);
        self.mark_dirty();
        Ok(())
    }

    /// Send breakpoints for a file to the debug adapter.
    pub async fn debug_sync_breakpoints(&mut self, path: &Path) -> anyhow::Result<()> {
        let lines = self.dap_manager.state.breakpoint_lines(path);
        self.dap_manager.set_breakpoints(path, &lines).await?;
        self.mark_dirty();
        Ok(())
    }

    /// Toggle debug panels visibility.
    pub fn toggle_debug_panels(&mut self) {
        self.dap_manager.state.panels_visible = !self.dap_manager.state.panels_visible;
        self.mark_dirty();
    }

    /// Select a stack frame by index.
    pub fn select_stack_frame(&mut self, index: usize) {
        if index < self.dap_manager.state.stack_frames.len() {
            self.dap_manager.state.selected_frame = index;
            self.dap_manager.state.update_execution_position();
            self.mark_dirty();
        }
    }

    /// Returns the current execution file and line (1-based), if any.
    pub fn execution_position(&self) -> Option<(&Path, u64)> {
        let file = self.dap_manager.state.execution_file.as_deref()?;
        let line = self.dap_manager.state.execution_line?;
        Some((file, line))
    }
}
