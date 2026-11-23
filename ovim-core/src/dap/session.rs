//! The task that talks to a debug adapter.
//!
//! An adapter answers when it likes, and a hung one only after the client's
//! 30 second timeout, so no request runs on the editor's tick. The tick hands
//! a [`DapCommand`] to a [`SessionHandle`], carries on, and finds the answer
//! as a [`DapResult`] the next time it polls. The task runs one command at a
//! time (the order they were sent in), but Stop preempts whatever it is
//! waiting for.
//!
//! Everything the task reports is tagged with the generation of its session,
//! so late output of an adapter that has since been replaced is recognised and
//! dropped instead of leaking into the new session.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::{mpsc, Notify};
use tokio::task::JoinHandle;

use super::client::{AdapterKiller, DebugAdapterClient};
use super::types::*;
use super::DapEvent;

/// The breakpoints of one file, as they were when the request was built.
#[derive(Debug, Clone)]
pub struct BreakpointBatch {
    pub path: PathBuf,
    /// The requested line of each breakpoint sent, in the order sent: the
    /// adapter answers in that order.
    pub sent: Vec<u64>,
    /// Requested lines left out because the adapter cannot honour what they
    /// carry (a logpoint or hit count).
    pub skipped: Vec<u64>,
    pub breakpoints: Vec<DapSourceBreakpoint>,
}

/// A [`BreakpointBatch`] and the adapter's answer to it.
#[derive(Debug)]
pub struct BatchOutcome {
    pub batch: BreakpointBatch,
    pub reply: Result<Vec<DapBreakpoint>, String>,
}

/// What stepping and continuing ask the adapter for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeKind {
    Continue,
    StepOver,
    StepIn,
    StepOut,
}

impl ResumeKind {
    /// How a failed request is described to the user.
    pub fn failure_text(self) -> &'static str {
        match self {
            Self::Continue => "Debug continue failed",
            Self::StepOver => "Debug step failed",
            Self::StepIn => "Debug step in failed",
            Self::StepOut => "Debug step out failed",
        }
    }
}

/// What to load when the debuggee stops: stack, exception, threads, the top
/// frame's variables and the watches.
#[derive(Debug, Clone)]
pub struct StateFetch {
    pub epoch: u64,
    pub thread_id: u64,
    /// The thread to ask `exceptionInfo` about (the stop was an exception).
    pub exception_thread: Option<u64>,
    /// Variable references the user has expanded.
    pub expanded: Vec<u64>,
    pub watches: Vec<String>,
}

/// A request for the session task. Built on the tick from the editor's state
/// at that moment; `epoch`s say which view of the debuggee the answer is for.
#[derive(Debug)]
pub enum DapCommand {
    /// `attach`, every file's breakpoints, the exception filters and
    /// `configurationDone`.
    Configure {
        attach: Value,
        batches: Vec<BreakpointBatch>,
        exception_filters: Option<Vec<String>>,
    },
    SyncBreakpoints {
        batches: Vec<BreakpointBatch>,
        exception_filters: Option<Vec<String>>,
    },
    Resume {
        kind: ResumeKind,
        thread_id: u64,
        epoch: u64,
    },
    FetchState(StateFetch),
    SelectFrame {
        epoch: u64,
        frame_id: u64,
        watches: Vec<String>,
    },
    RefreshWatches {
        epoch: u64,
        frame_id: Option<u64>,
        watches: Vec<String>,
    },
    FetchVariables {
        epoch: u64,
        reference: u64,
    },
    Evaluate {
        expression: String,
        frame_id: Option<u64>,
    },
    /// Evaluate for the hover popup, with the value's children.
    Hover {
        expression: String,
        frame_id: Option<u64>,
    },
}

/// The answer to a [`DapCommand`], or part of it: loading a stop reports each
/// piece as it arrives so the editor can show the stack before the variables.
#[derive(Debug)]
pub enum DapResult {
    /// The adapter could not be started or initialized.
    StartFailed(String),
    Configured {
        batches: Vec<BatchOutcome>,
        /// What went wrong, if the session could not be configured.
        failure: Option<String>,
    },
    BreakpointsSynced {
        batches: Vec<BatchOutcome>,
    },
    Resumed {
        kind: ResumeKind,
        epoch: u64,
        error: Option<String>,
    },
    Frames {
        epoch: u64,
        frames: Vec<DapStackFrame>,
    },
    Exception {
        epoch: u64,
        /// `Type: message`; `None` when the adapter would not say.
        summary: Option<String>,
    },
    Threads {
        epoch: u64,
        threads: Vec<DapThread>,
    },
    Scopes {
        epoch: u64,
        frame_id: u64,
        scopes: Vec<DapScope>,
    },
    Variables {
        epoch: u64,
        reference: u64,
        variables: Vec<DapVariable>,
    },
    Watch {
        epoch: u64,
        frame_id: Option<u64>,
        expression: String,
        /// `(value, type, variablesReference)`.
        result: Result<(String, Option<String>, u64), String>,
    },
    Evaluated {
        expression: String,
        result: Result<(String, Option<String>, u64), String>,
    },
    Hover {
        expression: String,
        result: Result<String, String>,
    },
}

/// What the task reports: adapter events, the lifecycle, and command results.
#[derive(Debug)]
pub(super) enum DapReport {
    Event(DapEvent),
    Started(Result<DapCapabilities, String>),
    Done(DapResult),
    /// The session is over: disconnected and the adapter killed.
    Disconnected,
}

/// A [`DapReport`] and the session it belongs to.
#[derive(Debug)]
pub(super) struct Tagged {
    pub generation: u64,
    pub report: DapReport,
}

/// The editor's side of a running session task.
pub(super) struct SessionHandle {
    commands: mpsc::UnboundedSender<DapCommand>,
    stop: Arc<Notify>,
    task: JoinHandle<()>,
    killer: Arc<Mutex<Option<AdapterKiller>>>,
    stopping: bool,
}

impl SessionHandle {
    /// Starts the task: spawn the adapter, `initialize`, then serve commands.
    pub(super) fn spawn(
        generation: u64,
        command: String,
        args: Vec<String>,
        terminate_debuggee: bool,
        reports: mpsc::Sender<Tagged>,
    ) -> Self {
        let (commands, command_rx) = mpsc::unbounded_channel();
        let stop = Arc::new(Notify::new());
        let killer = Arc::new(Mutex::new(None));
        let task = tokio::spawn(run_session(SessionTask {
            out: Reporter {
                generation,
                tx: reports,
            },
            command,
            args,
            terminate_debuggee,
            commands: command_rx,
            stop: stop.clone(),
            killer: killer.clone(),
        }));
        Self {
            commands,
            stop,
            task,
            killer,
            stopping: false,
        }
    }

    pub(super) fn send(&self, command: DapCommand) {
        let _ = self.commands.send(command);
    }

    /// Ends the session gracefully: the task drops what it is waiting for,
    /// sends `disconnect` (waiting at most two seconds), kills the adapter and
    /// reports [`DapReport::Disconnected`].
    pub(super) fn disconnect(&mut self) {
        if !std::mem::replace(&mut self.stopping, true) {
            self.stop.notify_one();
        }
    }

    /// The task is gone. Check this *before* draining the reports: whatever
    /// it sent has arrived by then.
    pub(super) fn task_finished(&self) -> bool {
        self.task.is_finished()
    }
}

impl Drop for SessionHandle {
    /// Dropping a session ends it for good: no task, no adapter process.
    fn drop(&mut self) {
        self.task.abort();
        let killer = self.killer.lock().ok().and_then(|mut slot| slot.take());
        if let Some(killer) = killer {
            killer.kill();
        }
    }
}

/// Sends reports tagged with the session's generation.
struct Reporter {
    generation: u64,
    tx: mpsc::Sender<Tagged>,
}

impl Reporter {
    async fn send(&self, report: DapReport) {
        let _ = self
            .tx
            .send(Tagged {
                generation: self.generation,
                report,
            })
            .await;
    }

    async fn result(&self, result: DapResult) {
        self.send(DapReport::Done(result)).await;
    }
}

struct SessionTask {
    out: Reporter,
    command: String,
    args: Vec<String>,
    terminate_debuggee: bool,
    commands: mpsc::UnboundedReceiver<DapCommand>,
    stop: Arc<Notify>,
    killer: Arc<Mutex<Option<AdapterKiller>>>,
}

fn kill_registered(killer: &Mutex<Option<AdapterKiller>>) {
    if let Some(killer) = killer.lock().ok().and_then(|mut slot| slot.take()) {
        killer.kill();
    }
}

async fn run_session(task: SessionTask) {
    let SessionTask {
        out,
        command,
        args,
        terminate_debuggee,
        mut commands,
        stop,
        killer,
    } = task;
    let (event_tx, mut event_rx) = mpsc::channel::<DapEvent>(256);

    // Starting can take as long as the adapter does; Stop does not wait for it.
    let started = tokio::select! {
        biased;
        _ = stop.notified() => None,
        started = start(&command, &args, event_tx, &killer) => Some(started),
    };
    let client = match started {
        None => {
            kill_registered(&killer);
            out.send(DapReport::Disconnected).await;
            return;
        }
        Some(Err(e)) => {
            kill_registered(&killer);
            out.send(DapReport::Started(Err(e.to_string()))).await;
            return;
        }
        Some(Ok((client, capabilities))) => {
            out.send(DapReport::Started(Ok(capabilities))).await;
            client
        }
    };

    // Events that arrived while initializing are forwarded after `Started`,
    // so the editor knows the capabilities before `initialized`.
    let forwarder = {
        let out = Reporter {
            generation: out.generation,
            tx: out.tx.clone(),
        };
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                out.send(DapReport::Event(event)).await;
            }
        })
    };

    let mut disconnect = false;
    loop {
        let command = tokio::select! {
            biased;
            _ = stop.notified() => {
                disconnect = true;
                break;
            }
            command = commands.recv() => command,
        };
        let Some(command) = command else { break };
        tokio::select! {
            biased;
            _ = stop.notified() => {
                disconnect = true;
                break;
            }
            _ = run_command(&client, command, &out) => {}
        }
    }

    forwarder.abort();
    if disconnect {
        let _ = client.disconnect(terminate_debuggee).await;
        client.kill();
        out.send(DapReport::Disconnected).await;
    } else {
        client.kill();
    }
}

async fn start(
    command: &str,
    args: &[String],
    event_tx: mpsc::Sender<DapEvent>,
    killer: &Mutex<Option<AdapterKiller>>,
) -> anyhow::Result<(DebugAdapterClient, DapCapabilities)> {
    let client = DebugAdapterClient::spawn(command, args, event_tx).await?;
    if let Ok(mut slot) = killer.lock() {
        *slot = Some(client.killer());
    }
    match client.initialize().await {
        Ok(capabilities) => Ok((client, capabilities)),
        Err(e) => {
            client.kill();
            Err(e)
        }
    }
}

async fn run_command(client: &DebugAdapterClient, command: DapCommand, out: &Reporter) {
    match command {
        DapCommand::Configure {
            attach,
            batches,
            exception_filters,
        } => {
            if let Err(e) = client.attach(attach).await {
                let failure = Some(format!("attach failed: {e}"));
                out.result(DapResult::Configured {
                    batches: Vec::new(),
                    failure,
                })
                .await;
                return;
            }
            let batches = send_breakpoints(client, batches, exception_filters).await;
            let failure = client
                .configuration_done()
                .await
                .err()
                .map(|e| format!("configurationDone failed: {e}"));
            out.result(DapResult::Configured { batches, failure }).await;
        }
        DapCommand::SyncBreakpoints {
            batches,
            exception_filters,
        } => {
            let batches = send_breakpoints(client, batches, exception_filters).await;
            out.result(DapResult::BreakpointsSynced { batches }).await;
        }
        DapCommand::Resume {
            kind,
            thread_id,
            epoch,
        } => {
            let result = match kind {
                ResumeKind::Continue => client.continue_(thread_id).await,
                ResumeKind::StepOver => client.next(thread_id).await,
                ResumeKind::StepIn => client.step_in(thread_id).await,
                ResumeKind::StepOut => client.step_out(thread_id).await,
            };
            let error = result.err().map(|e| e.to_string());
            out.result(DapResult::Resumed { kind, epoch, error }).await;
        }
        DapCommand::FetchState(fetch) => fetch_state(client, out, fetch).await,
        DapCommand::SelectFrame {
            epoch,
            frame_id,
            watches,
        } => {
            fetch_frame(client, out, epoch, frame_id, &[]).await;
            evaluate_watches(client, out, epoch, Some(frame_id), watches).await;
        }
        DapCommand::RefreshWatches {
            epoch,
            frame_id,
            watches,
        } => evaluate_watches(client, out, epoch, frame_id, watches).await,
        DapCommand::FetchVariables { epoch, reference } => {
            if let Ok(variables) = client.variables(reference).await {
                out.result(DapResult::Variables {
                    epoch,
                    reference,
                    variables,
                })
                .await;
            }
        }
        DapCommand::Evaluate {
            expression,
            frame_id,
        } => {
            let result = client
                .evaluate(&expression, frame_id, Some("repl"))
                .await
                .map_err(|e| e.to_string());
            out.result(DapResult::Evaluated { expression, result })
                .await;
        }
        DapCommand::Hover {
            expression,
            frame_id,
        } => {
            let result = hover_text(client, &expression, frame_id).await;
            out.result(DapResult::Hover { expression, result }).await;
        }
    }
}

/// Sends each file's breakpoints, then the exception filters. Failures stay
/// in the outcomes: a breakpoint that cannot be set is not a failed session.
async fn send_breakpoints(
    client: &DebugAdapterClient,
    batches: Vec<BreakpointBatch>,
    exception_filters: Option<Vec<String>>,
) -> Vec<BatchOutcome> {
    let mut outcomes = Vec::new();
    for batch in batches {
        let source = DapSource {
            name: batch
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string()),
            path: Some(batch.path.to_string_lossy().to_string()),
        };
        let reply = client
            .set_breakpoints(&source, &batch.breakpoints)
            .await
            .map_err(|e| e.to_string());
        outcomes.push(BatchOutcome { batch, reply });
    }
    if let Some(filters) = exception_filters {
        let _ = client.set_exception_breakpoints(&filters).await;
    }
    outcomes
}

async fn fetch_state(client: &DebugAdapterClient, out: &Reporter, fetch: StateFetch) {
    let StateFetch {
        epoch,
        thread_id,
        exception_thread,
        expanded,
        watches,
    } = fetch;
    let Ok(frames) = client.stack_trace(thread_id).await else {
        return;
    };
    let top_frame = frames.first().map(|frame| frame.id);
    out.result(DapResult::Frames { epoch, frames }).await;
    if let Some(thread) = exception_thread {
        let summary = client
            .exception_info(thread)
            .await
            .ok()
            .map(|i| i.summary());
        out.result(DapResult::Exception { epoch, summary }).await;
    }
    if let Ok(threads) = client.threads().await {
        out.result(DapResult::Threads { epoch, threads }).await;
    }
    let Some(frame_id) = top_frame else { return };
    fetch_frame(client, out, epoch, frame_id, &expanded).await;
    evaluate_watches(client, out, epoch, Some(frame_id), watches).await;
}

/// A frame's scopes, the variables of the cheap ones and of `expanded`.
async fn fetch_frame(
    client: &DebugAdapterClient,
    out: &Reporter,
    epoch: u64,
    frame_id: u64,
    expanded: &[u64],
) {
    let mut references = Vec::new();
    if let Ok(scopes) = client.scopes(frame_id).await {
        references.extend(
            scopes
                .iter()
                .filter(|scope| !scope.expensive)
                .map(|scope| scope.variables_reference),
        );
        out.result(DapResult::Scopes {
            epoch,
            frame_id,
            scopes,
        })
        .await;
    }
    references.extend_from_slice(expanded);
    for reference in references {
        if let Ok(variables) = client.variables(reference).await {
            out.result(DapResult::Variables {
                epoch,
                reference,
                variables,
            })
            .await;
        }
    }
}

async fn evaluate_watches(
    client: &DebugAdapterClient,
    out: &Reporter,
    epoch: u64,
    frame_id: Option<u64>,
    watches: Vec<String>,
) {
    for expression in watches {
        let result = client
            .evaluate(&expression, frame_id, Some("watch"))
            .await
            .map_err(|e| e.to_string());
        out.result(DapResult::Watch {
            epoch,
            frame_id,
            expression,
            result,
        })
        .await;
    }
}

/// `expr = value`, its type and its first children, for the `K` popup.
async fn hover_text(
    client: &DebugAdapterClient,
    expression: &str,
    frame_id: Option<u64>,
) -> Result<String, String> {
    let (result, type_, reference) = client
        .evaluate(expression, frame_id, Some("hover"))
        .await
        .map_err(|e| e.to_string())?;
    let mut text = format!("{expression} = {result}");
    if let Some(type_) = type_.filter(|t| !t.is_empty()) {
        text.push_str(&format!("\ntype: {type_}"));
    }
    if reference > 0 {
        if let Ok(children) = client.variables(reference).await {
            text.push('\n');
            for child in children.iter().take(20) {
                text.push_str(&format!("\n  {} = {}", child.name, child.value));
            }
            if children.len() > 20 {
                text.push_str(&format!("\n  ... {} more", children.len() - 20));
            }
        }
    }
    Ok(text)
}
