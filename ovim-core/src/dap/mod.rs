//! DAP (Debug Adapter Protocol) client implementation
//!
//! This module provides debug adapter support for ovim, enabling:
//! - Breakpoint management
//! - Step-through debugging (over, into, out)
//! - Stack trace inspection
//! - Variable inspection
//! - Debug output capture
//!
//! # Architecture
//!
//! - `DapManager`: Central coordinator managing a single debug session
//! - `DebugAdapterClient`: Individual debug adapter process management
//! - `protocol`: DAP message handling (Content-Length framing, same as LSP)
//! - `state`: Debug state (breakpoints, stack frames, variables)
//! - `types`: Client-side DAP type definitions

pub mod client;
pub mod follow;
pub mod panel;
pub mod protocol;
pub mod state;
pub mod types;

use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

use client::DebugAdapterClient;
use state::DebugState;
use types::*;

/// Event from the debug adapter to the editor.
#[derive(Debug, Clone)]
pub enum DapEvent {
    /// Debuggee stopped (breakpoint, step, exception, etc.)
    Stopped {
        reason: String,
        thread_id: Option<u64>,
        all_threads_stopped: bool,
        /// The adapter's `description`/`text` for the stop (for an exception
        /// stop: `Type: message`).
        description: Option<String>,
    },
    /// Debuggee continued execution.
    Continued { thread_id: u64 },
    /// Thread started or exited.
    Thread { reason: String, thread_id: u64 },
    /// Output from the debuggee.
    Output { category: String, output: String },
    /// The debuggee process exited (DAP `exited`), with its exit code.
    Exited { exit_code: Option<i32> },
    /// Debug session terminated.
    Terminated,
    /// The adapter process went away without saying goodbye: it crashed or
    /// was killed. `detail` is its exit status and last stderr lines.
    AdapterExited { detail: String },
    /// Debug adapter initialized (ready for configuration).
    Initialized,
}

/// Pending debug action to execute in the async event loop.
#[derive(Debug, Clone)]
pub enum PendingDebugAction {
    /// Spawn the debug adapter, then attach with `attach` (the DAP `attach`
    /// arguments, already fully resolved: see `launch::plan`). ovim starts the
    /// debuggee itself; the adapter never launches one.
    Start {
        command: String,
        args: Vec<String>,
        attach: serde_json::Value,
        /// Whether ending the session should also kill the debuggee. Only
        /// for programs ovim started itself: one the user attached to
        /// belongs to them and is left running.
        terminate_debuggee: bool,
    },
    /// Continue execution.
    Continue,
    /// Step over.
    StepOver,
    /// Step into.
    StepIn,
    /// Step out.
    StepOut,
    /// Fetch stack trace + scopes + variables for the stopped thread.
    FetchState,
    /// Send `attach` (from `attach_request`), then sync breakpoints.
    Attach,
    /// Sync all breakpoints to the adapter and send configurationDone.
    SyncBreakpoints,
    /// Select a stack frame and refresh variables.
    SelectFrame { index: usize },
    /// Evaluate an expression and show result.
    Evaluate { expression: String },
    /// Fetch variables for an expanded object reference.
    FetchVariables { var_ref: u64 },
    /// Re-evaluate the watch expressions in the selected frame.
    RefreshWatches,
    /// Evaluate an expression for the hover popup (`K`), with its children.
    EvaluateHover { expression: String },
}

/// Central coordinator for debug sessions.
pub struct DapManager {
    /// The active debug adapter client.
    client: Option<DebugAdapterClient>,
    /// Debug state (breakpoints, frames, variables).
    pub state: DebugState,
    /// Incoming events from the debug adapter.
    event_rx: mpsc::Receiver<DapEvent>,
    /// Sender side (given to the client).
    event_tx: mpsc::Sender<DapEvent>,
    /// Pending action to execute in the async event loop.
    pub pending_action: Option<PendingDebugAction>,
    /// Stop was requested. Kept apart from `pending_action` (a single slot
    /// that stop/step/fetch events overwrite) so a stop can never be lost.
    stop_requested: bool,
    /// Breakpoints or exception filters changed and the live session has to
    /// hear about it. Also its own flag: a session that keeps stopping queues
    /// a state fetch every tick, which would overwrite a queued sync.
    breakpoint_sync_requested: bool,
    /// The DAP `attach` arguments for the session being started.
    pub attach_request: Option<serde_json::Value>,
    /// Whether `disconnect` asks the adapter to terminate the debuggee.
    terminate_debuggee: bool,
    /// Debuggee/adapter output not yet copied into the run console.
    console_output: Vec<(String, String)>,
    /// Set when the session ended (adapter `terminated`/`exited`/EOF) and the
    /// editor has not yet acknowledged it.
    session_end: Option<SessionEnd>,
    /// Exit code from the most recent `exited` event.
    exit_code: Option<i32>,
    /// What the adapter said it can do (`initialize` response).
    capabilities: Option<DapCapabilities>,
}

/// How a debug session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEnd {
    pub exit_code: Option<i32>,
    /// Set when the adapter died mid-session (what it left behind).
    pub adapter_crash: Option<String>,
}

impl Default for DapManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DapManager {
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel(256);
        Self {
            client: None,
            state: DebugState::new(),
            event_rx,
            event_tx,
            pending_action: None,
            stop_requested: false,
            breakpoint_sync_requested: false,
            attach_request: None,
            terminate_debuggee: true,
            console_output: Vec::new(),
            session_end: None,
            exit_code: None,
            capabilities: None,
        }
    }

    /// Sets whether ending the session terminates the debuggee (see
    /// [`PendingDebugAction::Start`]).
    pub fn set_terminate_debuggee(&mut self, terminate: bool) {
        self.terminate_debuggee = terminate;
    }

    /// Start a debug adapter process.
    pub async fn start(&mut self, command: &str, args: &[String]) -> Result<()> {
        // A previous session may have left an adapter behind; never leak it.
        self.kill_adapter();
        self.state.clear();
        self.state.output_lines.clear();
        self.exit_code = None;
        self.session_end = None;
        while self.event_rx.try_recv().is_ok() {}
        let client = DebugAdapterClient::spawn(command, args, self.event_tx.clone()).await?;
        self.client = Some(client);
        self.state.session_active = true;
        Ok(())
    }

    /// Kills the adapter process (if any) without waiting for it.
    fn kill_adapter(&mut self) {
        if let Some(client) = self.client.take() {
            client.kill();
        }
    }

    /// Asks the event loop to end the session (and any queued start).
    pub fn request_stop(&mut self) {
        self.stop_requested = true;
        if matches!(self.pending_action, Some(PendingDebugAction::Start { .. })) {
            self.pending_action = None;
        }
    }

    /// Asks the event loop to re-send breakpoints and exception filters.
    pub fn request_breakpoint_sync(&mut self) {
        self.breakpoint_sync_requested = true;
    }

    /// True once after [`request_breakpoint_sync`](Self::request_breakpoint_sync).
    pub fn take_breakpoint_sync_request(&mut self) -> bool {
        std::mem::take(&mut self.breakpoint_sync_requested)
    }

    /// True once after [`request_stop`](Self::request_stop).
    pub fn take_stop_request(&mut self) -> bool {
        std::mem::take(&mut self.stop_requested)
    }

    /// Drains debuggee/adapter output that has not been shown in the run
    /// console yet, as `(DAP category, text)`.
    pub fn take_console_output(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.console_output)
    }

    /// Adds a line to the debug console (shown in the run console).
    pub fn log_console(&mut self, text: String) {
        self.state.output_lines.push(format!("[console] {text}"));
        self.console_output
            .push(("console".to_string(), format!("{text}\n")));
    }

    /// Returns (once) how the last session ended.
    pub fn take_session_end(&mut self) -> Option<SessionEnd> {
        self.session_end.take()
    }

    /// Initialize the debug adapter (send initialize request).
    pub async fn initialize(&mut self) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        let caps = client.initialize().await?;
        self.adopt_exception_filters(&caps.exception_breakpoint_filters);
        self.capabilities = Some(caps);
        Ok(())
    }

    /// Remembers the adapter's exception filters, keeping the user's earlier
    /// choice for filters it already knew.
    fn adopt_exception_filters(&mut self, offered: &[DapExceptionFilter]) {
        let previous = std::mem::take(&mut self.state.exception_filters);
        self.state.exception_filters = offered
            .iter()
            .map(|f| state::ExceptionFilter {
                enabled: previous
                    .iter()
                    .find(|p| p.id == f.filter)
                    .map(|p| p.enabled)
                    .unwrap_or(f.default.unwrap_or(false)),
                id: f.filter.clone(),
                label: f.label.clone(),
            })
            .collect();
    }

    /// Whether the running adapter supports logpoints (`None` before it has
    /// answered `initialize`).
    pub fn supports_log_points(&self) -> Option<bool> {
        self.capabilities.as_ref().map(|c| c.supports_log_points)
    }

    /// Whether the running adapter supports hit-count conditions.
    pub fn supports_hit_conditions(&self) -> Option<bool> {
        self.capabilities
            .as_ref()
            .map(|c| c.supports_hit_conditional_breakpoints)
    }

    /// Sends the enabled exception filters (`setExceptionBreakpoints`). A
    /// no-op when the adapter offered none.
    pub async fn sync_exception_breakpoints(&self) -> Result<()> {
        if self.state.exception_filters.is_empty() {
            return Ok(());
        }
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        let enabled: Vec<String> = self
            .state
            .exception_filters
            .iter()
            .filter(|f| f.enabled)
            .map(|f| f.id.clone())
            .collect();
        client.set_exception_breakpoints(&enabled).await
    }

    /// Attach to a running debuggee.
    pub async fn attach(&self, config: serde_json::Value) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        client.attach(config).await?;
        Ok(())
    }

    /// Send configurationDone after setting breakpoints.
    pub async fn configuration_done(&self) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        client.configuration_done().await?;
        Ok(())
    }

    /// Sends the enabled breakpoints of a source file.
    pub async fn set_breakpoints(&mut self, path: &Path) -> Result<Vec<DapBreakpoint>> {
        if self.client.is_none() {
            anyhow::bail!("no debug adapter running");
        }

        let source = DapSource {
            name: path.file_name().map(|n| n.to_string_lossy().to_string()),
            path: Some(path.to_string_lossy().to_string()),
        };

        // Conditions, logpoints and hit counts ride along. An adapter that
        // cannot honour a logpoint / hit count must not get the bare line
        // (it would stop on every hit), so those are left out and said so.
        let log_ok = self.supports_log_points() != Some(false);
        let hit_ok = self.supports_hit_conditions() != Some(false);
        let mut skipped = Vec::new();
        let mut sent = Vec::new();
        let mut source_bps: Vec<DapSourceBreakpoint> = Vec::new();
        for bp in self.state.enabled_breakpoints(path) {
            if (bp.log_message.is_some() && !log_ok) || (bp.hit_condition.is_some() && !hit_ok) {
                skipped.push(bp.line);
                continue;
            }
            sent.push(bp.line);
            source_bps.push(DapSourceBreakpoint {
                line: bp.line,
                condition: bp.condition,
                hit_condition: bp.hit_condition,
                log_message: bp.log_message,
            });
        }
        if !skipped.is_empty() {
            let file = path.file_name().map(|n| n.to_string_lossy().into_owned());
            self.log_console(format!(
                "The debug adapter does not support logpoints / hit counts: not set at {}:{}",
                file.unwrap_or_default(),
                skipped
                    .iter()
                    .map(|l| l.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        let result = client.set_breakpoints(&source, &source_bps).await?;

        // The ones we did not send stay, unverified.
        self.state.update_breakpoints(path, &sent, &result);
        self.state.mark_breakpoints_unverified(path, &skipped);

        Ok(result)
    }

    /// Continue execution.
    pub async fn continue_(&self, thread_id: u64) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        client.continue_(thread_id).await?;
        Ok(())
    }

    /// Step over.
    pub async fn next(&self, thread_id: u64) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        client.next(thread_id).await?;
        Ok(())
    }

    /// Step into.
    pub async fn step_in(&self, thread_id: u64) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        client.step_in(thread_id).await?;
        Ok(())
    }

    /// Step out.
    pub async fn step_out(&self, thread_id: u64) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        client.step_out(thread_id).await?;
        Ok(())
    }

    /// The debuggee's threads.
    pub async fn threads(&self) -> Result<Vec<DapThread>> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        client.threads().await
    }

    /// What the stopped thread threw.
    pub async fn exception_info(&self, thread_id: u64) -> Result<DapExceptionInfo> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        client.exception_info(thread_id).await
    }

    /// Get stack trace for a thread.
    pub async fn stack_trace(&self, thread_id: u64) -> Result<Vec<DapStackFrame>> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        client.stack_trace(thread_id).await
    }

    /// Get scopes for a frame.
    pub async fn scopes(&self, frame_id: u64) -> Result<Vec<DapScope>> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        client.scopes(frame_id).await
    }

    /// Get variables for a scope/reference.
    pub async fn variables(&self, variables_reference: u64) -> Result<Vec<DapVariable>> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        client.variables(variables_reference).await
    }

    /// Evaluate an expression in the context of a frame.
    pub async fn evaluate(
        &self,
        expression: &str,
        frame_id: Option<u64>,
        context: Option<&str>,
    ) -> Result<(String, Option<String>, u64)> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        client.evaluate(expression, frame_id, context).await
    }

    /// Disconnect from the debug adapter: ask it to terminate the debuggee,
    /// then make sure the adapter process itself is gone. The session's
    /// output stays available; only live state (frames, variables,
    /// execution marker) is cleared.
    pub async fn disconnect(&mut self) -> Result<()> {
        let had_session = self.client.is_some() || self.state.session_active;
        let result = match self.client.take() {
            Some(client) => {
                let result = client.disconnect(self.terminate_debuggee).await;
                client.kill();
                result
            }
            None => Ok(()),
        };
        self.attach_request = None;
        self.state.clear();
        if had_session && self.session_end.is_none() {
            self.session_end = Some(SessionEnd {
                exit_code: self.exit_code,
                adapter_crash: None,
            });
        }
        result
    }

    /// The debuggee is gone (`exited`/`terminated`/adapter EOF). Output is
    /// kept (it is the only record of why the program ended); everything tied
    /// to a live debuggee goes, and the adapter process must not linger.
    fn end_session(&mut self) {
        if !(self.state.session_active || self.client.is_some()) {
            return;
        }
        self.state.end_session_keep_output();
        self.kill_adapter();
        self.attach_request = None;
        if self.session_end.is_none() {
            self.session_end = Some(SessionEnd {
                exit_code: self.exit_code,
                adapter_crash: None,
            });
        }
    }

    /// Poll for events from the debug adapter. Returns the number of events processed.
    pub fn process_events(&mut self) -> usize {
        let mut count = 0;
        while let Ok(event) = self.event_rx.try_recv() {
            match &event {
                DapEvent::Stopped {
                    reason,
                    thread_id,
                    all_threads_stopped: _,
                    description,
                } => {
                    self.state.stopped_thread = *thread_id;
                    self.state.event_thread = *thread_id;
                    self.state.stop_reason = Some(reason.clone());
                    self.state.exception = (reason == "exception")
                        .then(|| description.clone())
                        .flatten();
                    self.state.is_running = false;
                    self.state.panels_visible = true;
                }
                DapEvent::Continued { thread_id: _ } => {
                    self.state.is_running = true;
                    self.state.stopped_thread = None;
                    self.state.event_thread = None;
                    self.state.threads.clear();
                    self.state.stop_reason = None;
                    self.state.exception = None;
                    // Clear stale frame/variable data.
                    self.state.stack_frames.clear();
                    self.state.scopes.clear();
                    self.state.variables.clear();
                    self.state.clear_watch_values();
                }
                DapEvent::Thread {
                    reason: _,
                    thread_id: _,
                } => {
                    // Thread lifecycle — we can track this later.
                }
                DapEvent::Output { category, output } => {
                    self.console_output.push((category.clone(), output.clone()));
                    self.state
                        .output_lines
                        .push(format!("[{category}] {output}"));
                    // Cap output buffer.
                    if self.state.output_lines.len() > 10_000 {
                        let drain_count = self.state.output_lines.len() - 5_000;
                        self.state.output_lines.drain(..drain_count);
                    }
                }
                DapEvent::Exited { exit_code } => {
                    self.exit_code = *exit_code;
                    self.end_session();
                }
                DapEvent::Terminated => self.end_session(),
                DapEvent::AdapterExited { detail } => {
                    if self.state.session_active || self.client.is_some() {
                        self.end_session();
                        if let Some(end) = self.session_end.as_mut() {
                            end.adapter_crash = Some(detail.clone());
                        }
                        self.console_output.push((
                            "console".to_string(),
                            format!("Debug adapter crashed ({detail})\n"),
                        ));
                    }
                }
                DapEvent::Initialized => {
                    // The adapter is ready: attach first, then breakpoints
                    // and configurationDone.
                    if self.attach_request.is_some() {
                        self.pending_action = Some(PendingDebugAction::Attach);
                    }
                }
            }
            count += 1;
        }
        count
    }

    /// Whether a debug session is active.
    pub fn is_active(&self) -> bool {
        self.state.session_active
    }

    /// Get the client (if connected).
    pub fn client(&self) -> Option<&DebugAdapterClient> {
        self.client.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stop_request_survives_other_actions_overwriting_the_pending_slot() {
        let mut dap = DapManager::new();
        dap.request_stop();
        // A `stopped` event queues a state fetch in the same tick.
        dap.pending_action = Some(PendingDebugAction::FetchState);
        assert!(dap.take_stop_request());
        assert!(!dap.take_stop_request(), "reported once");
    }

    #[test]
    fn a_breakpoint_sync_request_survives_the_pending_slot_being_overwritten() {
        let mut dap = DapManager::new();
        dap.request_breakpoint_sync();
        // A session that keeps stopping queues a state fetch every tick.
        dap.pending_action = Some(PendingDebugAction::FetchState);
        assert!(dap.take_breakpoint_sync_request());
        assert!(!dap.take_breakpoint_sync_request(), "reported once");
    }

    #[test]
    fn stopping_cancels_a_start_that_has_not_run_yet() {
        let mut dap = DapManager::new();
        dap.pending_action = Some(PendingDebugAction::Start {
            command: "x".into(),
            args: vec![],
            attach: serde_json::json!({}),
            terminate_debuggee: true,
        });
        dap.request_stop();
        assert!(dap.pending_action.is_none());
    }
}
