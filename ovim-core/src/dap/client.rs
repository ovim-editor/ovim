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
use tokio::sync::{mpsc, oneshot};

use super::protocol::{read_message, write_request, DapIncoming, DapRequest};
use super::types::*;
use super::DapEvent;

/// A running debug adapter process.
pub struct DebugAdapterClient {
    /// The spawned process (held for lifetime management and `kill`).
    process: Arc<std::sync::Mutex<Child>>,
    /// Set when we end the adapter ourselves, so its EOF is not a crash.
    killed: Arc<std::sync::atomic::AtomicBool>,
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
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow!("failed to spawn debug adapter '{}': {}", command, e))?;

        let child_stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let child_stderr = child.stderr.take();
        let child_stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;

        let process = Arc::new(std::sync::Mutex::new(child));
        let killed = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Keep the tail of the adapter's stderr: it is the only explanation
        // when the adapter dies (and an undrained pipe would block it).
        let stderr_tail = Arc::new(std::sync::Mutex::new(String::new()));
        if let Some(stderr) = child_stderr {
            let tail = stderr_tail.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(mut tail) = tail.lock() {
                        tail.push_str(&line);
                        tail.push('\n');
                        if tail.len() > 4096 {
                            let cut = tail.len() - 2048;
                            let cut = (cut..tail.len())
                                .find(|i| tail.is_char_boundary(*i))
                                .unwrap_or(tail.len());
                            tail.drain(..cut);
                        }
                    }
                }
            });
        }

        let pending: Arc<DashMap<u64, oneshot::Sender<Result<Value>>>> = Arc::new(DashMap::new());

        // Writer task: send requests to stdin. An adapter that stops reading
        // is as good as gone (stdin is closed with it): end the session
        // instead of letting every later request hang.
        let (writer_tx, mut writer_rx) = mpsc::channel::<DapRequest>(256);
        let writer_events = event_tx.clone();
        let writer_process = process.clone();
        let writer_killed = killed.clone();
        let writer_tail = stderr_tail.clone();
        let writer_handle = tokio::spawn(async move {
            let mut stdin = child_stdin;
            while let Some(request) = writer_rx.recv().await {
                if let Err(e) = write_request(&mut stdin, &request).await {
                    crate::log_warn!("DAP", "writing to the debug adapter failed: {e}");
                    if !writer_killed.load(Ordering::SeqCst) {
                        let detail = adapter_exit_detail(
                            &writer_process,
                            &writer_tail,
                            "stopped reading its input",
                        )
                        .await;
                        let _ = writer_events.send(DapEvent::AdapterExited { detail }).await;
                    }
                    break;
                }
            }
        });

        // Reader task: read responses and events from stdout.
        let pending_clone = pending.clone();
        let reader_process = process.clone();
        let reader_killed = killed.clone();
        let reader_tail = stderr_tail.clone();
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
                    Ok(None) | Err(_) => {
                        // EOF: the adapter is gone. Unless we ended it, that
                        // is a crash (a normal end sends `terminated` first).
                        if !reader_killed.load(Ordering::SeqCst) {
                            let detail = adapter_exit_detail(
                                &reader_process,
                                &reader_tail,
                                "closed its output",
                            )
                            .await;
                            let _ = event_tx.send(DapEvent::AdapterExited { detail }).await;
                        }
                        break;
                    }
                }
            }
            // Wake up any pending requests.
            pending_clone.clear();
        });

        Ok(Self {
            process,
            killed,
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

        tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_| anyhow!("DAP request '{}' timed out", command))?
            .map_err(|_| anyhow!("debug adapter disconnected"))?
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

    /// `setExceptionBreakpoints` with the ids of the enabled filters.
    pub async fn set_exception_breakpoints(&self, filters: &[String]) -> Result<()> {
        let args = serde_json::json!({ "filters": filters });
        self.request("setExceptionBreakpoints", Some(args)).await?;
        Ok(())
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

    /// What the thread stopped on (`exceptionInfo`).
    pub async fn exception_info(&self, thread_id: u64) -> Result<DapExceptionInfo> {
        let args = serde_json::json!({ "threadId": thread_id });
        let result = self.request("exceptionInfo", Some(args)).await?;
        Ok(serde_json::from_value(result)?)
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

    pub async fn evaluate(
        &self,
        expression: &str,
        frame_id: Option<u64>,
        context: Option<&str>,
    ) -> Result<(String, Option<String>, u64)> {
        let mut args = serde_json::json!({ "expression": expression });
        if let Some(fid) = frame_id {
            args["frameId"] = serde_json::json!(fid);
        }
        if let Some(ctx) = context {
            args["context"] = serde_json::json!(ctx);
        }
        let result = self.request("evaluate", Some(args)).await?;
        let eval_result = result
            .get("result")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let type_ = result
            .get("type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned());
        let variables_reference = result
            .get("variablesReference")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        Ok((eval_result, type_, variables_reference))
    }

    /// Asks the adapter to end the session. The adapter may exit without
    /// answering, so this waits at most two seconds; the caller follows up
    /// with [`kill`](Self::kill).
    pub async fn disconnect(&self, terminate_debuggee: bool) -> Result<()> {
        let args = serde_json::json!({
            "terminateDebuggee": terminate_debuggee,
        });
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            self.request("disconnect", Some(args)),
        )
        .await;
        Ok(())
    }

    /// Kills the adapter process. Never blocks.
    pub fn kill(&self) {
        self.killed.store(true, Ordering::SeqCst);
        if let Ok(mut child) = self.process.lock() {
            let _ = child.start_kill();
        }
    }
}

impl Drop for DebugAdapterClient {
    fn drop(&mut self) {
        self.killed.store(true, Ordering::SeqCst);
    }
}

/// How the adapter ended, for the message shown to the user: exit status
/// plus the tail of its stderr.
async fn adapter_exit_detail(
    process: &std::sync::Mutex<Child>,
    stderr_tail: &std::sync::Mutex<String>,
    still_running: &str,
) -> String {
    let mut status = None;
    for _ in 0..10 {
        status = process
            .lock()
            .ok()
            .and_then(|mut child| child.try_wait().ok().flatten());
        if status.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let how = match status {
        Some(status) => match status.code() {
            Some(code) => format!("exit code {code}"),
            None => format!("{status}"),
        },
        None => still_running.to_string(),
    };
    // Let the stderr reader catch up with what the adapter printed last.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let tail = stderr_tail
        .lock()
        .map(|t| t.trim().to_string())
        .unwrap_or_default();
    let tail: String = {
        let lines: Vec<&str> = tail.lines().collect();
        lines[lines.len().saturating_sub(3)..].join(" | ")
    };
    if tail.is_empty() {
        how
    } else {
        format!("{how}: {tail}")
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
            let thread_id = body
                .and_then(|b| b.get("threadId"))
                .and_then(|v| v.as_u64());
            let all_threads_stopped = body
                .and_then(|b| b.get("allThreadsStopped"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let description = ["description", "text"].iter().find_map(|key| {
                body.and_then(|b| b.get(*key))
                    .and_then(|v| v.as_str())
                    .filter(|t| !t.is_empty())
                    .map(str::to_owned)
            });
            Some(DapEvent::Stopped {
                reason,
                thread_id,
                all_threads_stopped,
                description,
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
        "exited" => Some(DapEvent::Exited {
            exit_code: body
                .and_then(|b| b.get("exitCode"))
                .and_then(|v| v.as_i64())
                .map(|c| c as i32),
        }),
        "terminated" => Some(DapEvent::Terminated),
        "initialized" => Some(DapEvent::Initialized),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// An adapter that closes its stdin but keeps running: the writer fails,
    /// and that has to reach the editor as an event (printing to stderr would
    /// paint over the TUI).
    #[tokio::test]
    async fn an_adapter_that_stops_reading_ends_the_session_through_an_event() {
        let (tx, mut rx) = mpsc::channel(8);
        let client = DebugAdapterClient::spawn(
            "sh",
            &["-c".to_string(), "exec 0<&-; sleep 30".to_string()],
            tx,
        )
        .await
        .unwrap();

        // The child closes stdin a moment after it starts; writes before that
        // land in the pipe buffer. Keep asking until one fails.
        let event = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let _ = tokio::time::timeout(Duration::from_millis(50), client.threads()).await;
                if let Ok(event) = rx.try_recv() {
                    return event;
                }
            }
        })
        .await
        .expect("a failed write must be reported");

        assert!(
            matches!(&event, DapEvent::AdapterExited { detail } if detail.contains("stopped reading")),
            "{event:?}"
        );
        client.kill();
    }
}
