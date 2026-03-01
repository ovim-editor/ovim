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
pub mod protocol;
pub mod state;
pub mod types;

use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

use crate::debug_config::DebugRunConfig;
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
    },
    /// Debuggee continued execution.
    Continued { thread_id: u64 },
    /// Thread started or exited.
    Thread { reason: String, thread_id: u64 },
    /// Output from the debuggee.
    Output { category: String, output: String },
    /// Debug session terminated.
    Terminated,
    /// Debug adapter initialized (ready for configuration).
    Initialized,
}

/// Pending debug action to execute in the async event loop.
#[derive(Debug, Clone)]
pub enum PendingDebugAction {
    /// Start a debug session with command + args + optional run config.
    Start {
        command: String,
        args: Vec<String>,
        run_config: Option<DebugRunConfig>,
    },
    /// Stop the current session.
    Stop,
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
    /// Send launch or attach based on run config, then sync breakpoints.
    LaunchOrAttach,
    /// Sync all breakpoints to the adapter and send configurationDone.
    SyncBreakpoints,
    /// Select a stack frame and refresh variables.
    SelectFrame { index: usize },
    /// Evaluate an expression and show result.
    Evaluate { expression: String },
    /// Fetch variables for an expanded object reference.
    FetchVariables { var_ref: u64 },
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
    /// The chosen run configuration for the current session.
    pub run_config: Option<DebugRunConfig>,
    /// Available debug configs for picker selection (temporary, cleared after selection).
    pub available_debug_configs: Vec<DebugRunConfig>,
    /// Background Gradle process (killed on disconnect).
    pub gradle_child: Option<tokio::process::Child>,
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
            run_config: None,
            available_debug_configs: Vec::new(),
            gradle_child: None,
        }
    }

    /// Start a debug adapter process.
    pub async fn start(&mut self, command: &str, args: &[String]) -> Result<()> {
        let client = DebugAdapterClient::spawn(command, args, self.event_tx.clone()).await?;
        self.client = Some(client);
        self.state.session_active = true;
        Ok(())
    }

    /// Initialize the debug adapter (send initialize request).
    pub async fn initialize(&mut self) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        client.initialize().await?;
        Ok(())
    }

    /// Launch a debuggee (send launch request).
    pub async fn launch(&self, config: serde_json::Value) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;
        client.launch(config).await?;
        Ok(())
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

    /// Set breakpoints for a source file.
    pub async fn set_breakpoints(
        &mut self,
        path: &Path,
        lines: &[u64],
    ) -> Result<Vec<DapBreakpoint>> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no debug adapter running"))?;

        let source = DapSource {
            name: path.file_name().map(|n| n.to_string_lossy().to_string()),
            path: Some(path.to_string_lossy().to_string()),
        };

        // Include conditions from state when syncing breakpoints.
        let source_bps: Vec<DapSourceBreakpoint> = lines
            .iter()
            .map(|&line| DapSourceBreakpoint {
                line,
                condition: self.state.breakpoint_condition(path, line).map(|s| s.to_owned()),
            })
            .collect();

        let result = client.set_breakpoints(&source, &source_bps).await?;

        // Update state.
        self.state
            .update_breakpoints(path, &result);

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

    /// Disconnect from the debug adapter.
    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(client) = self.client.take() {
            client.disconnect(true).await?;
        }
        // Kill the Gradle background process if one is running.
        if let Some(mut child) = self.gradle_child.take() {
            let _ = child.kill().await;
        }
        self.run_config = None;
        self.state.clear();
        Ok(())
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
                } => {
                    self.state.stopped_thread = *thread_id;
                    self.state.stop_reason = Some(reason.clone());
                    self.state.is_running = false;
                }
                DapEvent::Continued { thread_id: _ } => {
                    self.state.is_running = true;
                    self.state.stopped_thread = None;
                    self.state.stop_reason = None;
                    // Clear stale frame/variable data.
                    self.state.stack_frames.clear();
                    self.state.scopes.clear();
                    self.state.variables.clear();
                }
                DapEvent::Thread { reason: _, thread_id: _ } => {
                    // Thread lifecycle — we can track this later.
                }
                DapEvent::Output { category, output } => {
                    self.state.output_lines.push(format!("[{category}] {output}"));
                    // Cap output buffer.
                    if self.state.output_lines.len() > 10_000 {
                        let drain_count = self.state.output_lines.len() - 5_000;
                        self.state.output_lines.drain(..drain_count);
                    }
                }
                DapEvent::Terminated => {
                    self.state.session_active = false;
                    self.state.is_running = false;
                    self.state.stopped_thread = None;
                }
                DapEvent::Initialized => {
                    // The adapter is ready. If we have a run config, launch/attach first;
                    // otherwise fall back to just syncing breakpoints (legacy flow).
                    if self.run_config.is_some() {
                        self.pending_action = Some(PendingDebugAction::LaunchOrAttach);
                    } else {
                        self.pending_action = Some(PendingDebugAction::SyncBreakpoints);
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
