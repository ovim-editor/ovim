//! Official Claude Agent SDK transport. No credentials or inference endpoints
//! are handled here. The installed, unmodified Claude CLI owns the agent loop.
use super::{StreamChunk, ToolCallInfo};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::UnboundedSender;

const MAX_EVENT_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const SDK_VERSION: &str = "0.3.278";

// Curated defaults verified against Anthropic's model reference on 2026-09-21:
// https://platform.claude.com/docs/en/models/overview
// These are choices, not account entitlements. The installed CLI enforces access.
// Other IDs/aliases remain available through /model or the profile configuration.
pub(crate) struct ModelPreset {
    pub id: &'static str,
    alias: &'static str,
    supports_effort: bool,
    default_effort: Option<&'static str>,
}

pub(crate) const MODEL_PRESETS: &[ModelPreset] = &[
    ModelPreset {
        id: "default",
        alias: "default",
        supports_effort: true,
        default_effort: None,
    },
    ModelPreset {
        id: "claude-sonnet-5",
        alias: "sonnet",
        supports_effort: true,
        default_effort: Some("high"),
    },
    ModelPreset {
        id: "claude-opus-5",
        alias: "opus",
        supports_effort: true,
        default_effort: Some("high"),
    },
    ModelPreset {
        id: "claude-fable-5-1",
        alias: "fable",
        supports_effort: true,
        default_effort: Some("medium"),
    },
    ModelPreset {
        id: "claude-haiku-4-5-20251001",
        alias: "haiku",
        supports_effort: false,
        default_effort: None,
    },
];

fn model_preset(model: &str) -> Option<&'static ModelPreset> {
    let model = model.strip_suffix("[1m]").unwrap_or(model);
    MODEL_PRESETS
        .iter()
        .find(|preset| preset.id == model || preset.alias == model)
}

pub(crate) fn supports_effort(model: &str) -> bool {
    model_preset(model).is_none_or(|preset| preset.supports_effort)
}

// User-requested Fable default; Opus/Sonnet follow Anthropic's effort guidance:
// https://platform.claude.com/docs/en/build-with-claude/effort
pub(crate) fn default_effort(model: &str) -> Option<&'static str> {
    model_preset(model).and_then(|preset| preset.default_effort)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Request {
    pub cwd: PathBuf,
    pub executable: PathBuf,
    pub model: String,
    pub effort: Option<String>,
    pub allow_edits: bool,
    pub resume: Option<String>,
    pub content: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Event {
    Text {
        text: String,
    },
    Thinking {
        text: String,
    },
    MessageEnd,
    EditorRequest {
        id: String,
        request: Value,
    },
    EditorCancelled {
        id: String,
    },
    ToolStart {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        id: String,
        content: String,
        error: bool,
    },
    Permission {
        id: String,
        name: String,
        input: Value,
        reason: String,
    },
    PermissionCancelled,
    Session {
        id: String,
    },
    Done,
    Error {
        message: String,
    },
}

/// Killing only Node can leave Claude and its tools running after cancellation.
/// The process group also covers descendants started by the SDK on Unix.
struct ProcessTree(tokio::process::Child);

impl Drop for ProcessTree {
    fn drop(&mut self) {
        if let Some(id) = self.0.id() {
            #[cfg(unix)]
            {
                use nix::{
                    sys::signal::{killpg, Signal},
                    unistd::Pid,
                };
                let _ = killpg(Pid::from_raw(id as i32), Signal::SIGKILL);
            }
            #[cfg(windows)]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &id.to_string(), "/T", "/F"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let _ = self.0.start_kill();
        }
    }
}

/// Read a framed event without allowing an unterminated line to grow forever.
async fn read_event(reader: &mut (impl AsyncBufRead + Unpin)) -> Result<Option<Event>> {
    let mut line = Vec::new();
    loop {
        let bytes = reader.fill_buf().await?;
        if bytes.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            bail!("Claude runtime returned an incomplete event");
        }
        let count = bytes
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(bytes.len());
        if line.len() + count > MAX_EVENT_BYTES {
            bail!("Claude runtime event exceeds 8 MiB");
        }
        line.extend_from_slice(&bytes[..count]);
        reader.consume(count);
        if line.last() == Some(&b'\n') {
            return serde_json::from_slice(&line)
                .context("Invalid Claude runtime event")
                .map(Some);
        }
    }
}

pub(crate) async fn stream(request: Request, tx: UnboundedSender<StreamChunk>) -> Result<()> {
    let directory = tempfile::Builder::new().prefix("ovim-claude-").tempdir()?;
    let mut sdk = Vec::new();
    flate2::read::GzDecoder::new(include_bytes!("claude/sdk.mjs.gz").as_slice())
        .read_to_end(&mut sdk)?;
    std::fs::write(directory.path().join("sdk.mjs"), sdk)?;
    std::fs::write(
        directory.path().join("editor-mcp.mjs"),
        include_str!("claude/editor-mcp.mjs"),
    )?;
    let helper = directory.path().join("runtime.mjs");
    std::fs::write(&helper, include_str!("claude/runtime.mjs"))?;
    let mut command = tokio::process::Command::new("node");
    command
        .arg(helper)
        .current_dir(&request.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = ProcessTree(command.spawn().context("Could not start Claude runtime. Install Node.js and Claude Code, then run `claude auth login` in your terminal")?);
    let mut stdin = child
        .0
        .stdin
        .take()
        .context("Claude runtime stdin missing")?;
    let mut stdout = BufReader::new(
        child
            .0
            .stdout
            .take()
            .context("Claude runtime stdout missing")?,
    );
    let stderr = child
        .0
        .stderr
        .take()
        .context("Claude runtime stderr missing")?;
    // Always drain stderr, retaining only a bounded tail for startup failures.
    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut tail = Vec::new();
        let mut buffer = [0; 4096];
        while let Ok(count) = reader.read(&mut buffer).await {
            if count == 0 {
                break;
            }
            tail.extend_from_slice(&buffer[..count]);
            if tail.len() > 16384 {
                tail.drain(..tail.len() - 16384);
            }
        }
        String::from_utf8_lossy(&tail).into_owned()
    });
    stdin.write_all(&serde_json::to_vec(&request)?).await?;
    stdin.write_all(b"\n").await?;
    let (events, mut event_rx) = tokio::sync::mpsc::channel(32);
    // Frame reads are isolated from the select below: a permission response
    // cannot cancel a half-read UTF-8/JSON frame and lose its prefix.
    let reader_task = tokio::spawn(async move {
        loop {
            let event = read_event(&mut stdout).await;
            let finished = !matches!(&event, Ok(Some(_)));
            if events.send(event).await.is_err() || finished {
                break;
            }
        }
    });
    struct ReaderGuard(tokio::task::JoinHandle<()>);
    impl Drop for ReaderGuard {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _reader = ReaderGuard(reader_task);
    let mut answers = tokio::task::JoinSet::new();
    let mut saw_session = false;
    loop {
        let event = tokio::select! {
            event = event_rx.recv() => match event.transpose()? {
                Some(Some(event)) => event,
                _ => break,
            },
            answer = answers.join_next(), if !answers.is_empty() => {
                let (id, mut answer): (String, Value) = answer.context("Response task missing")??;
                answer["id"] = Value::String(id);
                stdin.write_all(&serde_json::to_vec(&answer)?).await?;
                stdin.write_all(b"\n").await?;
                continue;
            }
        };
        let chunk = match event {
            Event::EditorRequest { id, request } => {
                let (response, answer) = tokio::sync::oneshot::channel();
                tx.send(StreamChunk::ExternalEditorRequest {
                    id: id.clone(),
                    request,
                    response,
                })?;
                answers.spawn(async move { (id, serde_json::json!({"result": answer.await.unwrap_or(serde_json::json!({"error":"Editor request cancelled"}))})) });
                continue;
            }
            Event::EditorCancelled { id } => StreamChunk::ExternalEditorCancelled(id),
            Event::Text { text } => StreamChunk::Content(text),
            Event::Thinking { text } => StreamChunk::Thinking(text),
            Event::MessageEnd => StreamChunk::AgentMessageComplete,
            Event::ToolStart { id, name, input } => {
                if id.is_empty() || name.is_empty() || !input.is_object() {
                    bail!("Invalid Claude tool observation");
                }
                StreamChunk::ExternalToolStart(ToolCallInfo {
                    id,
                    name,
                    arguments: input,
                })
            }
            Event::ToolResult { id, content, error } => {
                StreamChunk::ExternalToolResult { id, content, error }
            }
            Event::Permission {
                id,
                name,
                input,
                reason,
            } => {
                if id.is_empty() || name.is_empty() || !input.is_object() {
                    bail!("Invalid Claude permission request");
                }
                let (response, answer) = tokio::sync::oneshot::channel();
                tx.send(StreamChunk::ExternalPermission {
                    name,
                    input,
                    reason,
                    response,
                })?;
                answers.spawn(async move {
                    (
                        id,
                        serde_json::to_value(answer.await.unwrap_or_default())
                            .expect("permission serializes"),
                    )
                });
                continue;
            }
            Event::PermissionCancelled => StreamChunk::ExternalPermissionCancelled,
            Event::Session { id } => {
                if id.is_empty() {
                    bail!("Empty Claude session ID");
                }
                saw_session = true;
                StreamChunk::ExternalSession(id)
            }
            Event::Done => {
                if !saw_session {
                    bail!("Claude runtime completed without a session checkpoint");
                }
                // Close the SDK process tree before presenting a completed turn.
                drop(stdin);
                drop(child);
                let _ = stderr_task.await;
                tx.send(StreamChunk::Done)?;
                return Ok(());
            }
            Event::Error { message } => bail!("{message}"),
        };
        tx.send(chunk)?;
    }
    drop(stdin);
    drop(child);
    let detail = stderr_task.await.unwrap_or_default();
    let detail = if detail.trim().is_empty() {
        "the helper closed its output without a completion event or diagnostic"
    } else {
        detail.trim()
    };
    bail!(
        "Claude runtime exited without completing the turn: {}",
        super::redact_high_risk_tokens(detail)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn protocol_rejects_malformed_truncated_and_oversized_events() {
        for bytes in [
            b"{\"type\":\"unknown\"}\n".as_slice(),
            b"{\"type\":\"done\"}",
            b"{\"type\":\"text\"}\n",
        ] {
            assert!(read_event(&mut BufReader::new(bytes)).await.is_err());
        }
        let bytes = vec![b'a'; MAX_EVENT_BYTES + 1];
        assert!(read_event(&mut BufReader::new(bytes.as_slice()))
            .await
            .is_err());
        assert!(matches!(
            read_event(&mut BufReader::new(b"{\"type\":\"done\"}\n".as_slice()))
                .await
                .unwrap(),
            Some(Event::Done)
        ));
    }

    #[test]
    fn shipped_sdk_is_the_unmodified_pinned_module() {
        use sha2::{Digest, Sha256};
        let mut sdk = Vec::new();
        flate2::read::GzDecoder::new(include_bytes!("claude/sdk.mjs.gz").as_slice())
            .read_to_end(&mut sdk)
            .unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(sdk)),
            "d768bb75542ea853a66c570bd82b010a7b843f4949ab3ebb20e9d1f2a27af881"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_closes_descendant_processes_not_only_the_helper() {
        let mut command = tokio::process::Command::new("sh");
        command
            .args(["-c", "sleep 30 & echo ready; wait"])
            .process_group(0)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut process = ProcessTree(command.spawn().unwrap());
        let mut output = BufReader::new(process.0.stdout.take().unwrap());
        let mut ready = String::new();
        output.read_line(&mut ready).await.unwrap();
        assert_eq!(ready.trim(), "ready");
        drop(process);
        // The sleep inherits stdout. EOF proves that it was killed as well;
        // killing only the shell would leave this pipe open for 30 seconds.
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            output.read_to_end(&mut Vec::new()),
        )
        .await
        .unwrap()
        .unwrap();
    }
}
