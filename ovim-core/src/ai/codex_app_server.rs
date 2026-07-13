//! Subscription-backed Codex transport using the documented app-server protocol.
//!
//! Authentication and token refresh remain owned by the installed Codex CLI.
//! Interactive chats share one app-server process and retain a native Codex
//! thread per ovim conversation. Non-chat edit requests remain ephemeral.

use super::{AiProfileConfig, StreamChunk};
use anyhow::{anyhow, bail, Context, Result};
#[cfg(test)]
use ignore::WalkBuilder;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc::UnboundedSender;

const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) async fn request(
    profile: &AiProfileConfig,
    initial_input: &str,
    continuation_input: Option<&str>,
    instructions: &str,
    file_path: Option<&str>,
    tools: Option<&[Value]>,
    stream_tx: Option<UnboundedSender<StreamChunk>>,
    session_key: Option<&str>,
) -> Result<String> {
    let cwd = request_cwd(file_path)?;
    if let Some(session_key) = session_key {
        return request_persistent(
            profile,
            initial_input,
            continuation_input.unwrap_or(initial_input),
            instructions,
            &cwd,
            tools.unwrap_or_default(),
            stream_tx,
            session_key,
        )
        .await;
    }

    request_ephemeral(profile, initial_input, instructions, &cwd, tools, stream_tx).await
}

async fn request_ephemeral(
    profile: &AiProfileConfig,
    input: &str,
    instructions: &str,
    cwd: &Path,
    tools: Option<&[Value]>,
    stream_tx: Option<UnboundedSender<StreamChunk>>,
) -> Result<String> {
    let project_tools_enabled = stream_tx.is_some();
    let mut client = AppServerClient::spawn(cwd).await?;
    client.initialize(project_tools_enabled).await?;
    let thread_id = client
        .start_thread(profile, instructions, cwd, tools.unwrap_or_default(), true)
        .await?;
    client.start_turn(profile, &thread_id, input).await?;
    let output = client.stream_turn(stream_tx, cwd).await;
    client.stop().await;
    output
}

#[derive(Clone)]
struct PersistentThread {
    id: String,
    configuration: String,
}

#[derive(Default)]
struct CodexRuntime {
    client: Option<AppServerClient>,
    threads: std::collections::HashMap<String, PersistentThread>,
}

fn runtime() -> &'static tokio::sync::Mutex<CodexRuntime> {
    static RUNTIME: OnceLock<tokio::sync::Mutex<CodexRuntime>> = OnceLock::new();
    RUNTIME.get_or_init(|| tokio::sync::Mutex::new(CodexRuntime::default()))
}

async fn request_persistent(
    profile: &AiProfileConfig,
    initial_input: &str,
    continuation_input: &str,
    instructions: &str,
    cwd: &Path,
    tools: &[Value],
    stream_tx: Option<UnboundedSender<StreamChunk>>,
    session_key: &str,
) -> Result<String> {
    let configuration = format!(
        "{}\n{}\n{}\n{}",
        profile.model,
        cwd.display(),
        instructions,
        serde_json::to_string(tools)?
    );
    let runtime_key = format!("{}:{}", cwd.display(), session_key);
    let mut runtime = runtime().lock().await;

    let client_dead = runtime
        .client
        .as_mut()
        .is_some_and(|client| !client.is_alive());
    if client_dead {
        runtime.client = None;
        runtime.threads.clear();
    }
    if runtime.client.is_none() {
        // A missing client with retained thread ids means the previous request
        // was cancelled while it owned the connection. Those ids cannot be
        // resumed safely on a newly spawned process without an explicit resume.
        runtime.threads.clear();
        let mut client = AppServerClient::spawn(cwd).await?;
        client.initialize(true).await?;
        runtime.client = Some(client);
    }

    // Lease the connection to this request. If the task is aborted while a turn
    // is active, dropping the owned client kills the process; the next request
    // observes `None`, clears stale thread ids, and reconstructs from ovim history.
    let mut client = runtime.client.take().expect("client initialized");

    let existing = runtime
        .threads
        .get(&runtime_key)
        .filter(|thread| thread.configuration == configuration)
        .cloned();
    let is_new = existing.is_none();
    let thread_id = if let Some(thread) = existing {
        thread.id
    } else {
        let id = client
            .start_thread(profile, instructions, cwd, tools, false)
            .await?;
        runtime.threads.insert(
            runtime_key,
            PersistentThread {
                id: id.clone(),
                configuration,
            },
        );
        id
    };

    let input = if is_new {
        initial_input
    } else {
        continuation_input
    };
    let result = async {
        client.start_turn(profile, &thread_id, input).await?;
        client.stream_turn(stream_tx, cwd).await
    }
    .await;
    if client.is_alive() {
        runtime.client = Some(client);
    } else {
        runtime.threads.clear();
    }
    result
}

fn request_cwd(file_path: Option<&str>) -> Result<PathBuf> {
    let current = std::env::current_dir().context("failed to determine Codex working directory")?;
    let Some(file_path) = file_path else {
        return Ok(current);
    };
    let path = Path::new(file_path);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        current.join(path)
    };
    let parent = absolute
        .parent()
        .filter(|parent| parent.exists())
        .unwrap_or(&current);
    Ok(parent
        .ancestors()
        .find(|candidate| candidate.join(".git").exists())
        .unwrap_or(parent)
        .to_path_buf())
}

struct AppServerClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_request_id: i64,
}

impl AppServerClient {
    async fn spawn(cwd: &Path) -> Result<Self> {
        let mut child = Command::new("codex")
            .args(["app-server", "--stdio"])
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context(
                "failed to start `codex app-server`; install Codex and run `codex login` first",
            )?;
        let stdin = child
            .stdin
            .take()
            .context("Codex app-server has no stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("Codex app-server has no stdout")?;
        if let Some(mut stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut discarded = Vec::new();
                let _ = stderr.read_to_end(&mut discarded).await;
            });
        }
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_request_id: 1,
        })
    }

    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn request_id(&mut self) -> i64 {
        let id = self.next_request_id;
        self.next_request_id += 1;
        id
    }

    async fn start_thread(
        &mut self,
        profile: &AiProfileConfig,
        instructions: &str,
        cwd: &Path,
        tools: &[Value],
        ephemeral: bool,
    ) -> Result<String> {
        let project_tools_enabled = !tools.is_empty();
        let tool_instruction = if project_tools_enabled {
            "Use only the ovim-provided dynamic tools when project context is needed. Do not use shell, apply_patch, built-in file tools, or any mutation tool. Ovim owns tool execution, edits, validation, and approvals."
        } else {
            "Do not run commands, use tools, or modify files. Return the requested answer only; ovim owns all tool execution, edits, validation, and approvals."
        };
        let mut params = json!({
            "model": profile.model,
            "cwd": cwd,
            "approvalPolicy": "never",
            "sandbox": "read-only",
            "ephemeral": ephemeral,
            "serviceName": "ovim",
            "developerInstructions": format!("{instructions}\n\n{tool_instruction}"),
        });
        if project_tools_enabled {
            params["dynamicTools"] = codex_dynamic_tool_specs(tools);
        }
        let request_id = self.request_id();
        self.send(json!({ "method": "thread/start", "id": request_id, "params": params }))
            .await?;
        let started = self.wait_for_response(request_id, None).await?;
        started
            .pointer("/result/thread/id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .context("Codex thread/start response did not include a thread id")
    }

    async fn start_turn(
        &mut self,
        profile: &AiProfileConfig,
        thread_id: &str,
        input: &str,
    ) -> Result<()> {
        let mut params = json!({
            "threadId": thread_id,
            "input": [{ "type": "text", "text": input }],
            "model": profile.model,
        });
        if let Some(effort) = profile.reasoning_effort.as_deref() {
            params["effort"] = json!(effort);
        }
        let request_id = self.request_id();
        self.send(json!({ "method": "turn/start", "id": request_id, "params": params }))
            .await?;
        self.wait_for_response(request_id, None).await?;
        Ok(())
    }

    async fn initialize(&mut self, experimental_api: bool) -> Result<()> {
        let mut params = json!({
            "clientInfo": {
                "name": "ovim",
                "title": "ovim",
                "version": CLIENT_VERSION,
            }
        });
        if experimental_api {
            params["capabilities"] = json!({ "experimentalApi": true });
        }
        self.send(json!({
            "method": "initialize",
            "id": 0,
            "params": params,
        }))
        .await?;
        self.wait_for_response(0, None).await?;
        self.send(json!({ "method": "initialized", "params": {} }))
            .await
    }

    async fn send(&mut self, value: Value) -> Result<()> {
        let mut encoded = serde_json::to_vec(&value)?;
        encoded.push(b'\n');
        self.stdin
            .write_all(&encoded)
            .await
            .context("failed to write to Codex app-server")?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn next_message(&mut self) -> Result<Value> {
        let mut line = String::new();
        let read = self
            .stdout
            .read_line(&mut line)
            .await
            .context("failed to read from Codex app-server")?;
        if read == 0 {
            bail!("Codex app-server closed the connection unexpectedly");
        }
        serde_json::from_str(line.trim_end())
            .with_context(|| format!("invalid JSON from Codex app-server: {}", line.trim_end()))
    }

    async fn wait_for_response(
        &mut self,
        id: i64,
        stream_tx: Option<&UnboundedSender<StreamChunk>>,
    ) -> Result<Value> {
        loop {
            let message = self.next_message().await?;
            if message.get("id").and_then(Value::as_i64) == Some(id) {
                if let Some(error) = message.get("error") {
                    let detail = error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown app-server error");
                    return Err(anyhow!("Codex app-server request failed: {detail}"));
                }
                return Ok(message);
            }
            emit_delta(&message, stream_tx);
        }
    }

    async fn stream_turn(
        &mut self,
        stream_tx: Option<UnboundedSender<StreamChunk>>,
        _cwd: &Path,
    ) -> Result<String> {
        let mut output = String::new();
        loop {
            let message = self.next_message().await?;
            if message.get("method").and_then(Value::as_str) == Some("item/tool/call") {
                self.respond_to_tool_call(&message, stream_tx.as_ref())
                    .await?;
                continue;
            }
            match message.get("method").and_then(Value::as_str) {
                Some("item/agentMessage/delta") => {
                    if let Some(delta) = message.pointer("/params/delta").and_then(Value::as_str) {
                        output.push_str(delta);
                        if let Some(tx) = stream_tx.as_ref() {
                            let _ = tx.send(StreamChunk::Content(delta.to_string()));
                        }
                    }
                }
                Some("item/reasoning/summaryTextDelta") => {
                    if let (Some(tx), Some(delta)) = (
                        stream_tx.as_ref(),
                        message.pointer("/params/delta").and_then(Value::as_str),
                    ) {
                        let _ = tx.send(StreamChunk::Thinking(delta.to_string()));
                    }
                }
                Some("item/completed")
                    if output.is_empty()
                        && message.pointer("/params/item/type").and_then(Value::as_str)
                            == Some("agentMessage") =>
                {
                    if let Some(text) = message.pointer("/params/item/text").and_then(Value::as_str)
                    {
                        output.push_str(text);
                        if let Some(tx) = stream_tx.as_ref() {
                            let _ = tx.send(StreamChunk::Content(text.to_string()));
                        }
                    }
                }
                Some("error") => {
                    let detail = message
                        .pointer("/params/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("Codex turn failed");
                    bail!("{detail}");
                }
                Some("turn/completed") => {
                    let status = message
                        .pointer("/params/turn/status")
                        .and_then(Value::as_str)
                        .unwrap_or("failed");
                    if status != "completed" {
                        let detail = message
                            .pointer("/params/turn/error/message")
                            .and_then(Value::as_str)
                            .unwrap_or(status);
                        bail!("Codex turn {status}: {detail}");
                    }
                    return Ok(output);
                }
                _ => {}
            }
        }
    }

    async fn stop(&mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }

    async fn respond_to_tool_call(
        &mut self,
        message: &Value,
        stream_tx: Option<&UnboundedSender<StreamChunk>>,
    ) -> Result<()> {
        let id = message
            .get("id")
            .cloned()
            .context("dynamic tool call has no id")?;
        let tool = message
            .pointer("/params/tool")
            .and_then(Value::as_str)
            .unwrap_or("");
        let arguments = message.pointer("/params/arguments").unwrap_or(&Value::Null);
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        let Some(stream_tx) = stream_tx else {
            bail!("Codex requested tool '{tool}' outside an interactive chat");
        };
        stream_tx
            .send(StreamChunk::DynamicToolRequest {
                call: super::ToolCallInfo {
                    id: id
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| id.to_string()),
                    name: tool.to_string(),
                    arguments: arguments.clone(),
                },
                response: response_tx,
            })
            .map_err(|_| anyhow!("ovim editor closed while executing '{tool}'"))?;
        let result = response_rx
            .await
            .map_err(|_| anyhow!("ovim dropped the response for '{tool}'"))?;
        let (text, success) = match result {
            Ok(text) => (text, true),
            Err(error) => (error, false),
        };
        self.send(json!({
            "id": id,
            "result": {
                "contentItems": [{ "type": "inputText", "text": text }],
                "success": success,
            }
        }))
        .await
    }
}

fn codex_dynamic_tool_specs(tools: &[Value]) -> Value {
    Value::Array(
        tools
            .iter()
            .filter_map(|tool| {
                let function = tool.get("function")?;
                Some(json!({
                    "type": "function",
                    "name": function.get("name")?,
                    "description": function.get("description").cloned().unwrap_or(Value::Null),
                    "inputSchema": function.get("parameters").cloned().unwrap_or_else(|| json!({"type": "object"})),
                }))
            })
            .collect(),
    )
}

#[cfg(test)]
fn project_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let relative = Path::new(relative);
    if relative.is_absolute() || super::path_policy::has_parent_traversal(relative) {
        bail!("path must be relative to the project root");
    }
    let root = super::path_policy::canonicalize_or_normalize(root);
    let path = super::path_policy::canonicalize_or_normalize(&root.join(relative));
    if !path.starts_with(&root) {
        bail!("path is outside the project root");
    }
    if let Some(reason) = super::path_policy::sensitive_path_reason(&path) {
        bail!("{reason}");
    }
    Ok(path)
}

#[cfg(test)]
fn list_project_files(root: &Path, arguments: &Value) -> Result<String> {
    let limit = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(200) as usize;
    let mut paths = Vec::new();
    for entry in WalkBuilder::new(root).hidden(false).build().flatten() {
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.path();
        if super::path_policy::is_sensitive_path(path) {
            continue;
        }
        if let Ok(relative) = path.strip_prefix(root) {
            paths.push(relative.to_string_lossy().to_string());
            if paths.len() >= limit.min(500) {
                break;
            }
        }
    }
    paths.sort();
    Ok(paths.join("\n"))
}

#[cfg(test)]
fn read_project_file(root: &Path, arguments: &Value) -> Result<String> {
    let relative = arguments
        .get("path")
        .and_then(Value::as_str)
        .context("path is required")?;
    let path = project_path(root, relative)?;
    let metadata = std::fs::metadata(&path).with_context(|| format!("cannot read {relative}"))?;
    if metadata.len() > 512 * 1024 {
        bail!("file is larger than 512 KiB");
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("{relative} is not readable UTF-8 text"))?;
    let start = arguments
        .get("start_line")
        .and_then(Value::as_u64)
        .unwrap_or(1) as usize;
    let end = arguments
        .get("end_line")
        .and_then(Value::as_u64)
        .map(|line| line as usize)
        .unwrap_or(usize::MAX);
    if end < start {
        bail!("end_line must be greater than or equal to start_line");
    }
    Ok(content
        .lines()
        .enumerate()
        .filter(|(index, _)| index + 1 >= start && index + 1 <= end)
        .map(|(index, line)| format!("{:>6}  {line}", index + 1))
        .collect::<Vec<_>>()
        .join("\n"))
}

#[cfg(test)]
fn search_project(root: &Path, arguments: &Value) -> Result<String> {
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .filter(|query| !query.is_empty())
        .context("query is required")?;
    let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
    let mut matches = Vec::new();
    for entry in WalkBuilder::new(root).hidden(false).build().flatten() {
        let path = entry.path();
        if !entry.file_type().is_some_and(|kind| kind.is_file())
            || super::path_policy::is_sensitive_path(path)
            || std::fs::metadata(path).is_ok_and(|metadata| metadata.len() > 512 * 1024)
        {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        for (line_index, line) in content.lines().enumerate() {
            if line.contains(query) {
                let relative = path.strip_prefix(root).unwrap_or(path);
                matches.push(format!("{}:{}: {line}", relative.display(), line_index + 1));
                if matches.len() >= limit.min(200) {
                    return Ok(matches.join("\n"));
                }
            }
        }
    }
    Ok(matches.join("\n"))
}

fn emit_delta(message: &Value, stream_tx: Option<&UnboundedSender<StreamChunk>>) {
    let Some(tx) = stream_tx else { return };
    if message.get("method").and_then(Value::as_str) == Some("warning") {
        if let Some(text) = message.pointer("/params/message").and_then(Value::as_str) {
            let _ = tx.send(StreamChunk::Thinking(format!("Codex warning: {text}")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn converts_ovim_openai_schema_to_codex_dynamic_tool() {
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "open_file",
                "description": "Open a file in ovim.",
                "parameters": {
                    "type": "object",
                    "required": ["path"],
                    "properties": { "path": { "type": "string" } }
                }
            }
        })];

        let converted = codex_dynamic_tool_specs(&tools);
        assert_eq!(converted[0]["name"], "open_file");
        assert_eq!(converted[0]["inputSchema"]["required"][0], "path");
        assert!(converted[0].get("function").is_none());
    }

    #[test]
    fn relative_file_uses_existing_parent() {
        let cwd = std::env::current_dir().unwrap();
        let result = request_cwd(Some("src/lib.rs")).unwrap();
        let expected_repo = cwd
            .ancestors()
            .find(|candidate| candidate.join(".git").exists());
        if let Some(expected_repo) = expected_repo {
            assert_eq!(result, expected_repo);
        } else {
            assert!(result == cwd || result == cwd.join("src"));
        }
    }

    #[test]
    fn project_tools_stay_inside_root_and_block_secrets() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "hello ovim\n").unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET=value\n").unwrap();

        let listed = list_project_files(dir.path(), &json!({})).unwrap();
        assert!(listed.contains("README.md"));
        assert!(!listed.contains(".env"));
        assert!(read_project_file(dir.path(), &json!({ "path": "../outside" })).is_err());
        assert!(read_project_file(dir.path(), &json!({ "path": ".env" })).is_err());
    }

    #[test]
    fn project_search_and_line_ranges_are_bounded() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("notes.txt"),
            "zero\nneedle one\nneedle two\n",
        )
        .unwrap();

        let read = read_project_file(
            dir.path(),
            &json!({ "path": "notes.txt", "start_line": 2, "end_line": 2 }),
        )
        .unwrap();
        assert_eq!(read, "     2  needle one");
        let found = search_project(dir.path(), &json!({ "query": "needle", "limit": 1 })).unwrap();
        assert_eq!(found.lines().count(), 1);
        assert!(found.contains("notes.txt:2"));
    }
}
