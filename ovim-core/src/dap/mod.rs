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
//! - `DapManager`: the editor's side of a single debug session: debug state,
//!   the queue of actions to run, and the reports coming back. It never waits
//!   for the adapter.
//! - `session`: the task that owns the adapter. The tick sends it commands and
//!   polls what it reports; nothing the adapter does can stall the editor.
//! - `DebugAdapterClient`: Individual debug adapter process management
//! - `protocol`: DAP message handling (Content-Length framing, same as LSP)
//! - `state`: Debug state (breakpoints, stack frames, variables)
//! - `types`: Client-side DAP type definitions

pub mod client;
pub mod follow;
pub mod panel;
pub mod protocol;
pub mod session;
pub mod state;
pub mod types;

use std::collections::VecDeque;
use std::path::PathBuf;
use tokio::sync::mpsc;

use session::{
    BatchOutcome, BreakpointBatch, DapCommand, DapReport, DapResult, ResumeKind, SessionHandle,
    StateFetch, Tagged,
};
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

/// A debug action queued for the next tick. The tick turns it into a command
/// for the session task (see [`DapManager::dispatch`]); none of them waits for
/// the adapter.
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
    /// Configure the session once the adapter is initialized: send `attach`
    /// (from `attach_request`), the breakpoints and `configurationDone`.
    Attach,
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
    /// The live session: the task that owns the adapter.
    session: Option<SessionHandle>,
    /// Number of the current session. Reports of any other are stale.
    generation: u64,
    /// Reports from the session tasks, tagged with their generation.
    reports_rx: mpsc::Receiver<Tagged>,
    /// Sender side (cloned into each session task).
    reports_tx: mpsc::Sender<Tagged>,
    /// Debug state (breakpoints, frames, variables).
    pub state: DebugState,
    /// Actions waiting for the next tick, in the order they were asked for.
    pending: VecDeque<PendingDebugAction>,
    /// Answers from the session task that the editor has not applied yet.
    results: Vec<DapResult>,
    /// Which view of the debuggee the answers in flight are for. Anything
    /// that changes what is being inspected (a stop, a resume, another
    /// thread, the end of the session) bumps it, and answers for an older
    /// view are dropped.
    epoch: u64,
    /// Stop was requested. Kept apart from the queue so it outranks
    /// everything queued before it.
    stop_requested: bool,
    /// Breakpoints or exception filters changed and the live session has to
    /// hear about it.
    breakpoint_sync_requested: bool,
    /// The session has been sent its configuration (`attach`, breakpoints,
    /// `configurationDone`); later breakpoint changes go as syncs.
    configured: bool,
    /// The DAP `attach` arguments for the session being started.
    pub attach_request: Option<serde_json::Value>,
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
        let (reports_tx, reports_rx) = mpsc::channel(1024);
        Self {
            session: None,
            generation: 0,
            reports_rx,
            reports_tx,
            state: DebugState::new(),
            pending: VecDeque::new(),
            results: Vec::new(),
            epoch: 0,
            stop_requested: false,
            breakpoint_sync_requested: false,
            configured: false,
            attach_request: None,
            console_output: Vec::new(),
            session_end: None,
            exit_code: None,
            capabilities: None,
        }
    }

    // ---- Queue ----

    /// Queues an action for the next tick. An action that makes a queued one
    /// redundant replaces it; steps and evaluations are never merged (each
    /// is something the user asked for).
    pub fn queue(&mut self, action: PendingDebugAction) {
        use PendingDebugAction as A;
        match &action {
            // A new session replaces whatever was meant for the old one.
            A::Start { .. } => self.pending.clear(),
            A::FetchState => self.pending.retain(|a| {
                !matches!(a, A::FetchState | A::SelectFrame { .. } | A::RefreshWatches)
            }),
            A::SelectFrame { .. } => self
                .pending
                .retain(|a| !matches!(a, A::SelectFrame { .. } | A::RefreshWatches)),
            A::RefreshWatches => self.pending.retain(|a| !matches!(a, A::RefreshWatches)),
            A::FetchVariables { var_ref } => self.pending.retain(
                |a| !matches!(a, A::FetchVariables { var_ref: queued } if queued == var_ref),
            ),
            _ => {}
        }
        self.pending.push_back(action);
    }

    /// The actions waiting for the next tick.
    pub fn queued(&self) -> impl Iterator<Item = &PendingDebugAction> {
        self.pending.iter()
    }

    /// Asks the event loop to end the session (and any queued start).
    pub fn request_stop(&mut self) {
        self.stop_requested = true;
        let queued = self.pending.len();
        self.pending
            .retain(|a| !matches!(a, PendingDebugAction::Start { .. }));
        if self.pending.len() != queued {
            self.attach_request = None;
            // No adapter will ever start and say how the session went, but
            // the launch that queued it waits for exactly that.
            self.session_end.get_or_insert(SessionEnd {
                exit_code: None,
                adapter_crash: None,
            });
        }
    }

    /// Asks the event loop to re-send breakpoints and exception filters.
    pub fn request_breakpoint_sync(&mut self) {
        self.breakpoint_sync_requested = true;
    }

    /// Sends what the tick has been asked to do to the session task. Stop
    /// comes first, then a breakpoint sync, then the queue in order. Never
    /// waits for the adapter. Returns what to tell the user about actions that
    /// could not be sent.
    pub fn dispatch(&mut self) -> Vec<String> {
        if std::mem::take(&mut self.stop_requested) {
            self.begin_stop();
            return Vec::new();
        }
        if self.configured
            && self.session.is_some()
            && std::mem::take(&mut self.breakpoint_sync_requested)
        {
            let batches = self.breakpoint_batches();
            let exception_filters = self.enabled_exception_filters();
            self.send(DapCommand::SyncBreakpoints {
                batches,
                exception_filters,
            })
            .ok();
        }
        let mut notices = Vec::new();
        while let Some(action) = self.pending.pop_front() {
            notices.extend(self.dispatch_one(action));
        }
        notices
    }

    fn dispatch_one(&mut self, action: PendingDebugAction) -> Option<String> {
        use PendingDebugAction as A;
        match action {
            A::Start {
                command,
                args,
                attach,
                terminate_debuggee,
            } => {
                self.begin_session(command, args, attach, terminate_debuggee);
                None
            }
            A::Continue => self.resume(ResumeKind::Continue),
            A::StepOver => self.resume(ResumeKind::StepOver),
            A::StepIn => self.resume(ResumeKind::StepIn),
            A::StepOut => self.resume(ResumeKind::StepOut),
            A::Attach => {
                self.configure();
                None
            }
            A::FetchState => {
                self.fetch_state();
                None
            }
            A::SelectFrame { .. } => {
                if let Some(frame_id) = self.selected_frame_id() {
                    self.send(DapCommand::SelectFrame {
                        epoch: self.epoch,
                        frame_id,
                        watches: self.watch_expressions(),
                    })
                    .ok();
                }
                None
            }
            A::RefreshWatches => {
                self.send(DapCommand::RefreshWatches {
                    epoch: self.epoch,
                    frame_id: self.selected_frame_id(),
                    watches: self.watch_expressions(),
                })
                .ok();
                None
            }
            A::FetchVariables { var_ref } => {
                self.send(DapCommand::FetchVariables {
                    epoch: self.epoch,
                    reference: var_ref,
                })
                .ok();
                None
            }
            A::Evaluate { expression } => {
                let frame_id = self.selected_frame_id();
                self.send(DapCommand::Evaluate {
                    expression,
                    frame_id,
                })
                .err()
                .map(|e| format!("Eval error: {e}"))
            }
            A::EvaluateHover { expression } => {
                let frame_id = self.selected_frame_id();
                self.send(DapCommand::Hover {
                    expression: expression.clone(),
                    frame_id,
                })
                .err()
                .map(|e| format!("{expression}: {e}"))
            }
        }
    }

    fn send(&self, command: DapCommand) -> Result<(), &'static str> {
        match &self.session {
            Some(session) => {
                session.send(command);
                Ok(())
            }
            None => Err("no debug adapter running"),
        }
    }

    fn selected_frame_id(&self) -> Option<u64> {
        self.state
            .stack_frames
            .get(self.state.selected_frame)
            .map(|f| f.id)
    }

    fn watch_expressions(&self) -> Vec<String> {
        self.state
            .watches
            .iter()
            .map(|w| w.expression.clone())
            .collect()
    }

    // ---- Session lifecycle ----

    /// Starts a session: the task spawns the adapter and initializes it; the
    /// editor hears about it through the reports.
    fn begin_session(
        &mut self,
        command: String,
        args: Vec<String>,
        attach: serde_json::Value,
        terminate_debuggee: bool,
    ) {
        // A previous session may have left an adapter behind; never leak it.
        self.session = None;
        self.generation += 1;
        self.epoch += 1;
        self.results.clear();
        self.state.clear();
        self.state.output_lines.clear();
        self.exit_code = None;
        self.session_end = None;
        self.capabilities = None;
        self.configured = false;
        self.breakpoint_sync_requested = false;
        self.attach_request = Some(attach);
        while self.reports_rx.try_recv().is_ok() {}
        if tokio::runtime::Handle::try_current().is_err() {
            self.results
                .push(DapResult::StartFailed("no async runtime available".into()));
            return;
        }
        self.session = Some(SessionHandle::spawn(
            self.generation,
            command,
            args,
            terminate_debuggee,
            self.reports_tx.clone(),
        ));
        self.state.session_active = true;
    }

    /// Ends the session gracefully (the task disconnects, then kills the
    /// adapter), or right away when there is no adapter to talk to.
    fn begin_stop(&mut self) {
        self.pending.clear();
        match self.session.as_mut() {
            Some(session) => session.disconnect(),
            None => self.finish_session(),
        }
    }

    /// Tears the session down without a goodbye (adapter killed, state
    /// reset) and drops whatever it still reports. For a session that could
    /// not be started or configured.
    pub fn abort_session(&mut self) {
        self.session = None;
        self.generation += 1;
        self.epoch += 1;
        self.pending.clear();
        self.results.clear();
        self.attach_request = None;
        self.configured = false;
        self.state.end_session_keep_output();
    }

    /// The session is over. Output stays (it is the only record of why a
    /// program ended); everything tied to a live debuggee goes, and the
    /// adapter process must not linger.
    fn finish_session(&mut self) {
        let had_session = self.session.is_some() || self.state.session_active;
        self.session = None;
        self.pending.clear();
        self.attach_request = None;
        self.configured = false;
        self.epoch += 1;
        self.state.clear();
        if had_session && self.session_end.is_none() {
            self.session_end = Some(SessionEnd {
                exit_code: self.exit_code,
                adapter_crash: None,
            });
        }
    }

    /// The debuggee is gone (`exited`/`terminated`/adapter EOF).
    fn end_session(&mut self) {
        if self.state.session_active || self.session.is_some() {
            self.finish_session();
        }
    }

    // ---- Commands ----

    fn resume(&mut self, kind: ResumeKind) -> Option<String> {
        let thread_id = self.state.stopped_thread.unwrap_or(1);
        // Whatever is still being loaded for the current stop is moot.
        self.epoch += 1;
        let sent = self.send(DapCommand::Resume {
            kind,
            thread_id,
            epoch: self.epoch,
        });
        match sent {
            Ok(()) => {
                if kind == ResumeKind::Continue {
                    self.state.is_running = true;
                }
                None
            }
            Err(e) => Some(format!("{}: {e}", kind.failure_text())),
        }
    }

    /// `attach`, then the breakpoints, then `configurationDone`.
    fn configure(&mut self) {
        let Some(attach) = self.attach_request.clone() else {
            // Nothing was asked for; a stray `initialized` event. Do not send
            // `configurationDone` to an adapter that has no debuggee.
            return;
        };
        let batches = self.breakpoint_batches();
        let exception_filters = self.enabled_exception_filters();
        self.configured = true;
        // The configuration carries every breakpoint as it is now.
        self.breakpoint_sync_requested = false;
        self.send(DapCommand::Configure {
            attach,
            batches,
            exception_filters,
        })
        .ok();
    }

    /// Loads the stack, threads and variables of the stop being shown.
    fn fetch_state(&mut self) {
        let state = &self.state;
        let thread_id = state.stopped_thread.unwrap_or(1);
        let exception_thread = (state.stop_reason.as_deref() == Some("exception"))
            .then(|| state.event_thread.unwrap_or(1))
            .filter(|thread| state.stopped_thread.unwrap_or(*thread) == *thread);
        let fetch = StateFetch {
            epoch: self.epoch,
            thread_id,
            exception_thread,
            expanded: state.expanded_refs.iter().copied().collect(),
            watches: self.watch_expressions(),
        };
        self.send(DapCommand::FetchState(fetch)).ok();
    }

    /// Every file's enabled breakpoints as `setBreakpoints` requests.
    ///
    /// Conditions, logpoints and hit counts ride along. An adapter that
    /// cannot honour a logpoint / hit count must not get the bare line (it
    /// would stop on every hit), so those are left out and said so.
    fn breakpoint_batches(&mut self) -> Vec<BreakpointBatch> {
        let log_ok = self.supports_log_points() != Some(false);
        let hit_ok = self.supports_hit_conditions() != Some(false);
        let mut paths: Vec<PathBuf> = self.state.breakpoints.keys().cloned().collect();
        paths.sort();
        let mut batches = Vec::new();
        for path in paths {
            let mut batch = BreakpointBatch {
                path,
                sent: Vec::new(),
                skipped: Vec::new(),
                breakpoints: Vec::new(),
            };
            for bp in self.state.enabled_breakpoints(&batch.path) {
                if (bp.log_message.is_some() && !log_ok) || (bp.hit_condition.is_some() && !hit_ok)
                {
                    batch.skipped.push(bp.line);
                    continue;
                }
                batch.sent.push(bp.line);
                batch.breakpoints.push(DapSourceBreakpoint {
                    line: bp.line,
                    condition: bp.condition,
                    hit_condition: bp.hit_condition,
                    log_message: bp.log_message,
                });
            }
            if !batch.skipped.is_empty() {
                let file = batch
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned());
                self.log_console(format!(
                    "The debug adapter does not support logpoints / hit counts: not set at {}:{}",
                    file.unwrap_or_default(),
                    batch
                        .skipped
                        .iter()
                        .map(|l| l.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            batches.push(batch);
        }
        batches
    }

    /// The ids of the enabled exception filters; `None` when the adapter
    /// offered none.
    fn enabled_exception_filters(&self) -> Option<Vec<String>> {
        (!self.state.exception_filters.is_empty()).then(|| {
            self.state
                .exception_filters
                .iter()
                .filter(|f| f.enabled)
                .map(|f| f.id.clone())
                .collect()
        })
    }

    /// Takes in the adapter's answers to `setBreakpoints`.
    pub fn apply_breakpoint_outcomes(&mut self, outcomes: Vec<BatchOutcome>) {
        for BatchOutcome { batch, reply } in outcomes {
            if let Ok(reply) = reply {
                self.state
                    .update_breakpoints(&batch.path, &batch.sent, &reply);
            }
            // The ones we did not send stay, unverified.
            self.state
                .mark_breakpoints_unverified(&batch.path, &batch.skipped);
        }
    }

    /// Makes answers still in flight for what is shown now stale (another
    /// thread is about to be inspected).
    pub fn invalidate_inspection(&mut self) {
        self.epoch += 1;
    }

    /// Whether an answer tagged `epoch` is still about what is being shown.
    pub fn is_current(&self, epoch: u64) -> bool {
        epoch == self.epoch
    }

    // ---- Reports ----

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

    /// The answers from the session task that the editor has yet to apply.
    pub fn take_results(&mut self) -> Vec<DapResult> {
        std::mem::take(&mut self.results)
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

    /// Takes in what the session task has reported since the last call:
    /// adapter events update the debug state (and queue what they call for),
    /// answers wait in [`take_results`](Self::take_results). Reports of a
    /// session that has since been replaced are dropped. Returns the number
    /// of reports processed.
    pub fn process_events(&mut self) -> usize {
        // Read before draining: whatever a finished task sent has arrived.
        let task_gone = self
            .session
            .as_ref()
            .is_some_and(SessionHandle::task_finished);
        let mut count = 0;
        while let Ok(Tagged { generation, report }) = self.reports_rx.try_recv() {
            if generation != self.generation {
                continue;
            }
            count += 1;
            match report {
                DapReport::Event(event) => self.handle_event(event),
                DapReport::Started(Ok(capabilities)) => {
                    self.adopt_exception_filters(&capabilities.exception_breakpoint_filters);
                    self.capabilities = Some(capabilities);
                }
                DapReport::Started(Err(message)) => {
                    self.abort_session();
                    self.results.push(DapResult::StartFailed(message));
                }
                DapReport::Done(result) => self.results.push(result),
                DapReport::Disconnected => self.finish_session(),
            }
        }
        if task_gone && self.session.is_some() {
            // The task died without a word (it panicked): nobody is left to
            // drive the adapter.
            self.end_session();
            if let Some(end) = self.session_end.as_mut() {
                end.adapter_crash = Some("the debug session task stopped unexpectedly".into());
            }
            count += 1;
        }
        count
    }

    fn handle_event(&mut self, event: DapEvent) {
        match event {
            DapEvent::Stopped {
                reason,
                thread_id,
                all_threads_stopped: _,
                description,
            } => {
                self.state.stopped_thread = thread_id;
                self.state.event_thread = thread_id;
                self.state.exception = (reason == "exception").then_some(description).flatten();
                self.state.stop_reason = Some(reason);
                self.state.is_running = false;
                self.state.panels_visible = true;
                self.epoch += 1;
                self.queue(PendingDebugAction::FetchState);
            }
            DapEvent::Continued { thread_id: _ } => {
                self.epoch += 1;
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
                self.state
                    .output_lines
                    .push(format!("[{category}] {output}"));
                self.console_output.push((category, output));
                // Cap output buffer.
                if self.state.output_lines.len() > 10_000 {
                    let drain_count = self.state.output_lines.len() - 5_000;
                    self.state.output_lines.drain(..drain_count);
                }
            }
            DapEvent::Exited { exit_code } => {
                self.exit_code = exit_code;
                self.end_session();
            }
            DapEvent::Terminated => self.end_session(),
            DapEvent::AdapterExited { detail } => {
                if self.state.session_active || self.session.is_some() {
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
                    self.queue(PendingDebugAction::Attach);
                }
            }
        }
    }

    /// Whether a debug session is active.
    pub fn is_active(&self) -> bool {
        self.state.session_active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start() -> PendingDebugAction {
        PendingDebugAction::Start {
            command: "x".into(),
            args: vec![],
            attach: serde_json::json!({}),
            terminate_debuggee: true,
        }
    }

    fn report(manager: &DapManager, generation: u64, event: DapEvent) {
        manager
            .reports_tx
            .try_send(Tagged {
                generation,
                report: DapReport::Event(event),
            })
            .unwrap();
    }

    fn stopped() -> DapEvent {
        DapEvent::Stopped {
            reason: "breakpoint".into(),
            thread_id: Some(1),
            all_threads_stopped: true,
            description: None,
        }
    }

    fn queued(manager: &DapManager) -> Vec<String> {
        manager
            .queued()
            .map(|action| {
                format!("{action:?}")
                    .split([' ', '{'])
                    .next()
                    .unwrap()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn a_stop_event_does_not_overwrite_a_step_queued_in_the_same_tick() {
        let mut dap = DapManager::new();
        dap.queue(PendingDebugAction::StepOver);
        report(&dap, 0, stopped());

        dap.process_events();

        assert_eq!(queued(&dap), ["StepOver", "FetchState"]);
    }

    #[test]
    fn refreshes_replace_each_other_but_steps_are_never_merged() {
        let mut dap = DapManager::new();
        dap.queue(PendingDebugAction::StepOver);
        dap.queue(PendingDebugAction::StepOver);
        dap.queue(PendingDebugAction::RefreshWatches);
        dap.queue(PendingDebugAction::SelectFrame { index: 1 });
        dap.queue(PendingDebugAction::FetchState);
        dap.queue(PendingDebugAction::FetchState);
        assert_eq!(queued(&dap), ["StepOver", "StepOver", "FetchState"]);
    }

    #[test]
    fn a_new_session_replaces_what_was_queued_for_the_old_one() {
        let mut dap = DapManager::new();
        dap.queue(PendingDebugAction::Continue);
        dap.queue(start());
        assert_eq!(queued(&dap), ["Start"]);
    }

    #[test]
    fn actions_without_a_session_are_reported_instead_of_silently_dropped() {
        let mut dap = DapManager::new();
        dap.queue(PendingDebugAction::StepIn);
        dap.queue(PendingDebugAction::Evaluate {
            expression: "x".into(),
        });
        let notices = dap.dispatch();
        assert_eq!(
            notices,
            [
                "Debug step in failed: no debug adapter running",
                "Eval error: no debug adapter running"
            ]
        );
        assert_eq!(dap.queued().count(), 0);
    }

    #[test]
    fn stop_outranks_everything_queued_and_a_breakpoint_sync_is_not_lost() {
        let mut dap = DapManager::new();
        dap.queue(PendingDebugAction::Continue);
        dap.request_stop();
        dap.request_breakpoint_sync();

        assert!(dap.dispatch().is_empty(), "the queued step is dropped");

        assert_eq!(dap.queued().count(), 0);
        assert!(
            dap.breakpoint_sync_requested,
            "a sync waits for a configured session instead of vanishing"
        );
    }

    #[test]
    fn stopping_cancels_a_start_that_has_not_run_yet_and_reports_the_session_over() {
        let mut dap = DapManager::new();
        dap.queue(start());
        dap.request_stop();
        assert_eq!(dap.queued().count(), 0);
        assert_eq!(
            dap.take_session_end(),
            Some(SessionEnd {
                exit_code: None,
                adapter_crash: None
            }),
            "the launch waiting for the debugger has to hear that it will not come"
        );
    }

    #[test]
    fn reports_of_a_replaced_session_are_dropped() {
        let mut dap = DapManager::new();
        dap.generation = 2;
        report(
            &dap,
            1,
            DapEvent::Output {
                category: "stdout".into(),
                output: "old adapter".into(),
            },
        );
        report(&dap, 1, stopped());
        report(
            &dap,
            2,
            DapEvent::Output {
                category: "stdout".into(),
                output: "new adapter".into(),
            },
        );

        assert_eq!(dap.process_events(), 1);

        assert_eq!(
            dap.take_console_output(),
            [("stdout".to_string(), "new adapter".to_string())]
        );
        assert_eq!(dap.queued().count(), 0, "the old stop fetches nothing");
        assert_eq!(dap.state.stopped_thread, None);
    }

    #[test]
    fn answers_about_an_earlier_stop_are_stale() {
        let mut dap = DapManager::new();
        report(&dap, 0, stopped());
        dap.process_events();
        let epoch = dap.epoch;
        assert!(dap.is_current(epoch));

        report(&dap, 0, DapEvent::Continued { thread_id: 1 });
        dap.process_events();

        assert!(!dap.is_current(epoch));
    }
}
