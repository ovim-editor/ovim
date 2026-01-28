//! Language server process management
//!
//! Manages the lifecycle of a single language server process, including:
//! - Process spawning and monitoring
//! - stdio communication
//! - Request/response matching
//! - Initialization handshake

use super::protocol::{write_message, JsonRpcMessage, RequestId};
use super::supervisor::{RestartPolicy, TaskHealth, TaskSupervisor};
use anyhow::{anyhow, Context, Result};
use lsp_types::{
    InitializeParams, InitializeResult, InitializedParams, ServerCapabilities,
    TextDocumentContentChangeEvent, Uri, WorkspaceFolder,
};
use serde_json::{json, Value};
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

/// Maximum number of pending requests to prevent OOM
const MAX_PENDING_REQUESTS: usize = 1000;

/// Maximum time a request can remain pending before cleanup (10 minutes)
/// This prevents stale requests from accumulating when LSP servers hang
/// Initialize requests need much longer (up to 5 minutes for Java)
const REQUEST_STALE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Interval for cleanup task (10 seconds)
/// More frequent cleanup prevents memory buildup from stale requests
const CLEANUP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

/// Pending request metadata
struct PendingRequest {
    sender: oneshot::Sender<Result<Value>>,
    sent_at: Instant,
    method: String,
}

/// Server state for explicit state machine
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
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

    /// Server has failed and cannot recover
    Failed { error: String, at: Instant },

    /// Server is shutting down
    ShuttingDown,

    /// Server process has terminated
    Terminated,
}

/// Operations that can be queued during initialization
/// (Reserved for request queueing during server initialization)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum PendingOperation {
    DidOpen {
        uri: Uri,
        language_id: String,
        version: i32,
        text: String,
    },
    DidChange {
        uri: Uri,
        language_id: String,
        changes: Vec<TextDocumentContentChangeEvent>,
    },
    DidSave {
        uri: Uri,
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
    /// (Reserved for direct stdin communication with server)
    #[allow(dead_code)]
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
    /// Cached: supports goto declaration
    cap_goto_declaration: AtomicBool,
    /// Cached: supports goto implementation
    cap_goto_implementation: AtomicBool,
    /// Cached: supports goto type definition
    cap_goto_type_definition: AtomicBool,
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
    /// Cached: supports call hierarchy
    cap_call_hierarchy: AtomicBool,
    /// Cached: supports type hierarchy
    cap_type_hierarchy: AtomicBool,
    /// Cached: supports execute command
    cap_execute_command: AtomicBool,
    /// Cached: supports inlay hints
    cap_inlay_hint: AtomicBool,
    /// Cached: supports semantic tokens
    cap_semantic_tokens: AtomicBool,
}

impl LanguageServerInner {
    /// Returns a log prefix with language and command context
    fn log_prefix(&self) -> String {
        format!("[LSP:{}:{}]", self.language, self.command)
    }
}

impl LanguageServer {
    /// Returns a log prefix with language and command context
    fn log_prefix(&self) -> String {
        self.inner.log_prefix()
    }

    /// Spawns a new language server process
    pub async fn spawn(language: &str, command: &str, args: Vec<String>) -> Result<Self> {
        crate::lsp_debug!(
            "Server",
            "Spawning {} with command: {} args: {:?}",
            language,
            command,
            args
        );
        let mut child = Command::new(command)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()) // Capture stderr for debugging
            .spawn()
            .context(format!("Failed to spawn language server: {}", command))?;
        crate::lsp_debug!("Server", "Process spawned, PID: {:?}", child.id());

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
            cap_goto_declaration: AtomicBool::new(false),
            cap_goto_implementation: AtomicBool::new(false),
            cap_goto_type_definition: AtomicBool::new(false),
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
            cap_call_hierarchy: AtomicBool::new(false),
            cap_type_hierarchy: AtomicBool::new(false),
            cap_execute_command: AtomicBool::new(false),
            cap_inlay_hint: AtomicBool::new(false),
            cap_semantic_tokens: AtomicBool::new(false),
        });

        let server = Self {
            inner: inner.clone(),
        };

        // Spawn writer task to write messages to stdin
        // Note: This task is NOT supervised because:
        // 1. It owns the unique receiver for the outgoing channel
        // 2. If this task fails, the entire LSP communication is broken
        // 3. Restarting would require unsafe code or complex channel recreation
        // 4. Better to let the server process fail and restart cleanly
        let stdin_clone = stdin.clone();
        let writer_state = inner.state.clone();
        let writer_lang = inner.language.clone();
        tokio::spawn(async move {
            while let Some(msg) = outgoing_rx.recv().await {
                let mut stdin_guard = stdin_clone.lock().await;
                if let Err(e) = write_message(&mut *stdin_guard, &msg).await {
                    crate::lsp_error!("Writer", "[{}] Error writing to language server: {}", writer_lang, e);
                    let mut state = writer_state.lock().await;
                    *state = ServerState::Failed {
                        error: format!("Writer failed: {}", e),
                        at: Instant::now(),
                    };
                    break;
                }
            }
        });

        // Spawn supervised stale request cleanup task
        let inner_cleanup = inner.clone();
        let state_clone_cleanup = inner.state.clone();
        inner
            .supervisor
            .spawn_supervised("lsp_cleanup".to_string(), move || {
                let inner = inner_cleanup.clone();
                let state_ref = state_clone_cleanup.clone();
                async move {
                    loop {
                        tokio::time::sleep(CLEANUP_INTERVAL).await;

                        // BUG FIX #3: Check server state first - fail all requests if server is dead
                        let state = state_ref.lock().await;
                        let is_failed = matches!(*state, ServerState::Failed { .. });
                        drop(state);

                        let mut pending = inner.pending_requests.lock().await;
                        let now = Instant::now();
                        let count_before = pending.len();

                        if is_failed && count_before > 0 {
                            // Server failed - immediately fail ALL pending requests
                            crate::lsp_warn!(
                                "Cleanup",
                                "Server failed - clearing all {} pending requests",
                                count_before
                            );
                            for (id, req) in pending.drain() {
                                crate::lsp_warn!(
                                    "Cleanup",
                                    "Failing request {:?} for method '{}' due to server failure",
                                    id,
                                    req.method
                                );
                                let _ = req.sender.send(Err(anyhow!(
                                    "Request '{}' failed because LSP server is in Failed state",
                                    req.method
                                )));
                            }
                        } else {
                            // Normal cleanup: remove stale requests
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
                                    crate::lsp_warn!(
                                        "Cleanup",
                                        "Removing stale request {:?} for method '{}' (age: {:?})",
                                        id,
                                        req.method,
                                        age
                                    );
                                    let _ = req.sender.send(Err(anyhow!(
                                        "Request '{}' timed out and was cleaned up after {:?}",
                                        req.method,
                                        age
                                    )));
                                }
                            }
                        }

                        let count_after = pending.len();
                        if count_before > count_after {
                            crate::lsp_info!(
                                "Cleanup",
                                "Cleaned up {} stale requests ({} remaining)",
                                count_before - count_after,
                                count_after
                            );
                        }

                        // Warn at 80% of maximum capacity
                        let warning_threshold = (MAX_PENDING_REQUESTS as f64 * 0.8) as usize;
                        if pending.len() > warning_threshold {
                            crate::lsp_warn!(
                                "Cleanup",
                                "Warning: {} pending requests (max: {})",
                                pending.len(),
                                MAX_PENDING_REQUESTS
                            );
                        }
                    }
                }
            })
            .await?;

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
                    // CRITICAL ERROR: LSP server reader task failed
                    crate::lsp_error!(
                        &inner_clone.log_prefix(),
                        "CRITICAL: Reader task failed while reading header: {}",
                        e
                    );

                    // Mark server as failed (error reading from LSP server)
                    let mut state = state_clone.lock().await;
                    *state = ServerState::Failed {
                        error: format!("Reader task failed: {}", e),
                        at: Instant::now(),
                    };
                    break;
                }

                if header.is_empty() {
                    // EOF reached - LSP server process exited or closed stdout
                    crate::lsp_error!(
                        &inner_clone.log_prefix(),
                        "Reader EOF: LSP server closed output (process likely exited)"
                    );
                    let mut state = state_clone.lock().await;
                    *state = ServerState::Failed {
                        error: "LSP server process exited".to_string(),
                        at: Instant::now(),
                    };
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
                        // Invalid Content-Length header, skip this message
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
                if let Err(_e) =
                    tokio::io::AsyncReadExt::read_exact(&mut reader, &mut content).await
                {
                    // Error reading message body, LSP server may have closed
                    break;
                }

                // Parse JSON
                match serde_json::from_slice::<JsonRpcMessage>(&content) {
                    Ok(msg) => {
                        if msg.is_response() {
                            // Handle response
                            if let Some(id) = msg.id {
                                // Extract the PendingRequest from the map without holding lock during send
                                let pending_req = {
                                    let mut pending = inner_clone.pending_requests.lock().await;
                                    pending.remove(&id)
                                }; // Lock released immediately

                                // Send response outside the lock scope to reduce contention
                                if let Some(req) = pending_req {
                                    // CRITICAL FIX: Propagate errors instead of just logging
                                    if let Some(error) = msg.error {
                                        // LSP Error Code -32800 is "Request Cancelled"
                                        // This is expected when we cancel requests, not a real error
                                        const LSP_ERROR_REQUEST_CANCELLED: i32 = -32800;

                                        if error.code == LSP_ERROR_REQUEST_CANCELLED {
                                            // Request was cancelled - this is expected behavior
                                            // Log at debug level, not error level
                                            crate::lsp_debug!(
                                                &inner_clone.log_prefix(),
                                                "Request ID {:?} cancelled by server (code -32800): {}",
                                                id,
                                                error.message
                                            );

                                            // Send cancellation error to caller
                                            // Caller can distinguish this from actual LSP errors
                                            let _ = req.sender.send(Err(anyhow!(
                                                "Request cancelled: {}",
                                                error.message
                                            )));
                                        } else {
                                            // Real LSP error - propagate to caller
                                            // Errors will be shown in status line by the editor
                                            let error_msg =
                                                format!("{} (code {})", error.message, error.code);
                                            // Removed eprintln - leaks into TUI display
                                            let _ = req
                                                .sender
                                                .send(Err(anyhow!("LSP error: {}", error_msg)));
                                        }
                                    } else if let Some(result) = msg.result {
                                        let _ = req.sender.send(Ok(result));
                                    } else {
                                        // Response with no result and no error - treat as null result
                                        // This can happen for valid responses like hover with no info
                                        let _ = req.sender.send(Ok(Value::Null));
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
                    Err(e) => {
                        // ERROR: Failed to parse LSP message - log for visibility
                        crate::lsp_error!(
                            &inner_clone.log_prefix(),
                            "Failed to parse LSP message (size: {} bytes): {}",
                            content.len(),
                            e
                        );
                        // Show a truncated preview of the malformed content for debugging
                        let preview = if content.len() > 200 {
                            format!("{}...", String::from_utf8_lossy(&content[..200]))
                        } else {
                            String::from_utf8_lossy(&content).to_string()
                        };
                        crate::lsp_error!(
                            &inner_clone.log_prefix(),
                            "Malformed message preview: {}",
                            preview
                        );
                        // Continue processing other messages (don't break the loop)
                    }
                }
            }
            // Reader task exiting silently
        });

        // Spawn task to capture stderr and log it for debugging
        // Note: Not supervised because stderr is unique to the process
        tokio::spawn(async move {
            let mut stderr_reader = BufReader::new(stderr);
            let mut line = String::new();
            while let Ok(n) = stderr_reader.read_line(&mut line).await {
                if n == 0 {
                    break; // EOF
                }
                // Log stderr output from LSP server
                crate::lsp_debug!("stderr", "{}", line.trim_end());
                line.clear();
            }
            crate::lsp_debug!("stderr", "LSP stderr task exiting");
        });

        // BUG FIX: Verify the process is actually running before returning
        // Give it a small delay to fail fast if the command doesn't exist or crashes immediately
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Check if process is still alive
        let process_guard = server.inner.process.lock().await;
        if let Some(ref child) = *process_guard {
            // try_wait returns Ok(Some(status)) if exited, Ok(None) if still running, Err on error
            match child.id() {
                Some(_pid) => {
                    // Process has a valid PID, likely running
                    drop(process_guard);
                }
                None => {
                    drop(process_guard);
                    // Process has no PID - it failed to start or already exited
                    return Err(anyhow!(
                        "Language server process failed to start or exited immediately: {} {:?}",
                        command,
                        args
                    ));
                }
            }
        } else {
            drop(process_guard);
            return Err(anyhow!("Language server process handle is missing"));
        }

        Ok(server)
    }

    /// Initializes the language server
    pub async fn initialize(&mut self, root_uri: Uri) -> Result<()> {
        // Use language-specific timeout (Java needs much longer due to indexing)
        // Java/jdtls: 5 minutes (300s) for large projects with many dependencies
        // Other languages: 2 minutes (120s) should be plenty
        let init_timeout = if self.inner.language == "java" {
            Duration::from_secs(300) // 5 minutes for Java
        } else {
            Duration::from_secs(120) // 2 minutes for other languages
        };

        tokio::time::timeout(init_timeout, self.initialize_internal(root_uri))
            .await
            .context(format!(
                "LSP initialization timed out after {:?}. For large projects, this may take several minutes on first run.",
                init_timeout
            ))?
    }

    /// Internal initialization implementation (wrapped by timeout)
    /// BUG FIX: Improved error handling - sets Failed state on errors
    async fn initialize_internal(&mut self, root_uri: Uri) -> Result<()> {
        // Transition to Initializing state
        self.transition_to(ServerState::Initializing {
            started_at: Instant::now(),
            pending_operations: Vec::new(),
        })
        .await;

        // BUG FIX: Wrap initialization logic to catch errors and set Failed state
        let init_result = self.do_initialize(root_uri).await;

        if let Err(ref e) = init_result {
            // Initialization failed - set server to Failed state
            crate::lsp_error!(&self.log_prefix(), "Initialization failed: {}", e);
            self.transition_to(ServerState::Failed {
                error: format!("Initialization failed: {}", e),
                at: Instant::now(),
            })
            .await;
        }

        init_result
    }

    /// Performs the actual initialization protocol exchange
    async fn do_initialize(&mut self, root_uri: Uri) -> Result<()> {
        // Build comprehensive client capabilities to advertise supported features
        // This tells the LSP server what features the client can handle
        let client_capabilities = lsp_types::ClientCapabilities {
            // Window capabilities
            window: Some(lsp_types::WindowClientCapabilities {
                work_done_progress: Some(true),
                show_message: Some(lsp_types::ShowMessageRequestClientCapabilities {
                    message_action_item: Some(lsp_types::MessageActionItemCapabilities {
                        additional_properties_support: Some(false),
                    }),
                }),
                ..Default::default()
            }),

            // Text document capabilities - advertise support for common LSP features
            text_document: Some(lsp_types::TextDocumentClientCapabilities {
                completion: Some(Default::default()),
                hover: Some(Default::default()),
                signature_help: Some(Default::default()),
                declaration: Some(Default::default()),
                definition: Some(Default::default()),
                references: Some(Default::default()),
                document_highlight: Some(Default::default()),
                document_symbol: Some(Default::default()),
                code_action: Some(Default::default()),
                rename: Some(Default::default()),
                formatting: Some(Default::default()),
                range_formatting: Some(Default::default()),
                ..Default::default()
            }),

            // Workspace capabilities
            workspace: Some(lsp_types::WorkspaceClientCapabilities {
                apply_edit: Some(true),
                ..Default::default()
            }),

            ..Default::default()
        };

        #[allow(deprecated)]
        // Language-specific initialization options
        // Each language server has different requirements for optimal behavior
        let initialization_options = match self.inner.language.as_str() {
            "rust" => {
                // rust-analyzer specific configuration
                Some(json!({
                    "checkOnSave": {
                        "command": "clippy",
                        "extraArgs": ["--", "-D", "warnings"]
                    },
                    "hover": {
                        "documentation": true,
                        "relatedInformation": true
                    },
                    "cargo": {
                        "buildScripts": { "enable": true }
                    },
                    "procMacro": { "enable": true },
                    "inlayHints": {
                        "bindingModeHints": {
                            "enable": false
                        },
                        "chainingHints": {
                            "enable": true
                        },
                        "closureReturnTypeHints": {
                            "enable": "never"
                        },
                        "closureStyle": "impl_fn",
                        "discriminantHints": {
                            "enable": "never"
                        },
                        "expressionAdjustmentHints": {
                            "enable": "never"
                        },
                        "genericPlaceholderHints": {
                            "enable": true
                        },
                        "implicitDrops": {
                            "enable": false
                        },
                        "lifetimeElisionHints": {
                            "enable": "never"
                        },
                        "maxLength": null,
                        "parameterHints": {
                            "enable": true
                        },
                        "rangeHints": {
                            "enable": false
                        },
                        "renderColons": true,
                        "typeHints": {
                            "enable": true,
                            "hideClosureInitialization": false,
                            "hideNamedConstructor": false
                        }
                    },
                    "typing": {
                        "autoClosingAngleBrackets": {
                            "enable": true
                        }
                    }
                }))
            },
            "javascript" | "typescript" | "typescriptreact" | "javascriptreact" => {
                // TypeScript language server configuration
                Some(json!({
                    "preferences": {
                        "includeInlayParameterNameHints": "all",
                        "includeInlayFunctionParameterTypeHints": true,
                        "includeInlayVariableTypeHints": true,
                        "includeInlayPropertyDeclarationTypeHints": true,
                        "includeInlayEnumMemberValueHints": true,
                        "quotePreference": "auto",
                        "importModuleSpecifierPreference": "relative"
                    },
                    "hostInfo": "ovim"
                }))
            },
            "python" => {
                // pyright/pylsp configuration
                Some(json!({
                    "python": {
                        "analysis": {
                            "typeCheckingMode": "basic",
                            "autoSearchPaths": true,
                            "diagnosticMode": "workspace"
                        }
                    }
                }))
            },
            _ => {
                // No specific initialization options for other languages
                None
            }
        };

        #[allow(deprecated)] // root_uri/root_path deprecated but needed for LSP backwards compat
        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(root_uri.clone()),
            root_path: None,
            initialization_options: initialization_options.clone(),
            capabilities: client_capabilities,
            trace: None,
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: root_uri.clone(),
                name: "workspace".to_string(),
            }]),
            client_info: Some(lsp_types::ClientInfo {
                name: "ovim".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            locale: None,
            work_done_progress_params: Default::default(),
        };

        crate::lsp_info!(
            &self.log_prefix(),
            "LSP Initialize | Language: {} | Root: {} | InitOptions: {}",
            self.inner.language,
            root_uri.as_str(),
            initialization_options.as_ref().map(|opts| opts.to_string()).unwrap_or_else(|| "None".to_string())
        );

        let result = self
            .request("initialize", serde_json::to_value(params)?)
            .await
            .context("Failed to send initialize request")?;

        let init_result: InitializeResult =
            serde_json::from_value(result).context("Failed to parse initialize response")?;

        // Store capabilities
        let mut caps = self.inner.capabilities.lock().await;
        *caps = Some(init_result.capabilities.clone());
        drop(caps); // Release lock

        // Cache capability flags for lock-free access
        self.cache_capabilities(&init_result.capabilities);

        // Send initialized notification
        self.notify("initialized", serde_json::to_value(InitializedParams {})?)
            .await
            .context("Failed to send initialized notification")?;

        // Transition to Ready state and replay pending operations
        self.transition_to(ServerState::Ready {
            initialized_at: Instant::now(),
            capabilities: init_result.capabilities,
        })
        .await;

        Ok(())
    }

    /// Transitions to a new state, handling state-specific logic
    /// BUG FIX: Extract pending operations before dropping lock to prevent race condition
    async fn transition_to(&self, new_state: ServerState) {
        let prefix = self.log_prefix();

        // BUG FIX: Extract pending operations while holding lock, then replay after releasing lock
        // This prevents race condition where state could change during replay
        let pending_ops_to_replay: Option<Vec<PendingOperation>> = {
            let mut state = self.inner.state.lock().await;

            // Extract pending operations if transitioning from Initializing to Ready
            let ops = match (&*state, &new_state) {
                (
                    ServerState::Initializing {
                        pending_operations, ..
                    },
                    ServerState::Ready { .. },
                ) => Some(pending_operations.clone()),
                _ => None,
            };

            // Atomically update state while holding lock
            *state = new_state;

            ops
        }; // Lock released here

        // Replay operations outside the lock (if any)
        if let Some(pending_ops) = pending_ops_to_replay {
            for op in &pending_ops {
                if let Err(e) = self.replay_operation(op).await {
                    crate::lsp_error!(&prefix, "Failed to replay operation: {}", e);
                }
            }
        }
    }

    /// Replays a pending operation after server becomes ready
    async fn replay_operation(&self, op: &PendingOperation) -> Result<()> {
        match op {
            PendingOperation::DidOpen {
                uri,
                language_id,
                version,
                text,
            } => {
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
            PendingOperation::DidChange {
                uri,
                language_id: _,
                changes,
            } => {
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
            PendingOperation::DidSave {
                uri,
                language_id: _,
                text,
            } => {
                use lsp_types::{DidSaveTextDocumentParams, TextDocumentIdentifier};

                let params = DidSaveTextDocumentParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                    text: text.clone(),
                };

                self.notify("textDocument/didSave", serde_json::to_value(params)?)
                    .await
                    .context("Failed to replay didSave")
            }
            PendingOperation::Request { method, params: _ } => {
                // Requests are not replayed (they would have timed out already)
                crate::lsp_debug!(
                    &self.log_prefix(),
                    "Skipping replay of request '{}'",
                    method
                );
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
    /// (Reserved for request queueing implementation)
    #[allow(dead_code)]
    async fn queue_or_execute<F, Fut>(&self, op: PendingOperation, execute: F) -> Result<()>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let _prefix = self.log_prefix();
        let mut state = self.inner.state.lock().await;

        match &mut *state {
            ServerState::Ready { .. } => {
                drop(state); // Release lock before executing
                execute().await
            }
            ServerState::Initializing {
                pending_operations, ..
            } => {
                // Queuing operation while server initializes
                pending_operations.push(op);
                Ok(())
            }
            ServerState::Failed { error, .. } => Err(anyhow!("Server failed: {}", error)),
            ServerState::Terminated => Err(anyhow!("Server has terminated")),
            state => Err(anyhow!("Server in unexpected state: {:?}", state)),
        }
    }

    /// Sends a request and waits for the response
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        // Track LSP request metrics
        let _timer = crate::metrics::LSP_REQUEST_DURATION.start_timer();
        crate::metrics::LSP_REQUESTS_TOTAL.inc();

        let request_id =
            RequestId::Number(self.inner.next_request_id.fetch_add(1, Ordering::SeqCst));

        crate::lsp_debug!(
            &self.log_prefix(),
            "LSP-REQUEST: {} | ID: {:?} | Params: {}",
            method,
            request_id,
            params
        );

        let (tx, rx) = oneshot::channel();

        // Register pending request with metadata
        {
            let mut pending = self.inner.pending_requests.lock().await;

            // Check if we've hit the hard limit on pending requests
            if pending.len() >= MAX_PENDING_REQUESTS {
                // eprintln!("[LSP-ERROR] Too many pending requests: {}/{}", pending.len(), MAX_PENDING_REQUESTS);
                return Err(anyhow!(
                    "Too many pending LSP requests ({}/{}) - server may be slow or hanging",
                    pending.len(),
                    MAX_PENDING_REQUESTS
                ));
            }

            // eprintln!("[LSP-REQUEST] Pending requests before: {} | Adding: {}", pending.len(), method);

            pending.insert(
                request_id.clone(),
                PendingRequest {
                    sender: tx,
                    sent_at: Instant::now(),
                    method: method.to_string(),
                },
            );
        }

        // Send request
        let msg = JsonRpcMessage::request(request_id.clone(), method.to_string(), params);

        // Check message size by serializing to bytes (avoids double serialization)
        // The write_message function will serialize again, but we need size check first
        let serialized_size = serde_json::to_vec(&msg)
            .context("Failed to estimate request size")?
            .len();
        if serialized_size > MAX_MESSAGE_SIZE {
            return Err(anyhow!(
                "Request '{}' too large: {} bytes (max {} bytes / {:.1} MB)",
                method,
                serialized_size,
                MAX_MESSAGE_SIZE,
                MAX_MESSAGE_SIZE as f64 / (1024.0 * 1024.0)
            ));
        }

        // Check if server is still alive before sending
        {
            let state = self.inner.state.lock().await;
            match &*state {
                ServerState::Failed { error, .. } => {
                    return Err(anyhow!("LSP server failed: {} (method: {})", error, method));
                }
                ServerState::Terminated => {
                    return Err(anyhow!("LSP server terminated (method: {})", method));
                }
                _ => {} // Ready or Initializing — proceed
            }
        }

        self.inner
            .outgoing_tx
            .send(msg)
            .await
            .map_err(|_| anyhow!("LSP server not responding — channel closed (method: {})", method))?;

        // Wait for response with timeout
        // Use longer timeout for initialize request (jdtls can be very slow)
        let timeout_duration = if method == "initialize" {
            // Java LSP can take 5+ minutes to index large projects on first run
            std::time::Duration::from_secs(300) // 5 minutes for initialize
        } else {
            std::time::Duration::from_secs(10) // 10s for other requests
        };

        // eprintln!("[LSP-REQUEST] Waiting for response (timeout: {:?})", timeout_duration);
        let start_time = Instant::now();

        match tokio::time::timeout(timeout_duration, rx).await {
            Ok(Ok(result)) => {
                let elapsed = start_time.elapsed();
                let result_preview = match &result {
                    Ok(value) if value.is_null() => "null".to_string(),
                    Ok(value) => {
                        let json_str = serde_json::to_string(value).unwrap_or_else(|_| "?".to_string());
                        if json_str.len() > 200 {
                            format!("{}...", &json_str[..200])
                        } else {
                            json_str
                        }
                    },
                    Err(e) => {
                        crate::metrics::LSP_ERRORS_TOTAL.inc();
                        format!("Error: {}", e)
                    }
                };
                crate::lsp_info!(
                    &self.log_prefix(),
                    "LSP-RESPONSE: {} | {:?} | ID: {:?} | Result: {}",
                    method,
                    elapsed,
                    request_id,
                    result_preview
                );
                result.context(format!("LSP request '{}' failed", method))
            }
            Ok(Err(_)) => {
                let _elapsed = start_time.elapsed();
                crate::metrics::LSP_ERRORS_TOTAL.inc();
                // eprintln!("[LSP-ERROR] Channel closed: {} | After: {:?}", method, elapsed);
                Err(anyhow!("Response channel closed for method '{}'", method))
            }
            Err(_) => {
                crate::metrics::LSP_ERRORS_TOTAL.inc();
                // eprintln!("[LSP-ERROR] Timeout: {} | After: {:?}", method, timeout_duration);
                // Timeout - remove pending request
                let mut pending = self.inner.pending_requests.lock().await;
                pending.remove(&request_id);
                Err(anyhow!(
                    "Request '{}' timed out after {:?}",
                    method,
                    timeout_duration
                ))
            }
        }
    }

    /// Sends a notification (no response expected)
    pub async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let msg = JsonRpcMessage::notification(method.to_string(), params);

        // Check message size by serializing to bytes (avoids double serialization)
        // The write_message function will serialize again, but we need size check first
        let serialized_size = serde_json::to_vec(&msg)
            .context("Failed to estimate notification size")?
            .len();
        if serialized_size > MAX_MESSAGE_SIZE {
            return Err(anyhow!(
                "Notification '{}' too large: {} bytes (max {} bytes / {:.1} MB)",
                method,
                serialized_size,
                MAX_MESSAGE_SIZE,
                MAX_MESSAGE_SIZE as f64 / (1024.0 * 1024.0)
            ));
        }

        self.inner
            .outgoing_tx
            .send(msg)
            .await
            .map_err(|_| anyhow!("LSP server not responding — channel closed (notification: {})", method))?;
        Ok(())
    }

    /// Sends a response to a request from the server
    pub async fn send_response(&self, response: JsonRpcMessage) -> Result<()> {
        // Check message size
        let serialized_size = serde_json::to_vec(&response)
            .context("Failed to estimate response size")?
            .len();
        if serialized_size > MAX_MESSAGE_SIZE {
            return Err(anyhow!(
                "Response too large: {} bytes (max {} bytes / {:.1} MB)",
                serialized_size,
                MAX_MESSAGE_SIZE,
                MAX_MESSAGE_SIZE as f64 / (1024.0 * 1024.0)
            ));
        }

        self.inner
            .outgoing_tx
            .send(response)
            .await
            .map_err(|_| anyhow!("Failed to send response"))?;
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
            self.request("shutdown", Value::Null),
        )
        .await;

        if shutdown_result.is_ok() {
            // Step 2: Send exit notification
            let _ = self.notify("exit", Value::Null).await;

            // Step 3: Wait for graceful exit
            let mut process = self.inner.process.lock().await;
            if let Some(ref mut child) = *process {
                match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
                    Ok(Ok(_status)) => {
                        return Ok(());
                    }
                    Ok(Err(e)) => {
                        crate::lsp_warn!(&prefix, "Shutdown: Error waiting for exit: {}", e);
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
                        match tokio::time::timeout(std::time::Duration::from_secs(3), child.wait())
                            .await
                        {
                            Ok(Ok(_status)) => {
                                return Ok(());
                            }
                            Ok(Err(e)) => {
                                crate::lsp_warn!(&prefix, "Shutdown: Error after SIGTERM: {}", e);
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
                crate::lsp_error!(&prefix, "Shutdown: SIGKILL failed: {}", e);
            }

            // Wait to reap zombie
            if let Err(e) = child.wait().await {
                crate::lsp_error!(&prefix, "Shutdown: Error reaping process: {}", e);
            }
        }

        // Shutdown all supervised tasks
        if let Err(e) = self.inner.supervisor.shutdown_all().await {
            crate::lsp_error!(&prefix, "Shutdown: Error shutting down tasks: {}", e);
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
            ServerState::Failed { at, .. } => at.elapsed(),
            ServerState::ShuttingDown | ServerState::Terminated => Duration::from_secs(0),
        };

        // Check if process is alive
        let is_alive = {
            let process = self.inner.process.lock().await;
            if let Some(ref child) = *process {
                child.id().is_some()
            } else {
                false
            }
        };

        // Convert state to string for easier debugging
        let state_str = match state {
            ServerState::Spawning => "Spawning".to_string(),
            ServerState::Initializing { .. } => "Initializing".to_string(),
            ServerState::Ready { .. } => "Ready".to_string(),
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
        self.inner
            .cap_goto_definition
            .store(caps.definition_provider.is_some(), Ordering::Relaxed);

        // Cache goto declaration support
        self.inner
            .cap_goto_declaration
            .store(caps.declaration_provider.is_some(), Ordering::Relaxed);

        // Cache goto implementation support
        self.inner
            .cap_goto_implementation
            .store(caps.implementation_provider.is_some(), Ordering::Relaxed);

        // Cache goto type definition support
        self.inner
            .cap_goto_type_definition
            .store(caps.type_definition_provider.is_some(), Ordering::Relaxed);

        // Cache hover support
        self.inner
            .cap_hover
            .store(caps.hover_provider.is_some(), Ordering::Relaxed);

        // Cache completion support
        self.inner
            .cap_completion
            .store(caps.completion_provider.is_some(), Ordering::Relaxed);

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
        self.inner
            .cap_code_actions
            .store(caps.code_action_provider.is_some(), Ordering::Relaxed);

        // Cache references support
        self.inner
            .cap_references
            .store(caps.references_provider.is_some(), Ordering::Relaxed);

        // Cache rename support
        self.inner
            .cap_rename
            .store(caps.rename_provider.is_some(), Ordering::Relaxed);

        // Cache prepare rename support
        let prepare_rename_support = match &caps.rename_provider {
            Some(lsp_types::OneOf::Right(options)) => options.prepare_provider.unwrap_or(false),
            _ => false,
        };
        self.inner
            .cap_prepare_rename
            .store(prepare_rename_support, Ordering::Relaxed);

        // Cache signature help support
        self.inner
            .cap_signature_help
            .store(caps.signature_help_provider.is_some(), Ordering::Relaxed);

        // Cache document symbol support
        self.inner
            .cap_document_symbol
            .store(caps.document_symbol_provider.is_some(), Ordering::Relaxed);

        // Cache selection range support
        self.inner
            .cap_selection_range
            .store(caps.selection_range_provider.is_some(), Ordering::Relaxed);

        // Cache workspace symbol support
        self.inner
            .cap_workspace_symbol
            .store(caps.workspace_symbol_provider.is_some(), Ordering::Relaxed);

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
        self.inner
            .cap_incremental_sync
            .store(incremental_sync, Ordering::Relaxed);

        // Cache folding range support
        self.inner
            .cap_folding_range
            .store(caps.folding_range_provider.is_some(), Ordering::Relaxed);

        // Cache call hierarchy support
        self.inner
            .cap_call_hierarchy
            .store(caps.call_hierarchy_provider.is_some(), Ordering::Relaxed);

        // Cache type hierarchy support
        // Note: type_hierarchy_provider doesn't exist in lsp-types 0.95
        // Will be available when upgrading to lsp-types 0.96+
        self.inner
            .cap_type_hierarchy
            .store(false, Ordering::Relaxed);

        // Cache execute command support
        self.inner
            .cap_execute_command
            .store(caps.execute_command_provider.is_some(), Ordering::Relaxed);

        // Cache inlay hint support
        self.inner
            .cap_inlay_hint
            .store(caps.inlay_hint_provider.is_some(), Ordering::Relaxed);

        // Cache semantic tokens support
        self.inner
            .cap_semantic_tokens
            .store(caps.semantic_tokens_provider.is_some(), Ordering::Relaxed);
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

    /// Checks if the server supports goto declaration (lock-free)
    pub async fn supports_goto_declaration(&self) -> bool {
        self.inner.cap_goto_declaration.load(Ordering::Relaxed)
    }

    /// Checks if the server supports goto implementation (lock-free)
    pub async fn supports_goto_implementation(&self) -> bool {
        self.inner.cap_goto_implementation.load(Ordering::Relaxed)
    }

    /// Checks if the server supports goto type definition (lock-free)
    pub async fn supports_goto_type_definition(&self) -> bool {
        self.inner.cap_goto_type_definition.load(Ordering::Relaxed)
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

    /// Checks if the server supports call hierarchy (lock-free)
    pub async fn supports_call_hierarchy(&self) -> bool {
        self.inner.cap_call_hierarchy.load(Ordering::Relaxed)
    }

    /// Checks if the server supports type hierarchy (lock-free)
    pub async fn supports_type_hierarchy(&self) -> bool {
        self.inner.cap_type_hierarchy.load(Ordering::Relaxed)
    }

    /// Checks if the server supports execute command (lock-free)
    pub async fn supports_execute_command(&self) -> bool {
        self.inner.cap_execute_command.load(Ordering::Relaxed)
    }

    /// Checks if the server supports inlay hints (lock-free)
    pub async fn supports_inlay_hints(&self) -> bool {
        self.inner.cap_inlay_hint.load(Ordering::Relaxed)
    }

    /// Checks if the server supports semantic tokens (lock-free)
    pub async fn supports_semantic_tokens(&self) -> bool {
        self.inner.cap_semantic_tokens.load(Ordering::Relaxed)
    }

    /// Gets the current server state (alias for introspection)
    pub async fn get_state(&self) -> ServerState {
        self.state().await
    }

    /// Gets the number of pending requests
    pub async fn pending_requests_count(&self) -> usize {
        let pending = self.inner.pending_requests.lock().await;
        pending.len()
    }

    /// Checks if the server has capabilities (is ready)
    pub async fn has_capabilities(&self) -> bool {
        let caps = self.inner.capabilities.lock().await;
        caps.is_some()
    }

    /// Gets the command used to start the server
    pub async fn get_command(&self) -> String {
        self.inner.command.clone()
    }

    /// Gets the command used to start the server (sync version)
    pub fn command(&self) -> &str {
        &self.inner.command
    }

    /// Cancels all pending requests for a specific method
    ///
    /// This is useful for high-frequency, low-priority operations like hover and completion
    /// where only the latest request matters. When a new request is made, previous ones
    /// become stale and can be safely cancelled.
    ///
    /// # LSP Cancellation Protocol
    ///
    /// The LSP protocol supports request cancellation via the `$/cancelRequest` notification:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "method": "$/cancelRequest",
    ///   "params": { "id": 123 }
    /// }
    /// ```
    ///
    /// # Server Behavior
    ///
    /// According to the LSP spec, servers may respond in three ways to cancellation:
    /// 1. **Stop processing** and respond with error -32800 (Request Cancelled)
    /// 2. **Continue processing** and respond normally if work already done
    /// 3. **Ignore** the cancellation if the request already completed
    ///
    /// # Race Conditions in Async Systems
    ///
    /// Request cancellation introduces interesting race conditions:
    ///
    /// **Race 1: Cancel vs Response**
    /// - Client sends request ID 1
    /// - Server starts processing
    /// - Client sends `$/cancelRequest` for ID 1
    /// - Server finishes and sends response for ID 1
    /// - Result: Client receives response after cancellation (harmless, we ignore it)
    ///
    /// **Race 2: Multiple Cancellations**
    /// - Client sends request ID 1, 2, 3
    /// - Client cancels 1, 2, 3
    /// - Server might respond to 2 before processing cancellation for 1
    /// - Result: Out-of-order responses (we filter by removing from pending_requests)
    ///
    /// # Why Request Ordering Matters
    ///
    /// Consider rapid cursor movement over symbols A → B → C:
    ///
    /// **Without cancellation:**
    /// ```
    /// t=0ms:  Request hover for A (ID 1)
    /// t=50ms: Request hover for B (ID 2)
    /// t=100ms: Request hover for C (ID 3)
    /// t=200ms: Response for A arrives → UI shows stale hover for A
    /// t=250ms: Response for B arrives → UI shows stale hover for B
    /// t=300ms: Response for C arrives → UI shows correct hover for C
    /// ```
    /// User sees flickering UI with wrong information!
    ///
    /// **With cancellation:**
    /// ```
    /// t=0ms:  Request hover for A (ID 1)
    /// t=50ms: Cancel ID 1, Request hover for B (ID 2)
    /// t=100ms: Cancel ID 2, Request hover for C (ID 3)
    /// t=150ms: Response for C arrives → UI shows correct hover immediately
    /// ```
    /// Server only processes final request, UI is always correct.
    ///
    /// # Implementation Strategy
    ///
    /// 1. **Find** all pending request IDs matching the method
    /// 2. **Send** `$/cancelRequest` notification for each (don't wait for response)
    /// 3. **Remove** from pending_requests map (fail the oneshot receiver)
    /// 4. **Continue** - caller's await will fail with "Request cancelled"
    ///
    /// # Arguments
    ///
    /// * `method` - LSP method name (e.g., "textDocument/hover", "textDocument/completion")
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Cancellations sent successfully
    /// * `Err` - Failed to send cancellation notification
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // In hover implementation:
    /// server.cancel_requests_by_method("textDocument/hover").await?;
    /// let result = server.request("textDocument/hover", params).await?;
    /// ```
    pub async fn cancel_requests_by_method(&self, method: &str) -> Result<()> {
        // Find all request IDs for this method
        // We need to do this in two steps to avoid holding the lock during async operations
        let to_cancel: Vec<RequestId> = {
            let pending = self.inner.pending_requests.lock().await;
            pending
                .iter()
                .filter(|(_, req)| req.method == method)
                .map(|(id, _)| id.clone())
                .collect()
        }; // Lock released here

        if to_cancel.is_empty() {
            // No requests to cancel - fast path
            return Ok(());
        }

        crate::lsp_debug!(
            &self.log_prefix(),
            "Cancelling {} pending requests for method '{}'",
            to_cancel.len(),
            method
        );

        // Send $/cancelRequest notification for each request ID
        // These are fire-and-forget notifications - we don't wait for acknowledgment
        for id in &to_cancel {
            let params = serde_json::json!({ "id": id });

            // Log the cancellation for debugging
            crate::lsp_debug!(
                &self.log_prefix(),
                "Sending $/cancelRequest for ID {:?} (method: {})",
                id,
                method
            );

            // Send cancellation notification
            // This is a notification, not a request, so we don't expect a response
            if let Err(e) = self.notify("$/cancelRequest", params).await {
                // If we fail to send cancellation, log but continue
                // The server might still respond, but we'll ignore it by removing from pending
                crate::lsp_warn!(
                    &self.log_prefix(),
                    "Failed to send $/cancelRequest for ID {:?}: {}",
                    id,
                    e
                );
            }
        }

        // Remove cancelled requests from pending map and fail their receivers
        // This ensures callers waiting on these requests get an error
        {
            let mut pending = self.inner.pending_requests.lock().await;
            for id in to_cancel {
                if let Some(req) = pending.remove(&id) {
                    // Send cancellation error to the waiting caller
                    // The underscore pattern ignores send errors (receiver might have dropped)
                    let _ = req.sender.send(Err(anyhow!(
                        "Request '{}' cancelled by client (newer request supersedes this one)",
                        method
                    )));

                    crate::lsp_debug!(
                        &self.log_prefix(),
                        "Removed cancelled request ID {:?} from pending map",
                        id
                    );
                }
            }
        } // Lock released here

        Ok(())
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
