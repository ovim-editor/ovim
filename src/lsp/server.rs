//! Language server process management
//!
//! Manages the lifecycle of a single language server process, including:
//! - Process spawning and monitoring
//! - stdio communication
//! - Request/response matching
//! - Initialization handshake

use super::protocol::{write_message, JsonRpcMessage, RequestId, ResponseError};
use anyhow::{anyhow, Result};
use lsp_types::{
    ClientCapabilities, InitializeParams, InitializeResult, InitializedParams, ServerCapabilities,
    Url, WorkspaceFolder,
};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

/// A language server process
#[derive(Clone)]
pub struct LanguageServer {
    inner: Arc<LanguageServerInner>,
}

struct LanguageServerInner {
    /// Child process handle
    process: Mutex<Option<Child>>,

    /// Stdin writer
    stdin: Mutex<Option<ChildStdin>>,

    /// Server capabilities after initialization
    capabilities: Mutex<Option<ServerCapabilities>>,

    /// Pending requests awaiting responses
    pending_requests: Mutex<HashMap<RequestId, oneshot::Sender<Value>>>,

    /// Next request ID
    next_request_id: AtomicU64,

    /// Channel to send outgoing messages
    outgoing_tx: mpsc::Sender<JsonRpcMessage>,

    /// Channel to receive incoming messages
    incoming_rx: Mutex<Option<mpsc::Receiver<JsonRpcMessage>>>,
}

impl LanguageServer {
    /// Spawns a new language server process
    pub async fn spawn(command: &str, args: Vec<String>) -> Result<Self> {
        let mut child = Command::new(command)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()) // TODO: Capture stderr for logging
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to open stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Failed to open stdout"))?;

        let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<JsonRpcMessage>(100);
        let (incoming_tx, incoming_rx) = mpsc::channel::<JsonRpcMessage>(100);

        let inner = Arc::new(LanguageServerInner {
            process: Mutex::new(Some(child)),
            stdin: Mutex::new(Some(stdin)),
            capabilities: Mutex::new(None),
            pending_requests: Mutex::new(HashMap::new()),
            next_request_id: AtomicU64::new(1),
            outgoing_tx,
            incoming_rx: Mutex::new(Some(incoming_rx)),
        });

        let server = Self { inner };

        // Spawn task to write messages to stdin
        let inner_clone = server.inner.clone();
        tokio::spawn(async move {
            let mut stdin = inner_clone.stdin.lock().await.take();
            if let Some(ref mut stdin) = stdin {
                while let Some(msg) = outgoing_rx.recv().await {
                    if let Err(e) = write_message(stdin, &msg).await {
                        eprintln!("Error writing to language server: {}", e);
                        break;
                    }
                }
            }
        });

        // Spawn task to read messages from stdout
        let inner_clone = server.inner.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            loop {
                // Read Content-Length header
                let mut header = String::new();
                if reader.read_line(&mut header).await.is_err() {
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
                    None => continue,
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
                    eprintln!("Error reading message: {}", e);
                    break;
                }

                // Parse JSON
                match serde_json::from_slice::<JsonRpcMessage>(&content) {
                    Ok(msg) => {
                        if msg.is_response() {
                            // Handle response
                            if let Some(id) = msg.id {
                                let mut pending = inner_clone.pending_requests.lock().await;
                                if let Some(tx) = pending.remove(&id) {
                                    if let Some(result) = msg.result {
                                        let _ = tx.send(result);
                                    } else if let Some(error) = msg.error {
                                        eprintln!("LSP error: {:?}", error);
                                    }
                                }
                            }
                        } else {
                            // Handle notification or request
                            let _ = incoming_tx.send(msg).await;
                        }
                    }
                    Err(e) => {
                        eprintln!("Error parsing LSP message: {}", e);
                    }
                }
            }
        });

        Ok(server)
    }

    /// Initializes the language server
    pub async fn initialize(&mut self, root_uri: Url) -> Result<()> {
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

        let result = self.request("initialize", serde_json::to_value(params)?).await?;

        let init_result: InitializeResult = serde_json::from_value(result)?;

        // Store capabilities
        let mut caps = self.inner.capabilities.lock().await;
        *caps = Some(init_result.capabilities);

        // Send initialized notification
        self.notify("initialized", serde_json::to_value(InitializedParams {})?).await?;

        Ok(())
    }

    /// Sends a request and waits for the response
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let request_id = RequestId::Number(
            self.inner
                .next_request_id
                .fetch_add(1, Ordering::SeqCst),
        );

        let (tx, rx) = oneshot::channel();

        // Register pending request
        {
            let mut pending = self.inner.pending_requests.lock().await;
            pending.insert(request_id.clone(), tx);
        }

        // Send request
        let msg = JsonRpcMessage::request(request_id.clone(), method.to_string(), params);
        self.inner
            .outgoing_tx
            .send(msg)
            .await
            .map_err(|_| anyhow!("Failed to send request"))?;

        // Wait for response with timeout
        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(_)) => Err(anyhow!("Response channel closed")),
            Err(_) => {
                // Timeout - remove pending request
                let mut pending = self.inner.pending_requests.lock().await;
                pending.remove(&request_id);
                Err(anyhow!("Request timed out"))
            }
        }
    }

    /// Sends a notification (no response expected)
    pub async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let msg = JsonRpcMessage::notification(method.to_string(), params);
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

    /// Shuts down the language server
    pub async fn shutdown(&mut self) -> Result<()> {
        // Send shutdown request
        let _ = self.request("shutdown", Value::Null).await;

        // Send exit notification
        let _ = self.notify("exit", Value::Null).await;

        // Kill process
        let mut process = self.inner.process.lock().await;
        if let Some(ref mut child) = *process {
            let _ = child.kill().await;
        }

        Ok(())
    }

    /// Gets the server capabilities
    pub async fn capabilities(&self) -> Option<ServerCapabilities> {
        let caps = self.inner.capabilities.lock().await;
        caps.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_id_generation() {
        let server_inner = LanguageServerInner {
            process: Mutex::new(None),
            stdin: Mutex::new(None),
            capabilities: Mutex::new(None),
            pending_requests: Mutex::new(HashMap::new()),
            next_request_id: AtomicU64::new(1),
            outgoing_tx: mpsc::channel(1).0,
            incoming_rx: Mutex::new(None),
        };

        assert_eq!(
            server_inner.next_request_id.fetch_add(1, Ordering::SeqCst),
            1
        );
        assert_eq!(
            server_inner.next_request_id.fetch_add(1, Ordering::SeqCst),
            2
        );
    }
}
