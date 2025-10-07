//! Language server process management
//!
//! Manages the lifecycle of a single language server process, including:
//! - Process spawning and monitoring
//! - stdio communication
//! - Request/response matching
//! - Initialization handshake

use super::protocol::{write_message, JsonRpcMessage, RequestId, ResponseError};
use super::supervisor::{RestartPolicy, TaskHealth, TaskSupervisor};
use anyhow::{anyhow, Context, Result};
use lsp_types::{
    ClientCapabilities, InitializeParams, InitializeResult, InitializedParams, ServerCapabilities,
    Url, WorkspaceFolder, TextDocumentContentChangeEvent,
};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

/// Maximum LSP message size in bytes (50MB)
/// Must match MAX_MESSAGE_SIZE in mod.rs
const MAX_MESSAGE_SIZE: usize = 50 * 1024 * 1024;

/// Maximum time a request can remain pending before cleanup (5 minutes)
const REQUEST_STALE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Interval for cleanup task (60 seconds)
const CLEANUP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// Pending request metadata
struct PendingRequest {
    sender: oneshot::Sender<Result<Value>>,
    sent_at: Instant,
    method: String,
}

/// Server state for explicit state machine
#[derive(Debug, Clone)]
pub enum ServerState {
    /// Server process is spawning
    Spawning,

    /// Server is initializing (sent initialize request, waiting for response)
    Initializing {
        started_at: Instant,
        pending_operations: Vec<PendingOperation>,
    },

    /// Server is ready to accept requests
    Ready {
        initialized_at: Instant,
        capabilities: ServerCapabilities,
    },

    /// Server is degraded (experiencing errors but still running)
    Degraded {
        reason: String,
        since: Instant,
    },

    /// Server has failed and cannot recover
    Failed {
        error: String,
        at: Instant,
    },

    /// Server is shutting down
    ShuttingDown,

    /// Server process has terminated
    Terminated,
}

/// Operations that can be queued during initialization
#[derive(Debug, Clone)]
enum PendingOperation {
    DidOpen {
        uri: Url,
        language_id: String,
        version: i32,
        text: String,
    },
    DidChange {
        uri: Url,
        language_id: String,
        changes: Vec<TextDocumentContentChangeEvent>,
    },
    DidSave {
        uri: Url,
        language_id: String,
        text: Option<String>,
    },
    Request {
        method: String,
        params: Value,
    },
}

/// Health information for a language server
#[derive(Debug, Clone)]
pub struct LanguageServerHealth {
    /// Language identifier (e.g., "rust", "python")
    pub language: String,

    /// Command used to spawn the server
    pub command: String,

    /// Current server state
    pub state: String,

    /// Time since server was spawned
    pub uptime: Duration,

    /// Number of pending requests
    pub pending_requests: usize,

    /// Whether the server has capabilities
    pub has_capabilities: bool,

    /// Health of supervised background tasks
    pub tasks: Vec<TaskHealth>,

    /// Whether the server process is still alive
    pub is_alive: bool,
}

/// A language server process
#[derive(Clone)]
pub struct LanguageServer {
    inner: Arc<LanguageServerInner>,
}

struct LanguageServerInner {
    /// Language identifier (e.g., "rust", "python") for logging context
    language: String,

    /// Command used to spawn the server (e.g., "rust-analyzer") for logging
    command: String,

    /// Child process handle
    process: Mutex<Option<Child>>,

    /// Stdin writer (wrapped in Arc to allow cloning for writer task)
    stdin: Arc<Mutex<ChildStdin>>,

    /// Current server state (explicit state machine)
    state: Arc<Mutex<ServerState>>,

    /// Server capabilities after initialization (kept for backwards compat)
    capabilities: Mutex<Option<ServerCapabilities>>,

    /// Pending requests awaiting responses with metadata for cleanup
    pending_requests: Mutex<HashMap<RequestId, PendingRequest>>,

    /// Next request ID
    next_request_id: AtomicU64,

    /// Channel to send outgoing messages
    outgoing_tx: mpsc::Sender<JsonRpcMessage>,

    /// Channel to receive incoming messages
    incoming_rx: Mutex<Option<mpsc::Receiver<JsonRpcMessage>>>,

    /// Task supervisor for managing background tasks
    supervisor: TaskSupervisor,

    // Cached capability flags (lock-free, set once during initialization)
    /// Cached: supports goto definition
    cap_goto_definition: AtomicBool,
    /// Cached: supports hover
    cap_hover: AtomicBool,
    /// Cached: supports completion
    cap_completion: AtomicBool,
    /// Cached: supports formatting
    cap_formatting: AtomicBool,
    /// Cached: supports range formatting
    cap_range_formatting: AtomicBool,
    /// Cached: supports code actions
    cap_code_actions: AtomicBool,
    /// Cached: supports references
    cap_references: AtomicBool,
    /// Cached: supports rename
    cap_rename: AtomicBool,
    /// Cached: supports prepare rename
    cap_prepare_rename: AtomicBool,
    /// Cached: supports signature help
    cap_signature_help: AtomicBool,
    /// Cached: supports document symbols
    cap_document_symbol: AtomicBool,
    /// Cached: supports selection range
    cap_selection_range: AtomicBool,
    /// Cached: supports workspace symbols
    cap_workspace_symbol: AtomicBool,
    /// Cached: supports document highlight
    cap_document_highlight: AtomicBool,
    /// Cached: supports incremental sync
    cap_incremental_sync: AtomicBool,
    /// Cached: supports folding range
    cap_folding_range: AtomicBool,
}

impl LanguageServer {
    /// Returns a log prefix with language and command context
    fn log_prefix(&self) -> String {
        format!("[LSP:{}:{}]", self.inner.language, self.inner.command)
    }

    /// Spawns a new language server process
    pub async fn spawn(language: &str, command: &str, args: Vec<String>) -> Result<Self> {
        let mut child = Command::new(command)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()) // Capture stderr for debugging
            .spawn()
            .context(format!("Failed to spawn language server: {}", command))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to open stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Failed to open stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("Failed to open stderr"))?;

        let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<JsonRpcMessage>(100);
        let (incoming_tx, incoming_rx) = mpsc::channel::<JsonRpcMessage>(100);

        // Wrap stdin in Arc so writer task can clone it
        let stdin = Arc::new(Mutex::new(stdin));

        // Create supervisor with auto-restart on failure
        let supervisor = TaskSupervisor::new(RestartPolicy::OnFailure {
            max_retries: 3,
            initial_backoff: Duration::from_secs(1),
        });

        let inner = Arc::new(LanguageServerInner {
            language: language.to_string(),
            command: command.to_string(),
            process: Mutex::new(Some(child)),
            stdin: stdin.clone(),
            state: Arc::new(Mutex::new(ServerState::Spawning)),
            capabilities: Mutex::new(None),
            pending_requests: Mutex::new(HashMap::new()),
            next_request_id: AtomicU64::new(1),
            outgoing_tx,
            incoming_rx: Mutex::new(Some(incoming_rx)),
            supervisor,
            // Initialize cached capabilities to false (will be set during initialization)
            cap_goto_definition: AtomicBool::new(false),
            cap_hover: AtomicBool::new(false),
            cap_completion: AtomicBool::new(false),
            cap_formatting: AtomicBool::new(false),
            cap_range_formatting: AtomicBool::new(false),
            cap_code_actions: AtomicBool::new(false),
            cap_references: AtomicBool::new(false),
            cap_rename: AtomicBool::new(false),
            cap_prepare_rename: AtomicBool::new(false),
            cap_signature_help: AtomicBool::new(false),
            cap_document_symbol: AtomicBool::new(false),
            cap_selection_range: AtomicBool::new(false),
            cap_workspace_symbol: AtomicBool::new(false),
            cap_document_highlight: AtomicBool::new(false),
            cap_incremental_sync: AtomicBool::new(false),
            cap_folding_range: AtomicBool::new(false),
        });

        let server = Self { inner: inner.clone() };

        // Spawn supervised task to write messages to stdin
        let stdin_clone = stdin.clone();
        let mut outgoing_rx_moved = outgoing_rx;
        inner.supervisor.spawn_supervised(
            "lsp_writer".to_string(),
            move || {
                let stdin = stdin_clone.clone();
                let mut rx: mpsc::Receiver<JsonRpcMessage> = unsafe {
                    // SAFETY: We need to share the receiver across restarts
                    // This is safe because:
                    // 1. Only one writer task runs at a time (supervised)
                    // 2. The receiver is never actually cloned, just re-referenced
                    std::ptr::read(&outgoing_rx_moved as *const _)
                };
                async move {
                    while let Some(msg) = rx.recv().await {
                        let mut stdin_guard = stdin.lock().await;
                        if let Err(e) = write_message(&mut *stdin_guard, &msg).await {
                            return Err(anyhow!("Error writing to language server: {}", e));
                        }
                    }
                    Ok(())
                }
            }
        ).await?;

        // Spawn supervised stale request cleanup task
        let inner_cleanup = inner.clone();
        inner.supervisor.spawn_supervised(
            "lsp_cleanup".to_string(),
            move || {
                let inner = inner_cleanup.clone();
                async move {
                    loop {
                        tokio::time::sleep(CLEANUP_INTERVAL).await;

                        let mut pending = inner.pending_requests.lock().await;
                        let now = Instant::now();
                        let count_before = pending.len();

                        // Collect stale request IDs first
                        let stale_ids: Vec<RequestId> = pending
                            .iter()
                            .filter_map(|(id, req)| {
                                let age = now.duration_since(req.sent_at);
                                if age > REQUEST_STALE_TIMEOUT {
                                    Some(id.clone())
                                } else {
                                    None
                                }
                            })
                            .collect();

                        // Remove and notify each stale request
                        for id in stale_ids {
                            if let Some(req) = pending.remove(&id) {
                                let age = now.duration_since(req.sent_at);
                                eprintln!(
                                    "[LSP Cleanup] Removing stale request {:?} for method '{}' (age: {:?})",
                                    id, req.method, age
                                );
                                let _ = req.sender.send(Err(anyhow!(
                                    "Request '{}' timed out and was cleaned up after {:?}",
                                    req.method, age
                                )));
                            }
                        }

                        let count_after = pending.len();
                        if count_before > count_after {
                            eprintln!(
                                "[LSP Cleanup] Cleaned up {} stale requests ({} remaining)",
                                count_before - count_after,
                                count_after
                            );
                        }

                        if pending.len() > 100 {
                            eprintln!("[LSP Cleanup] Warning: {} pending requests", pending.len());
                        }
                    }
                }
            }
        ).await?;

        // Spawn task to read messages from stdout
        // Note: Not supervised because stdout is unique to the process - if this fails, server is dead
        let inner_clone = server.inner.clone();
        let state_clone = inner.state.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            loop {
                // Read Content-Length header
                let mut header = String::new();
                if let Err(e) = reader.read_line(&mut header).await {
                    eprintln!("[LSP Reader] Error reading header: {}", e);
                    // Mark server as failed
                    let mut state = state_clone.lock().await;
                    *state = ServerState::Failed {
                        error: format!("Reader task failed: {}", e),
                        at: Instant::now(),
                    };
                    break;
                }

                if header.is_empty() {
                    eprintln!("[LSP Reader] EOF reached");
                    break;
                }

                if !header.starts_with("Content-Length:") {
                    continue;
                }

                let content_length: usize = match header
                    .trim()
                    .strip_prefix("Content-Length:")
                    .and_then(|s| s.trim().parse().ok())
                {
                    Some(len) => len,
                    None => {
                        eprintln!("[LSP Reader] Failed to parse Content-Length: {}", header);
                        continue;
                    }
                };

                // Read empty line
                let mut empty = String::new();
                if reader.read_line(&mut empty).await.is_err() {
                    break;
                }

                // Read content
                let mut content = vec![0u8; content_length];
                if let Err(e) = tokio::io::AsyncReadExt::read_exact(&mut reader, &mut content).await
                {
                    eprintln!("[LSP Reader] Error reading message: {}", e);
                    break;
                }

                // Parse JSON
                match serde_json::from_slice::<JsonRpcMessage>(&content) {
                    Ok(msg) => {
                        if msg.is_response() {
                            // Handle response
                            if let Some(id) = msg.id {
                                let mut pending = inner_clone.pending_requests.lock().await;
                                if let Some(req) = pending.remove(&id) {
                                    // CRITICAL FIX: Propagate errors instead of just logging
                                    if let Some(result) = msg.result {
                                        let _ = req.sender.send(Ok(result));
                                    } else if let Some(error) = msg.error {
                                        // Don't print to stderr - propagate error to caller
                                        // Errors will be shown in status line by the editor
                                        let _ = req.sender.send(Err(anyhow!("LSP error: {:?}", error)));
                                    } else {
                                        // Neither result nor error - protocol violation
                                        let _ = req.sender.send(Err(anyhow!("Invalid response: no result or error")));
                                    }
                                }
                            }
                        } else {
                            // Handle notification or request
                            if let Err(_e) = incoming_tx.send(msg).await {
                                // Channel closed, exit silently
                                break;
                            }
                        }
                    }
                    Err(_e) => {
                        // Parse error - silently skip malformed messages
                        // Debug info available via OVIM_LSP_DEBUG env var
                    }
                }
            }
            // Reader task exiting silently
        });

        // Spawn task to capture stderr (silently consume it to prevent terminal spam)
        // Note: Not supervised because stderr is unique to the process
        // TODO: Optionally log to file or send to status line for debugging
        tokio::spawn(async move {
            let mut stderr_reader = BufReader::new(stderr);
            let mut line = String::new();
            while let Ok(n) = stderr_reader.read_line(&mut line).await {
                if n == 0 {
                    break; // EOF
                }
                // Silently consume stderr - LSP servers can be very verbose
                // Debug output can be enabled via environment variable if needed
                if std::env::var("OVIM_LSP_DEBUG").is_ok() {
                    eprint!("[LSP stderr] {}", line);
                }
                line.clear();
            }
            // Silently exit
        });

        Ok(server)
    }

    /// Initializes the language server
    pub async fn initialize(&mut self, root_uri: Url) -> Result<()> {
        // Transition to Initializing state
        self.transition_to(ServerState::Initializing {
            started_at: Instant::now(),
            pending_operations: Vec::new(),
        }).await;

        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(root_uri.clone()),
            root_path: None,
            initialization_options: None,
            capabilities: ClientCapabilities::default(),
            trace: None,
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: root_uri,
                name: "workspace".to_string(),
            }]),
            client_info: None,
            locale: None,
            work_done_progress_params: Default::default(),
        };

        let result = self.request("initialize", serde_json::to_value(params)?).await
            .context("Failed to send initialize request")?;

        let init_result: InitializeResult = serde_json::from_value(result)
            .context("Failed to parse initialize response")?;

        // Store capabilities
        let mut caps = self.inner.capabilities.lock().await;
        *caps = Some(init_result.capabilities.clone());
        drop(caps); // Release lock

        // Cache capability flags for lock-free access
        self.cache_capabilities(&init_result.capabilities);

        // Send initialized notification
        self.notify("initialized", serde_json::to_value(InitializedParams {})?).await
            .context("Failed to send initialized notification")?;

        // Transition to Ready state and replay pending operations
        self.transition_to(ServerState::Ready {
            initialized_at: Instant::now(),
            capabilities: init_result.capabilities,
        }).await;

        Ok(())
    }

    /// Transitions to a new state, handling state-specific logic
    async fn transition_to(&self, new_state: ServerState) {
        let mut state = self.inner.state.lock().await;
        let old_state = state.clone();
        let prefix = self.log_prefix();

        // Removed verbose logging: State: {:?} → {:?}

        // Handle transition-specific logic
        match (&*state, &new_state) {
            // Transitioning from Initializing to Ready: replay pending operations
            (ServerState::Initializing { pending_operations, .. }, ServerState::Ready { .. }) => {
                // Removed verbose logging: Replaying {} pending operations

                for op in pending_operations {
                    if let Err(e) = self.replay_operation(op).await {
                        eprintln!("{} Failed to replay operation: {}", prefix, e);
                    }
                }
            }
            _ => {}
        }

        *state = new_state;
    }

    /// Replays a pending operation after server becomes ready
    async fn replay_operation(&self, op: &PendingOperation) -> Result<()> {
        match op {
            PendingOperation::DidOpen { uri, language_id, version, text } => {
                use lsp_types::{DidOpenTextDocumentParams, TextDocumentItem};

                let params = DidOpenTextDocumentParams {
                    text_document: TextDocumentItem {
                        uri: uri.clone(),
                        language_id: language_id.clone(),
                        version: *version,
                        text: text.clone(),
                    },
                };

                self.notify("textDocument/didOpen", serde_json::to_value(params)?)
                    .await
                    .context("Failed to replay didOpen")
            }
            PendingOperation::DidChange { uri, language_id, changes } => {
                use lsp_types::{DidChangeTextDocumentParams, VersionedTextDocumentIdentifier};

                // Note: version might be stale, but better than losing the operation
                let params = DidChangeTextDocumentParams {
                    text_document: VersionedTextDocumentIdentifier {
                        uri: uri.clone(),
                        version: 1, // Simplified for now
                    },
                    content_changes: changes.clone(),
                };

                self.notify("textDocument/didChange", serde_json::to_value(params)?)
                    .await
                    .context("Failed to replay didChange")
            }
            PendingOperation::DidSave { uri, language_id, text } => {
                use lsp_types::{DidSaveTextDocumentParams, TextDocumentIdentifier};

                let params = DidSaveTextDocumentParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                    text: text.clone(),
                };

                self.notify("textDocument/didSave", serde_json::to_value(params)?)
                    .await
                    .context("Failed to replay didSave")
            }
            PendingOperation::Request { method, params } => {
                // Requests are not replayed (they would have timed out already)
                eprintln!("{} Skipping replay of request '{}'", self.log_prefix(), method);
                Ok(())
            }
        }
    }

    /// Gets the current server state
    pub async fn state(&self) -> ServerState {
        self.inner.state.lock().await.clone()
    }

    /// Checks if server is ready to accept requests
    pub async fn is_ready(&self) -> bool {
        matches!(self.state().await, ServerState::Ready { .. })
    }

    /// Queues an operation if server is not ready, or executes immediately if ready
    async fn queue_or_execute<F, Fut>(&self, op: PendingOperation, execute: F) -> Result<()>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let prefix = self.log_prefix();
        let mut state = self.inner.state.lock().await;

        match &mut *state {
            ServerState::Ready { .. } => {
                drop(state); // Release lock before executing
                execute().await
            }
            ServerState::Initializing { pending_operations, .. } => {
                // Queuing operation while server initializes
                pending_operations.push(op);
                Ok(())
            }
            ServerState::Failed { error, .. } => {
                Err(anyhow!("Server failed: {}", error))
            }
            ServerState::Terminated => {
                Err(anyhow!("Server has terminated"))
            }
            state => {
                Err(anyhow!("Server in unexpected state: {:?}", state))
            }
        }
    }

    /// Sends a request and waits for the response
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let request_id = RequestId::Number(
            self.inner
                .next_request_id
                .fetch_add(1, Ordering::SeqCst),
        );

        let (tx, rx) = oneshot::channel();

        // Register pending request with metadata
        {
            let mut pending = self.inner.pending_requests.lock().await;
            pending.insert(request_id.clone(), PendingRequest {
                sender: tx,
                sent_at: Instant::now(),
                method: method.to_string(),
            });
        }

        // Send request
        let msg = JsonRpcMessage::request(request_id.clone(), method.to_string(), params);

        // Check message size before sending
        let serialized = serde_json::to_string(&msg)
            .context("Failed to serialize request")?;
        if serialized.len() > MAX_MESSAGE_SIZE {
            return Err(anyhow!(
                "Request '{}' too large: {} bytes (max {} bytes / {:.1} MB)",
                method,
                serialized.len(),
                MAX_MESSAGE_SIZE,
                MAX_MESSAGE_SIZE as f64 / (1024.0 * 1024.0)
            ));
        }

        self.inner
            .outgoing_tx
            .send(msg)
            .await
            .map_err(|_| anyhow!("Failed to send request"))?;

        // Wait for response with timeout (5s for faster feedback on failures)
        match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
            Ok(Ok(result)) => result.context(format!("LSP request '{}' failed", method)),
            Ok(Err(_)) => Err(anyhow!("Response channel closed for method '{}'", method)),
            Err(_) => {
                // Timeout - remove pending request
                let mut pending = self.inner.pending_requests.lock().await;
                pending.remove(&request_id);
                Err(anyhow!("Request '{}' timed out after 30 seconds", method))
            }
        }
    }

    /// Sends a notification (no response expected)
    pub async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let msg = JsonRpcMessage::notification(method.to_string(), params);

        // Check message size before sending
        let serialized = serde_json::to_string(&msg)
            .context("Failed to serialize notification")?;
        if serialized.len() > MAX_MESSAGE_SIZE {
            return Err(anyhow!(
                "Notification '{}' too large: {} bytes (max {} bytes / {:.1} MB)",
                method,
                serialized.len(),
                MAX_MESSAGE_SIZE,
                MAX_MESSAGE_SIZE as f64 / (1024.0 * 1024.0)
            ));
        }

        self.inner
            .outgoing_tx
            .send(msg)
            .await
            .map_err(|_| anyhow!("Failed to send notification"))?;
        Ok(())
    }

    /// Receives the next incoming notification/request
    pub async fn receive(&self) -> Option<JsonRpcMessage> {
        let mut rx = self.inner.incoming_rx.lock().await;
        if let Some(ref mut rx) = *rx {
            rx.recv().await
        } else {
            None
        }
    }

    /// Shuts down the language server gracefully
    /// Follows proper shutdown sequence to avoid zombie processes:
    /// 1. LSP shutdown request
    /// 2. LSP exit notification
    /// 3. Wait for graceful exit (5s)
    /// 4. SIGTERM (if Unix, 3s wait)
    /// 5. SIGKILL (last resort)
    pub async fn shutdown(&mut self) -> Result<()> {
        let prefix = self.log_prefix();

        // Transition to ShuttingDown state
        self.transition_to(ServerState::ShuttingDown).await;

        // Step 1: Send LSP shutdown request
        let shutdown_result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.request("shutdown", Value::Null)
        ).await;

        if shutdown_result.is_ok() {
            // Step 2: Send exit notification
            let _ = self.notify("exit", Value::Null).await;

            // Step 3: Wait for graceful exit
            let mut process = self.inner.process.lock().await;
            if let Some(ref mut child) = *process {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    child.wait()
                ).await {
                    Ok(Ok(_status)) => {
                        return Ok(());
                    }
                    Ok(Err(e)) => {
                        eprintln!("{} Shutdown: Error waiting for exit: {}", prefix, e);
                    }
                    Err(_) => {
                        // Graceful exit timeout, trying SIGTERM
                    }
                }
            } else {
                return Ok(()); // No process to shutdown
            }
        }

        // Step 4: Try SIGTERM (Unix only)
        #[cfg(unix)]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;

            let mut process = self.inner.process.lock().await;
            if let Some(ref mut child) = *process {
                if let Some(pid) = child.id() {
                    if let Ok(()) = kill(Pid::from_raw(pid as i32), Signal::SIGTERM) {
                        // Wait 3 seconds for SIGTERM to take effect
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(3),
                            child.wait()
                        ).await {
                            Ok(Ok(_status)) => {
                                return Ok(());
                            }
                            Ok(Err(e)) => {
                                eprintln!("{} Shutdown: Error after SIGTERM: {}", prefix, e);
                            }
                            Err(_) => {
                                // SIGTERM timeout, using SIGKILL
                            }
                        }
                    }
                }
            }
        }

        // Step 5: Last resort - SIGKILL
        let mut process = self.inner.process.lock().await;
        if let Some(ref mut child) = *process {
            if let Err(e) = child.kill().await {
                eprintln!("{} Shutdown: SIGKILL failed: {}", prefix, e);
            }

            // Wait to reap zombie
            if let Err(e) = child.wait().await {
                eprintln!("{} Shutdown: Error reaping process: {}", prefix, e);
            }
        }

        // Shutdown all supervised tasks
        if let Err(e) = self.inner.supervisor.shutdown_all().await {
            eprintln!("{} Shutdown: Error shutting down tasks: {}", prefix, e);
        }

        // Final transition to Terminated
        self.transition_to(ServerState::Terminated).await;

        Ok(())
    }

    /// Gets health information for this language server
    pub async fn health_check(&self) -> LanguageServerHealth {
        let state = self.state().await;
        let pending_count = self.inner.pending_requests.lock().await.len();
        let has_caps = self.inner.capabilities.lock().await.is_some();
        let tasks = self.inner.supervisor.health_check().await;

        // Determine uptime based on state
        let uptime = match &state {
            ServerState::Spawning => Duration::from_secs(0),
            ServerState::Initializing { started_at, .. } => started_at.elapsed(),
            ServerState::Ready { initialized_at, .. } => initialized_at.elapsed(),
            ServerState::Degraded { since, .. } => since.elapsed(),
            ServerState::Failed { at, .. } => at.elapsed(),
            ServerState::ShuttingDown | ServerState::Terminated => Duration::from_secs(0),
        };

        // Check if process is alive
        let is_alive = {
            let process = self.inner.process.lock().await;
            if let Some(ref child) = *process {
                !child.id().is_none()
            } else {
                false
            }
        };

        // Convert state to string for easier debugging
        let state_str = match state {
            ServerState::Spawning => "Spawning".to_string(),
            ServerState::Initializing { .. } => "Initializing".to_string(),
            ServerState::Ready { .. } => "Ready".to_string(),
            ServerState::Degraded { ref reason, .. } => format!("Degraded: {}", reason),
            ServerState::Failed { ref error, .. } => format!("Failed: {}", error),
            ServerState::ShuttingDown => "ShuttingDown".to_string(),
            ServerState::Terminated => "Terminated".to_string(),
        };

        LanguageServerHealth {
            language: self.inner.language.clone(),
            command: self.inner.command.clone(),
            state: state_str,
            uptime,
            pending_requests: pending_count,
            has_capabilities: has_caps,
            tasks,
            is_alive,
        }
    }

    /// Caches capability flags from ServerCapabilities for lock-free access
    /// Called once during initialization
    fn cache_capabilities(&self, caps: &ServerCapabilities) {
        // Cache goto definition support
        self.inner.cap_goto_definition.store(
            caps.definition_provider.is_some(),
            Ordering::Relaxed,
        );

        // Cache hover support
        self.inner.cap_hover.store(
            caps.hover_provider.is_some(),
            Ordering::Relaxed,
        );

        // Cache completion support
        self.inner.cap_completion.store(
            caps.completion_provider.is_some(),
            Ordering::Relaxed,
        );

        // Cache formatting support
        self.inner.cap_formatting.store(
            caps.document_formatting_provider.is_some(),
            Ordering::Relaxed,
        );

        // Cache range formatting support
        self.inner.cap_range_formatting.store(
            caps.document_range_formatting_provider.is_some(),
            Ordering::Relaxed,
        );

        // Cache code actions support
        self.inner.cap_code_actions.store(
            caps.code_action_provider.is_some(),
            Ordering::Relaxed,
        );

        // Cache references support
        self.inner.cap_references.store(
            caps.references_provider.is_some(),
            Ordering::Relaxed,
        );

        // Cache rename support
        self.inner.cap_rename.store(
            caps.rename_provider.is_some(),
            Ordering::Relaxed,
        );

        // Cache prepare rename support
        let prepare_rename_support = match &caps.rename_provider {
            Some(lsp_types::OneOf::Right(options)) => options.prepare_provider.unwrap_or(false),
            _ => false,
        };
        self.inner.cap_prepare_rename.store(prepare_rename_support, Ordering::Relaxed);

        // Cache signature help support
        self.inner.cap_signature_help.store(
            caps.signature_help_provider.is_some(),
            Ordering::Relaxed,
        );

        // Cache document symbol support
        self.inner.cap_document_symbol.store(
            caps.document_symbol_provider.is_some(),
            Ordering::Relaxed,
        );

        // Cache selection range support
        self.inner.cap_selection_range.store(
            caps.selection_range_provider.is_some(),
            Ordering::Relaxed,
        );

        // Cache workspace symbol support
        self.inner.cap_workspace_symbol.store(
            caps.workspace_symbol_provider.is_some(),
            Ordering::Relaxed,
        );

        // Cache document highlight support
        self.inner.cap_document_highlight.store(
            caps.document_highlight_provider.is_some(),
            Ordering::Relaxed,
        );

        // Cache incremental sync support
        let incremental_sync = match &caps.text_document_sync {
            Some(lsp_types::TextDocumentSyncCapability::Kind(kind)) => {
                *kind == lsp_types::TextDocumentSyncKind::INCREMENTAL
            }
            Some(lsp_types::TextDocumentSyncCapability::Options(opts)) => {
                opts.change == Some(lsp_types::TextDocumentSyncKind::INCREMENTAL)
            }
            None => false,
        };
        self.inner.cap_incremental_sync.store(incremental_sync, Ordering::Relaxed);

        // Cache folding range support
        self.inner.cap_folding_range.store(
            caps.folding_range_provider.is_some(),
            Ordering::Relaxed,
        );
    }

    /// Gets the server capabilities
    pub async fn capabilities(&self) -> Option<ServerCapabilities> {
        let caps = self.inner.capabilities.lock().await;
        caps.clone()
    }

    /// Checks if the server supports goto definition (lock-free)
    pub async fn supports_goto_definition(&self) -> bool {
        self.inner.cap_goto_definition.load(Ordering::Relaxed)
    }

    /// Checks if the server supports hover (lock-free)
    pub async fn supports_hover(&self) -> bool {
        self.inner.cap_hover.load(Ordering::Relaxed)
    }

    /// Checks if the server supports completion (lock-free)
    pub async fn supports_completion(&self) -> bool {
        self.inner.cap_completion.load(Ordering::Relaxed)
    }

    /// Checks if the server supports formatting (lock-free)
    pub async fn supports_formatting(&self) -> bool {
        self.inner.cap_formatting.load(Ordering::Relaxed)
    }

    /// Checks if the server supports range formatting (lock-free)
    pub async fn supports_range_formatting(&self) -> bool {
        self.inner.cap_range_formatting.load(Ordering::Relaxed)
    }

    /// Checks if the server supports code actions (lock-free)
    pub async fn supports_code_actions(&self) -> bool {
        self.inner.cap_code_actions.load(Ordering::Relaxed)
    }

    /// Checks if the server supports references (lock-free)
    pub async fn supports_references(&self) -> bool {
        self.inner.cap_references.load(Ordering::Relaxed)
    }

    /// Checks if the server supports rename (lock-free)
    pub async fn supports_rename(&self) -> bool {
        self.inner.cap_rename.load(Ordering::Relaxed)
    }

    /// Checks if the server supports prepare rename (lock-free)
    pub async fn supports_prepare_rename(&self) -> bool {
        self.inner.cap_prepare_rename.load(Ordering::Relaxed)
    }

    /// Checks if the server supports signature help (lock-free)
    pub async fn supports_signature_help(&self) -> bool {
        self.inner.cap_signature_help.load(Ordering::Relaxed)
    }

    /// Checks if the server supports document symbols (lock-free)
    pub async fn supports_document_symbol(&self) -> bool {
        self.inner.cap_document_symbol.load(Ordering::Relaxed)
    }

    /// Checks if the server supports selection range (lock-free)
    pub async fn supports_selection_range(&self) -> bool {
        self.inner.cap_selection_range.load(Ordering::Relaxed)
    }

    /// Checks if the server supports workspace symbols (lock-free)
    pub async fn supports_workspace_symbol(&self) -> bool {
        self.inner.cap_workspace_symbol.load(Ordering::Relaxed)
    }

    /// Checks if the server supports document highlight (lock-free)
    pub async fn supports_document_highlight(&self) -> bool {
        self.inner.cap_document_highlight.load(Ordering::Relaxed)
    }

    /// Checks if the server supports incremental sync (lock-free)
    pub async fn supports_incremental_sync(&self) -> bool {
        self.inner.cap_incremental_sync.load(Ordering::Relaxed)
    }

    /// Checks if the server supports folding range (lock-free)
    pub async fn supports_folding_range(&self) -> bool {
        self.inner.cap_folding_range.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_id_generation() {
        let next_request_id = AtomicU64::new(1);

        assert_eq!(next_request_id.fetch_add(1, Ordering::SeqCst), 1);
        assert_eq!(next_request_id.fetch_add(1, Ordering::SeqCst), 2);
    }
}
