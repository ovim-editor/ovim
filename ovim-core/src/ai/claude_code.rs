//! Official Claude Agent SDK transport. No credentials or inference endpoints
//! are handled here. The installed, unmodified Claude CLI owns the agent loop.
use super::{StreamChunk, ToolCallInfo};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::UnboundedSender;

#[path = "claude/approval.rs"]
mod approval;
pub(crate) use approval::permission_summary;
#[path = "claude/sdk_install.rs"]
mod sdk_install;

const MAX_EVENT_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const SDK_VERSION: &str = "0.3.278";

/// Claude Agent SDK permission modes. Keep the stable SDK values separate
/// from their presentation so every frontend sends the same runtime contract.
pub(crate) const PERMISSION_MODES: &[super::AiPermissionModeOption] = &[
    super::AiPermissionModeOption {
        id: "auto",
        label: "Auto",
        description: "Let Claude classify permission prompts and approve or deny them.",
        requires_confirmation: false,
    },
    super::AiPermissionModeOption {
        id: "default",
        label: "Manual",
        description: "Ask before operations that require permission.",
        requires_confirmation: false,
    },
    super::AiPermissionModeOption {
        id: "acceptEdits",
        label: "Accept edits",
        description: "Approve file edits automatically and ask for other protected operations.",
        requires_confirmation: false,
    },
    super::AiPermissionModeOption {
        id: "plan",
        label: "Plan",
        description: "Explore and plan; ask before changing project files.",
        requires_confirmation: false,
    },
    super::AiPermissionModeOption {
        id: "dontAsk",
        label: "Don't ask",
        description: "Deny calls that would require approval instead of prompting.",
        requires_confirmation: false,
    },
    super::AiPermissionModeOption {
        id: "bypassPermissions",
        label: "Bypass permissions",
        description:
            "Skip ordinary approval prompts; Claude policy and hook restrictions still apply.",
        requires_confirmation: true,
    },
];

/// Whether `mode` disables approval prompts and so needs an explicit,
/// repeated command to enable and is never remembered.
pub(crate) fn permission_mode_requires_confirmation(mode: &str) -> bool {
    PERMISSION_MODES
        .iter()
        .any(|option| option.id == mode && option.requires_confirmation)
}

pub(crate) const PERMISSION_CAPABILITY: super::AiPermissionModes = super::AiPermissionModes {
    default: "auto",
    options: PERMISSION_MODES,
};

// Curated defaults verified against Anthropic's model reference on 2026-09-29:
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
        id: "claude-sonnet-5-5",
        alias: "sonnet",
        supports_effort: true,
        default_effort: Some("high"),
    },
    ModelPreset {
        id: "claude-opus-5-5",
        alias: "opus",
        supports_effort: true,
        default_effort: Some("medium"),
    },
    ModelPreset {
        id: "claude-fable-5-1",
        alias: "fable",
        supports_effort: true,
        default_effort: Some("medium"),
    },
    ModelPreset {
        id: "claude-haiku-5-5",
        alias: "haiku",
        supports_effort: true,
        default_effort: Some("medium"),
    },
];

/// Known IDs the picker no longer offers. Saved selections and `/model` can
/// still name them, and they keep their own effort support: Haiku 4.5 has none.
const LEGACY_MODEL_PRESETS: &[ModelPreset] = &[ModelPreset {
    id: "claude-haiku-4-5-20251001",
    alias: "haiku-4-5",
    supports_effort: false,
    default_effort: None,
}];

fn model_preset(model: &str) -> Option<&'static ModelPreset> {
    let model = model.strip_suffix("[1m]").unwrap_or(model);
    MODEL_PRESETS
        .iter()
        .chain(LEGACY_MODEL_PRESETS)
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
    /// Whether Claude may load the workspace's own `.claude` settings. The SDK
    /// skips Claude Code's folder-trust dialog, so a cloned repository's hooks
    /// and permission rules would otherwise run unprompted.
    pub project_settings: bool,
    pub permission_mode: String,
    pub resume: Option<String>,
    pub content: Vec<Value>,
}

/// Where Claude Code keeps its global state, including which folders the user
/// has trusted through its own dialog.
fn global_config_path(
    config_dir: Option<std::ffi::OsString>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    match config_dir.filter(|dir| !dir.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir).join(".claude.json")),
        None => home.map(|home| home.join(".claude.json")),
    }
}

/// Whether Claude Code has already trusted `workspace` or one of its
/// ancestors. This only reads Claude's record; Ovim never grants trust.
pub(crate) fn workspace_trusted(workspace: &std::path::Path) -> bool {
    workspace_trusted_by(
        global_config_path(std::env::var_os("CLAUDE_CONFIG_DIR"), dirs::home_dir()),
        workspace,
    )
}

fn workspace_trusted_by(config_path: Option<PathBuf>, workspace: &std::path::Path) -> bool {
    config_path
        .and_then(|path| std::fs::read(path).ok())
        .is_some_and(|bytes| trusted_in_global_config(&bytes, workspace))
}

fn trusted_in_global_config(config: &[u8], workspace: &std::path::Path) -> bool {
    let Ok(config) = serde_json::from_slice::<Value>(config) else {
        return false;
    };
    let Some(projects) = config.get("projects").and_then(Value::as_object) else {
        return false;
    };
    workspace.ancestors().any(|folder| {
        projects
            .get(folder.to_string_lossy().as_ref())
            .and_then(|project| project.get("hasTrustDialogAccepted"))
            .and_then(Value::as_bool)
            == Some(true)
    })
}

/// Whether the workspace carries project-level Claude configuration that an
/// untrusted folder keeps Claude from loading.
pub(crate) fn has_project_configuration(workspace: &std::path::Path) -> bool {
    [".claude", "CLAUDE.md", ".mcp.json"]
        .iter()
        .any(|name| workspace.join(name).exists())
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
    std::fs::copy(
        sdk_install::ensure_sdk().await?,
        directory.path().join("sdk.mjs"),
    )?;
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
    fn haiku_alias_names_the_current_model_and_keeps_effort() {
        assert_eq!(model_preset("haiku").unwrap().id, "claude-haiku-5-5");
        assert!(supports_effort("haiku"));
        assert_eq!(default_effort("haiku"), Some("medium"));
        assert_eq!(default_effort("claude-haiku-5-5"), Some("medium"));
        // The picker no longer offers Haiku 4.5, but its ID still resolves and
        // still omits effort.
        assert!(MODEL_PRESETS
            .iter()
            .all(|preset| preset.id != "claude-haiku-4-5-20251001"));
        assert!(!supports_effort("claude-haiku-4-5-20251001"));
        assert_eq!(default_effort("claude-haiku-4-5-20251001"), None);
    }

    #[test]
    fn trust_comes_only_from_claudes_own_record_for_the_folder_or_an_ancestor() {
        let config = br#"{
            "projects": {
                "/work": {"hasTrustDialogAccepted": true},
                "/work/untrusted": {"hasTrustDialogAccepted": false},
                "/other": {"hasTrustDialogAccepted": "yes"},
                "/exact/repo": {"hasTrustDialogAccepted": true},
                "/typo": {}
            }
        }"#;
        let trusted = |path: &str| trusted_in_global_config(config, std::path::Path::new(path));
        assert!(trusted("/work"));
        assert!(trusted("/work/sub/dir"));
        assert!(trusted("/exact/repo"));
        // Any trusted ancestor is enough; an unrecorded sibling, an explicit
        // refusal with no trusted ancestor, or a non-boolean is untrusted.
        assert!(trusted("/work/untrusted"));
        assert!(!trusted("/other"));
        assert!(!trusted("/other/project"));
        assert!(!trusted("/typo"));
        assert!(!trusted("/exact"));
        assert!(!trusted("/"));
    }

    #[test]
    fn trust_is_read_from_the_config_directory_claude_uses() {
        use std::ffi::OsString;
        assert_eq!(
            global_config_path(
                Some(OsString::from("/cfg")),
                Some(PathBuf::from("/home/me"))
            ),
            Some(PathBuf::from("/cfg/.claude.json"))
        );
        assert_eq!(
            global_config_path(Some(OsString::new()), Some(PathBuf::from("/home/me"))),
            Some(PathBuf::from("/home/me/.claude.json"))
        );
        assert_eq!(global_config_path(None, None), None);

        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(".claude.json");
        let workspace = std::path::Path::new("/work/repo");
        assert!(!workspace_trusted_by(Some(config.clone()), workspace));
        std::fs::write(
            &config,
            r#"{"projects":{"/work":{"hasTrustDialogAccepted":true}}}"#,
        )
        .unwrap();
        assert!(workspace_trusted_by(Some(config), workspace));
        assert!(!workspace_trusted_by(None, workspace));
    }

    #[test]
    fn missing_or_malformed_global_config_is_untrusted() {
        let workspace = std::path::Path::new("/work");
        for config in [
            b"".as_slice(),
            b"not json",
            b"null",
            b"{}",
            br#"{"projects": []}"#,
            br#"{"projects": {"/work": true}}"#,
        ] {
            assert!(!trusted_in_global_config(config, workspace));
        }
    }

    #[test]
    fn project_configuration_is_detected_from_claude_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!has_project_configuration(dir.path()));
        std::fs::write(dir.path().join("CLAUDE.md"), "notes").unwrap();
        assert!(has_project_configuration(dir.path()));
        let other = tempfile::tempdir().unwrap();
        std::fs::create_dir(other.path().join(".claude")).unwrap();
        assert!(has_project_configuration(other.path()));
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
