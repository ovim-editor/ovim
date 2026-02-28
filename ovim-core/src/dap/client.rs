//! Debug adapter client — manages a single DAP server process.
//!
//! Mirrors the `LanguageServer` from `lsp/server.rs`:
//! - Process spawning via `tokio::process::Child`
//! - stdin/stdout communication with Content-Length framing
//! - Request/response matching via `DashMap<u64, oneshot::Sender>`
//! - Background reader task for demuxing responses and events

use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use dashmap::DashMap;
use serde_json::Value;
use tokio::io::BufReader;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

use super::protocol::{read_message, write_request, DapIncoming, DapRequest};
use super::types::*;
use super::DapEvent;

/// A running debug adapter process.
pub struct DebugAdapterClient {
    /// The spawned process (held for lifetime management).
    _process: Mutex<Child>,
    /// Stdin writer channel.
    writer_tx: mpsc::Sender<DapRequest>,
    /// Monotonically increasing sequence number.
    next_seq: AtomicU64,
    /// Pending response map: request_seq → oneshot sender.
    pending: Arc<DashMap<u64, oneshot::Sender<Result<Value>>>>,
    /// Background reader task handle.
    _reader_handle: tokio::task::JoinHandle<()>,
    /// Background writer task handle.
    _writer_handle: tokio::task::JoinHandle<()>,
}

impl DebugAdapterClient {
    /// Spawn a debug adapter process and set up communication.
    pub async fn spawn(
        command: &str,
        args: &[String],
        event_tx: mpsc::Sender<DapEvent>,
    ) -> Result<Self> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| anyhow!("failed to spawn debug adapter '{}': {}", command, e))?;

        let child_stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let child_stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;

        let pending: Arc<DashMap<u64, oneshot::Sender<Result<Value>>>> =
            Arc::new(DashMap::new());

        // Writer task: send requests to stdin.
        let (writer_tx, mut writer_rx) = mpsc::channel::<DapRequest>(256);
        let writer_handle = tokio::spawn(async move {
            let mut stdin = child_stdin;
            while let Some(request) = writer_rx.recv().await {
                if let Err(e) = write_request(&mut stdin, &request).await {
                    eprintln!("DAP write error: {e}");
                    break;
                }
            }
        });

        // Reader task: read responses and events from stdout.
        let pending_clone = pending.clone();
        let reader_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(child_stdout);
            loop {
                match read_message(&mut reader).await {
                    Ok(Some(msg)) => {
                        if msg.is_response() {
                            if let Some(request_seq) = msg.request_seq {
                                if let Some((_, tx)) = pending_clone.remove(&request_seq) {
                                    let result = if msg.success.unwrap_or(false) {
                                        Ok(msg.body.unwrap_or(Value::Null))
                                    } else {
                                        Err(anyhow!(
                                            "DAP error: {}",
                                            msg.message
                                                .unwrap_or_else(|| "unknown error".to_owned())
                                        ))
                                    };
                                    let _ = tx.send(result);
                                }
                            }
                        } else if msg.is_event() {
                            if let Some(event) = parse_dap_event(&msg) {
                                if event_tx.send(event).await.is_err() {
                                    break; // Receiver dropped.
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        // EOF — adapter exited.
                        let _ = event_tx.send(DapEvent::Terminated).await;
                        break;
                    }
                    Err(e) => {
                        eprintln!("DAP read error: {e}");
                        let _ = event_tx.send(DapEvent::Terminated).await;
                        break;
                    }
                }
            }
            // Wake up any pending requests.
            pending_clone.clear();
        });

        Ok(Self {
            _process: Mutex::new(child),
            writer_tx,
            next_seq: AtomicU64::new(1),
            pending,
            _reader_handle: reader_handle,
            _writer_handle: writer_handle,
        })
    }

    /// Send a DAP request and wait for the response.
    async fn request(&self, command: &str, arguments: Option<Value>) -> Result<Value> {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);

        let (tx, rx) = oneshot::channel();
        self.pending.insert(seq, tx);

        let request = DapRequest {
            seq,
            message_type: "request",
            command: command.to_owned(),
            arguments,
        };

        self.writer_tx
            .send(request)
            .await
            .map_err(|_| anyhow!("debug adapter stdin closed"))?;

        let result = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_| anyhow!("DAP request '{}' timed out", command))?
            .map_err(|_| anyhow!("debug adapter disconnected"))?;

        result
    }

    // ---- High-level request methods ----

    pub async fn initialize(&self) -> Result<DapCapabilities> {
        let args = serde_json::json!({
            "clientID": "ovim",
            "clientName": "ovim",
            "adapterID": "hyperion-dap",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "supportsVariableType": true,
        });
        let result = self.request("initialize", Some(args)).await?;
        let caps: DapCapabilities = serde_json::from_value(result)?;
        Ok(caps)
    }

    pub async fn launch(&self, config: Value) -> Result<()> {
        self.request("launch", Some(config)).await?;
        Ok(())
    }

    pub async fn attach(&self, config: Value) -> Result<()> {
        self.request("attach", Some(config)).await?;
        Ok(())
    }

    pub async fn configuration_done(&self) -> Result<()> {
        self.request("configurationDone", None).await?;
        Ok(())
    }

    pub async fn set_breakpoints(
        &self,
        source: &DapSource,
        breakpoints: &[DapSourceBreakpoint],
    ) -> Result<Vec<DapBreakpoint>> {
        let args = serde_json::json!({
            "source": source,
            "breakpoints": breakpoints,
        });
        let result = self.request("setBreakpoints", Some(args)).await?;
        let bps: Vec<DapBreakpoint> = serde_json::from_value(
            result
                .get("breakpoints")
                .cloned()
                .unwrap_or(Value::Array(vec![])),
        )?;
        Ok(bps)
    }

    pub async fn continue_(&self, thread_id: u64) -> Result<()> {
        let args = serde_json::json!({ "threadId": thread_id });
        self.request("continue", Some(args)).await?;
        Ok(())
    }

    pub async fn next(&self, thread_id: u64) -> Result<()> {
        let args = serde_json::json!({ "threadId": thread_id });
        self.request("next", Some(args)).await?;
        Ok(())
    }

    pub async fn step_in(&self, thread_id: u64) -> Result<()> {
        let args = serde_json::json!({ "threadId": thread_id });
        self.request("stepIn", Some(args)).await?;
        Ok(())
    }

    pub async fn step_out(&self, thread_id: u64) -> Result<()> {
        let args = serde_json::json!({ "threadId": thread_id });
        self.request("stepOut", Some(args)).await?;
        Ok(())
    }

    pub async fn threads(&self) -> Result<Vec<DapThread>> {
        let result = self.request("threads", None).await?;
        let threads: Vec<DapThread> = serde_json::from_value(
            result
                .get("threads")
                .cloned()
                .unwrap_or(Value::Array(vec![])),
        )?;
        Ok(threads)
    }

    pub async fn stack_trace(&self, thread_id: u64) -> Result<Vec<DapStackFrame>> {
        let args = serde_json::json!({ "threadId": thread_id });
        let result = self.request("stackTrace", Some(args)).await?;
        let frames: Vec<DapStackFrame> = serde_json::from_value(
            result
                .get("stackFrames")
                .cloned()
                .unwrap_or(Value::Array(vec![])),
        )?;
        Ok(frames)
    }

    pub async fn scopes(&self, frame_id: u64) -> Result<Vec<DapScope>> {
        let args = serde_json::json!({ "frameId": frame_id });
        let result = self.request("scopes", Some(args)).await?;
        let scopes: Vec<DapScope> = serde_json::from_value(
            result
                .get("scopes")
                .cloned()
                .unwrap_or(Value::Array(vec![])),
        )?;
        Ok(scopes)
    }

    pub async fn variables(&self, variables_reference: u64) -> Result<Vec<DapVariable>> {
        let args = serde_json::json!({ "variablesReference": variables_reference });
        let result = self.request("variables", Some(args)).await?;
        let vars: Vec<DapVariable> = serde_json::from_value(
            result
                .get("variables")
                .cloned()
                .unwrap_or(Value::Array(vec![])),
        )?;
        Ok(vars)
    }

    pub async fn disconnect(&self, terminate_debuggee: bool) -> Result<()> {
        let args = serde_json::json!({
            "terminateDebuggee": terminate_debuggee,
        });
        // Don't wait for response — the adapter may exit immediately.
        let _ = self.request("disconnect", Some(args)).await;
        Ok(())
    }
}

/// Parse a DAP event message into a `DapEvent`.
fn parse_dap_event(msg: &DapIncoming) -> Option<DapEvent> {
    let event_name = msg.event.as_deref()?;
    let body = msg.body.as_ref();

    match event_name {
        "stopped" => {
            let reason = body
                .and_then(|b| b.get("reason"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_owned();
            let thread_id = body.and_then(|b| b.get("threadId")).and_then(|v| v.as_u64());
            let all_threads_stopped = body
                .and_then(|b| b.get("allThreadsStopped"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Some(DapEvent::Stopped {
                reason,
                thread_id,
                all_threads_stopped,
            })
        }
        "continued" => {
            let thread_id = body
                .and_then(|b| b.get("threadId"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            Some(DapEvent::Continued { thread_id })
        }
        "thread" => {
            let reason = body
                .and_then(|b| b.get("reason"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_owned();
            let thread_id = body
                .and_then(|b| b.get("threadId"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            Some(DapEvent::Thread { reason, thread_id })
        }
        "output" => {
            let category = body
                .and_then(|b| b.get("category"))
                .and_then(|v| v.as_str())
                .unwrap_or("console")
                .to_owned();
            let output = body
                .and_then(|b| b.get("output"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            Some(DapEvent::Output { category, output })
        }
        "terminated" | "exited" => Some(DapEvent::Terminated),
        "initialized" => Some(DapEvent::Initialized),
        _ => None,
    }
}
