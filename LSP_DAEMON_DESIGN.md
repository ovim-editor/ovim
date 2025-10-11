# LSP Daemon Mode - Design Document

## Problem Statement

Current workflow for terminal user who opens/closes frequently:

```bash
$ ovim File1.java    # Wait 60s for jdtls init
$ ovim File2.java    # Wait ANOTHER 60s (jdtls restarted!)
$ ovim File3.java    # Wait ANOTHER 60s...
```

**This is unacceptable for quick edits.**

## Solution: Per-Project LSP Daemon

### Design Goals

1. **Fast subsequent opens** - <1s after first initialization
2. **Zero user intervention** - Daemon starts/stops automatically
3. **Project isolation** - Each project has its own daemon
4. **Resource efficient** - Auto-shutdown when idle
5. **Robust** - Handle crashes, conflicts, edge cases gracefully

### Architecture

```
User opens ovim File.java
         ↓
    Detect project root
         ↓
    Check for daemon
    ├─ Exists? → Connect (instant!)
    └─ Not exists? → Start daemon → Connect
         ↓
    Use LSP features
         ↓
    User types :q
         ↓
    Disconnect (daemon stays alive)
         ↓
    (30 min later, no activity)
         ↓
    Daemon auto-shutdown
```

### File Structure

```
~/.cache/ovim/daemons/
├── {project-hash-1}/
│   ├── daemon.sock      # Unix domain socket for IPC
│   ├── daemon.pid       # Process ID of daemon
│   ├── daemon.log       # Daemon logs
│   ├── jdtls.pid        # jdtls process ID
│   └── workspace/       # jdtls workspace data
├── {project-hash-2}/
│   └── ...
```

### Project Hash

```rust
fn project_hash(project_root: &Path) -> String {
    use std::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;

    let mut hasher = DefaultHasher::new();
    // Use canonical path to handle symlinks
    let canonical = project_root.canonicalize().unwrap_or(project_root.to_path_buf());
    canonical.to_str().unwrap().hash(&mut hasher);
    format!("{:x}", hasher.finish())
}
```

### Communication Protocol

**Socket-based IPC (Unix domain socket):**

```rust
// Message format: [4-byte length][JSON payload]

#[derive(Serialize, Deserialize)]
enum DaemonRequest {
    // Lifecycle
    Ping,
    Shutdown,

    // LSP operations
    DidOpen { uri: String, language_id: String, text: String },
    DidChange { uri: String, text: String, version: i32 },
    DidSave { uri: String, text: Option<String> },
    DidClose { uri: String },

    Hover { uri: String, line: u32, character: u32 },
    GotoDefinition { uri: String, line: u32, character: u32 },
    Completion { uri: String, line: u32, character: u32 },
    FormatDocument { uri: String },
    CodeActions { uri: String, line: u32, character: u32 },

    GetDiagnostics { uri: String },
}

#[derive(Serialize, Deserialize)]
enum DaemonResponse {
    Ok,
    Pong { uptime_secs: u64 },

    HoverResult(Option<String>),
    DefinitionResult(Option<Location>),
    CompletionResult(Vec<CompletionItem>),
    FormatResult(Vec<TextEdit>),
    CodeActionsResult(Vec<CodeAction>),
    DiagnosticsResult(Vec<Diagnostic>),

    Error(String),
}
```

## Implementation Plan

### Phase 1: Basic Daemon Infrastructure

**Files to create:**

1. `src/daemon/mod.rs` - Main daemon module
2. `src/daemon/server.rs` - Daemon server process
3. `src/daemon/client.rs` - Client that connects to daemon
4. `src/daemon/protocol.rs` - Message protocol
5. `src/daemon/manager.rs` - Daemon lifecycle management

### Phase 2: Integration with ovim

**Files to modify:**

1. `src/main.rs` - Add `--daemon-mode` flag, integrate client
2. `src/editor/mod.rs` - Use daemon client instead of direct LSP manager

### Phase 3: Edge Case Handling

Handle all the edge cases (see testing section)

### Phase 4: Performance Optimization

Benchmark and optimize startup time

## Key Components

### 1. Daemon Server (`src/daemon/server.rs`)

```rust
pub struct LspDaemonServer {
    project_root: PathBuf,
    socket_path: PathBuf,
    lsp_manager: Arc<tokio::sync::Mutex<LspManager>>,
    last_activity: Arc<tokio::sync::Mutex<Instant>>,
    idle_timeout: Duration,
    shutdown_tx: broadcast::Sender<()>,
}

impl LspDaemonServer {
    pub async fn start(project_root: PathBuf, socket_path: PathBuf) -> Result<Self> {
        // 1. Initialize LSP manager (spawn jdtls, initialize)
        // 2. Create Unix socket listener
        // 3. Start activity timeout checker
        // 4. Accept connections and handle requests
    }

    pub async fn run(&mut self) -> Result<()> {
        let listener = UnixListener::bind(&self.socket_path)?;
        let (shutdown_tx, _) = broadcast::channel(1);

        loop {
            tokio::select! {
                // Handle new connections
                result = listener.accept() => {
                    let (stream, _) = result?;
                    self.handle_client(stream).await?;
                }

                // Check for idle timeout every minute
                _ = tokio::time::sleep(Duration::from_secs(60)) => {
                    if self.is_idle() {
                        info!("Daemon idle for {} minutes, shutting down",
                              self.idle_timeout.as_secs() / 60);
                        break;
                    }
                }

                // Handle shutdown signal
                _ = shutdown_tx.subscribe().recv() => {
                    info!("Received shutdown signal");
                    break;
                }
            }
        }

        self.cleanup().await
    }

    async fn handle_client(&self, stream: UnixStream) -> Result<()> {
        // Update last activity
        *self.last_activity.lock().await = Instant::now();

        // Spawn task to handle this client's requests
        tokio::spawn(async move {
            Self::process_requests(stream, lsp_manager.clone()).await
        });

        Ok(())
    }

    async fn process_requests(
        mut stream: UnixStream,
        lsp_manager: Arc<Mutex<LspManager>>,
    ) -> Result<()> {
        loop {
            // Read request
            let request = Self::read_request(&mut stream).await?;

            // Process
            let response = Self::handle_request(request, &lsp_manager).await;

            // Send response
            Self::write_response(&mut stream, response).await?;
        }
    }

    async fn handle_request(
        request: DaemonRequest,
        lsp_manager: &Arc<Mutex<LspManager>>,
    ) -> DaemonResponse {
        match request {
            DaemonRequest::Ping => {
                DaemonResponse::Pong { uptime_secs: /* calculate */ }
            }

            DaemonRequest::Hover { uri, line, character } => {
                let lsp = lsp_manager.lock().await;
                match lsp.hover(&uri, line, character, "java").await {
                    Ok(result) => DaemonResponse::HoverResult(result),
                    Err(e) => DaemonResponse::Error(e.to_string()),
                }
            }

            // ... handle other requests
        }
    }

    fn is_idle(&self) -> bool {
        let last = *self.last_activity.lock().unwrap();
        last.elapsed() > self.idle_timeout
    }

    async fn cleanup(&self) -> Result<()> {
        // 1. Shutdown LSP servers
        // 2. Remove socket file
        // 3. Remove PID file
    }
}
```

### 2. Daemon Client (`src/daemon/client.rs`)

```rust
pub struct DaemonClient {
    socket_path: PathBuf,
    stream: UnixStream,
    request_id: AtomicU64,
}

impl DaemonClient {
    /// Connect to existing daemon or start new one
    pub async fn connect_or_start(project_root: &Path) -> Result<Self> {
        let hash = project_hash(project_root);
        let daemon_dir = get_daemon_dir(&hash).await?;
        let socket_path = daemon_dir.join("daemon.sock");
        let pid_file = daemon_dir.join("daemon.pid");

        // Try to connect to existing daemon
        match UnixStream::connect(&socket_path).await {
            Ok(stream) => {
                // Verify daemon is responsive
                if Self::verify_daemon(&stream).await.is_ok() {
                    return Ok(Self {
                        socket_path,
                        stream,
                        request_id: AtomicU64::new(0),
                    });
                }
                // Daemon not responsive, clean up and restart
                Self::cleanup_stale_daemon(&daemon_dir).await?;
            }
            Err(_) => {
                // Daemon not running, check for stale PID file
                if pid_file.exists() {
                    Self::cleanup_stale_daemon(&daemon_dir).await?;
                }
            }
        }

        // Start new daemon
        Self::start_daemon(project_root, &daemon_dir).await?;

        // Connect to newly started daemon
        let stream = Self::wait_for_daemon(&socket_path).await?;

        Ok(Self {
            socket_path,
            stream,
            request_id: AtomicU64::new(0),
        })
    }

    async fn verify_daemon(stream: &UnixStream) -> Result<()> {
        // Send ping, expect pong within 5 seconds
        timeout(Duration::from_secs(5), async {
            // Send ping request
            // Read pong response
        }).await?
    }

    async fn start_daemon(project_root: &Path, daemon_dir: &Path) -> Result<()> {
        tokio::fs::create_dir_all(daemon_dir).await?;

        // Get path to current executable
        let exe = std::env::current_exe()?;

        // Spawn daemon process
        let mut cmd = tokio::process::Command::new(&exe);
        cmd.arg("--daemon-mode")
            .arg("--project-root").arg(project_root)
            .arg("--daemon-dir").arg(daemon_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped()); // Capture errors

        let child = cmd.spawn()?;
        let pid = child.id().ok_or_else(|| anyhow::anyhow!("No PID"))?;

        // Write PID file
        tokio::fs::write(daemon_dir.join("daemon.pid"), pid.to_string()).await?;

        Ok(())
    }

    async fn wait_for_daemon(socket_path: &Path) -> Result<UnixStream> {
        // Wait up to 120 seconds for daemon to be ready
        for i in 0..1200 {
            if let Ok(stream) = UnixStream::connect(socket_path).await {
                return Ok(stream);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;

            if i > 0 && i % 100 == 0 {
                debug!("Still waiting for daemon... {}s", i / 10);
            }
        }

        Err(anyhow::anyhow!("Daemon failed to start within 120s"))
    }

    async fn cleanup_stale_daemon(daemon_dir: &Path) -> Result<()> {
        // Read PID file
        if let Ok(pid_str) = tokio::fs::read_to_string(daemon_dir.join("daemon.pid")).await {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                // Try to kill process
                unsafe {
                    libc::kill(pid, libc::SIGTERM);
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }

        // Remove socket and PID files
        let _ = tokio::fs::remove_file(daemon_dir.join("daemon.sock")).await;
        let _ = tokio::fs::remove_file(daemon_dir.join("daemon.pid")).await;

        Ok(())
    }

    pub async fn hover(&mut self, uri: &str, line: u32, character: u32) -> Result<Option<String>> {
        let request = DaemonRequest::Hover {
            uri: uri.to_string(),
            line,
            character,
        };

        let response = self.send_request(request).await?;

        match response {
            DaemonResponse::HoverResult(result) => Ok(result),
            DaemonResponse::Error(e) => Err(anyhow::anyhow!(e)),
            _ => Err(anyhow::anyhow!("Unexpected response")),
        }
    }

    async fn send_request(&mut self, request: DaemonRequest) -> Result<DaemonResponse> {
        // Serialize request
        let json = serde_json::to_vec(&request)?;

        // Write length prefix
        let len = (json.len() as u32).to_be_bytes();
        self.stream.write_all(&len).await?;
        self.stream.write_all(&json).await?;
        self.stream.flush().await?;

        // Read response length
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        // Read response
        let mut buf = vec![0u8; len];
        self.stream.read_exact(&mut buf).await?;

        // Deserialize
        let response: DaemonResponse = serde_json::from_slice(&buf)?;
        Ok(response)
    }
}
```

### 3. Integration with main.rs

```rust
// src/main.rs

#[derive(Parser)]
struct Args {
    file: Option<String>,

    #[arg(long)]
    daemon_mode: bool,

    #[arg(long)]
    project_root: Option<String>,

    #[arg(long)]
    daemon_dir: Option<String>,

    // ... other args
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // If daemon mode, run as daemon server
    if args.daemon_mode {
        let project_root = PathBuf::from(args.project_root.unwrap());
        let daemon_dir = PathBuf::from(args.daemon_dir.unwrap());

        return run_daemon_server(project_root, daemon_dir).await;
    }

    // Normal mode
    // ... existing code ...
}

async fn run_daemon_server(project_root: PathBuf, daemon_dir: PathBuf) -> Result<()> {
    use daemon::LspDaemonServer;

    // Set up logging to daemon.log
    let log_file = daemon_dir.join("daemon.log");
    // ... configure logging ...

    info!("Starting LSP daemon for project: {:?}", project_root);

    let socket_path = daemon_dir.join("daemon.sock");
    let mut server = LspDaemonServer::start(project_root, socket_path).await?;

    server.run().await
}
```

## Edge Cases & Tests

See DAEMON_EDGE_CASES_TESTS.md for comprehensive test plan.

## Performance Targets

| Metric | Target | Acceptable | Current |
|--------|--------|------------|---------|
| First open (cold) | 60s | 90s | 60-120s |
| Second open (warm) | <1s | <5s | 60-120s ❌ |
| Third open (warm) | <1s | <5s | 60-120s ❌ |
| Memory (daemon) | <500MB | <1GB | TBD |
| Daemon startup | <60s | <90s | TBD |
| Socket latency | <10ms | <50ms | TBD |

## Future Enhancements

1. **Global daemon mode** - Single daemon for all projects
2. **Hot reload** - Reload jdtls without restart
3. **Resource limits** - Memory/CPU limits per daemon
4. **Monitoring** - Prometheus metrics
5. **Multi-language** - Support Python, TypeScript, etc.

## Migration Plan

1. Implement daemon mode as **opt-in** initially
2. Add `--use-daemon` flag (default: true)
3. Test thoroughly in production
4. Make daemon mode default
5. Remove old direct-spawn mode

## Rollout Strategy

**Week 1:**
- Implement basic daemon server and client
- Test with single file

**Week 2:**
- Implement edge case handling
- Comprehensive test suite

**Week 3:**
- Performance testing and optimization
- Fix issues

**Week 4:**
- Beta release with opt-in daemon mode
- Gather feedback

**Week 5+:**
- Make daemon mode default
- Remove old code path
