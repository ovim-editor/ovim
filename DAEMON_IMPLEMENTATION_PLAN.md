# LSP Daemon Mode - Implementation Plan

## Executive Summary

**Goal:** Make ovim usable for the "quick edit" workflow where users open/close files frequently.

**Current Problem:** Every `ovim File.java` waits 60-120s for jdtls initialization.

**Solution:** Keep jdtls alive in a daemon process, reuse it across editor sessions.

**Target Performance:**
- 1st open: 60-90s (acceptable, one-time cost per project)
- 2nd+ opens: <5s (instant editing)

---

## Phase 1: Foundation (Week 1)

### 1.1 Create Daemon Module Structure

```
src/daemon/
├── mod.rs           # Public API
├── protocol.rs      # Request/Response types
├── server.rs        # Daemon server process
├── client.rs        # Client that connects to daemon
└── manager.rs       # Daemon lifecycle management
```

**Tasks:**
- [ ] Create module structure
- [ ] Define DaemonRequest/DaemonResponse protocol
- [ ] Implement message serialization (length-prefix + JSON)
- [ ] Unit tests for protocol

**Acceptance Criteria:**
- Can serialize/deserialize all message types
- Protocol handles errors gracefully
- Tests pass

### 1.2 Implement Basic Daemon Server

**File:** `src/daemon/server.rs`

**Tasks:**
- [ ] Create Unix domain socket listener
- [ ] Accept client connections
- [ ] Read/write protocol messages
- [ ] Integrate with existing LspManager
- [ ] Implement Ping/Pong for health checks

**Key Functions:**
```rust
impl LspDaemonServer {
    pub async fn start(
        project_root: PathBuf,
        socket_path: PathBuf,
    ) -> Result<Self>;

    pub async fn run(&mut self) -> Result<()>;

    async fn handle_client(&self, stream: UnixStream) -> Result<()>;

    async fn process_request(
        &self,
        request: DaemonRequest,
    ) -> DaemonResponse;
}
```

**Acceptance Criteria:**
- Daemon starts and listens on socket
- Accepts connections
- Responds to Ping with Pong
- Can shutdown cleanly

### 1.3 Implement Basic Daemon Client

**File:** `src/daemon/client.rs`

**Tasks:**
- [ ] Detect project root
- [ ] Generate project hash
- [ ] Check for existing daemon
- [ ] Connect to daemon via socket
- [ ] Send requests, receive responses

**Key Functions:**
```rust
impl DaemonClient {
    pub async fn connect_or_start(
        project_root: &Path,
    ) -> Result<Self>;

    pub async fn hover(
        &mut self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<String>>;

    // ... other LSP operations
}
```

**Acceptance Criteria:**
- Can find project root correctly
- Connects to existing daemon if available
- Sends/receives messages correctly
- Basic error handling

---

## Phase 2: Daemon Lifecycle (Week 2)

### 2.1 Auto-Start Daemon

**Tasks:**
- [ ] Spawn daemon process in background
- [ ] Daemon detaches from parent
- [ ] Write PID file
- [ ] Wait for socket to be available
- [ ] Handle race conditions (multiple clients starting simultaneously)

**Implementation:**
```rust
async fn start_daemon(
    project_root: &Path,
    daemon_dir: &Path,
) -> Result<()> {
    // Use file locking to prevent race
    let lock_file = daemon_dir.join("daemon.lock");
    let lock = FileLock::try_lock(&lock_file)?;

    // Double-check daemon didn't start while we waited for lock
    if socket_exists() {
        return Ok(());
    }

    // Spawn daemon process
    let child = Command::new(exe_path)
        .arg("--daemon-mode")
        .arg("--project-root").arg(project_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    // Write PID
    write_pid_file(child.id())?;

    // Wait for socket (up to 120s)
    wait_for_socket(&socket_path).await?;

    drop(lock);
    Ok(())
}
```

**Acceptance Criteria:**
- Daemon starts successfully
- Only one daemon starts even with concurrent clients
- PID file written correctly
- Socket becomes available

### 2.2 Idle Timeout and Auto-Shutdown

**Tasks:**
- [ ] Track last activity time
- [ ] Check for idle timeout periodically
- [ ] Graceful shutdown when idle
- [ ] Activity resets timeout

**Implementation:**
```rust
pub struct LspDaemonServer {
    last_activity: Arc<Mutex<Instant>>,
    idle_timeout: Duration,  // Default: 30 minutes
}

impl LspDaemonServer {
    async fn run(&mut self) -> Result<()> {
        loop {
            tokio::select! {
                // Handle connections
                result = listener.accept() => { ... }

                // Check idle timeout every minute
                _ = interval.tick() => {
                    let idle_time = self.last_activity.lock().await.elapsed();
                    if idle_time > self.idle_timeout {
                        info!("Idle timeout reached, shutting down");
                        break;
                    }
                }
            }
        }

        self.cleanup().await
    }
}
```

**Acceptance Criteria:**
- Daemon shuts down after idle timeout
- Activity resets the timeout
- Graceful cleanup on shutdown

### 2.3 Stale Daemon Detection and Cleanup

**Tasks:**
- [ ] Check if PID in file is still running
- [ ] Verify daemon is responsive (ping)
- [ ] Clean up stale socket/PID files
- [ ] Restart daemon if stale

**Implementation:**
```rust
async fn verify_daemon_alive(daemon_dir: &Path) -> Result<bool> {
    // Read PID file
    let pid = read_pid_file(daemon_dir)?;

    // Check if process exists
    if !process_exists(pid) {
        return Ok(false);
    }

    // Try to ping
    match ping_daemon(socket_path).await {
        Ok(_) => Ok(true),
        Err(_) => {
            // Process exists but not responsive
            kill_process(pid)?;
            Ok(false)
        }
    }
}

async fn cleanup_stale_daemon(daemon_dir: &Path) -> Result<()> {
    // Remove socket
    remove_file(daemon_dir.join("daemon.sock")).await?;

    // Remove PID
    remove_file(daemon_dir.join("daemon.pid")).await?;

    Ok(())
}
```

**Acceptance Criteria:**
- Detects stale daemon correctly
- Cleans up stale files
- Can restart after detecting stale daemon

---

## Phase 3: Integration with ovim (Week 2-3)

### 3.1 Add --daemon-mode Flag

**File:** `src/main.rs`

**Tasks:**
- [ ] Add CLI argument for daemon mode
- [ ] Run as daemon server if flag set
- [ ] Normal mode uses daemon client

**Implementation:**
```rust
#[derive(Parser)]
struct Args {
    file: Option<String>,

    #[arg(long)]
    daemon_mode: bool,

    #[arg(long)]
    project_root: Option<PathBuf>,

    #[arg(long)]
    daemon_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    if args.daemon_mode {
        // Run as daemon
        return run_daemon_server(
            args.project_root.unwrap(),
            args.daemon_dir.unwrap(),
        ).await;
    }

    // Normal mode - use daemon client
    run_normal_mode(args.file).await
}
```

**Acceptance Criteria:**
- Daemon mode runs as background server
- Normal mode uses daemon client
- Graceful fallback if daemon unavailable

### 3.2 Replace Direct LSP Manager with Daemon Client

**File:** `src/editor/mod.rs`

**Tasks:**
- [ ] Add DaemonClient as alternative to LspManager
- [ ] Route LSP requests through daemon
- [ ] Handle daemon connection errors
- [ ] Fallback to direct mode if daemon fails

**Implementation:**
```rust
pub struct Editor {
    // OLD: lsp_manager: Option<Arc<Mutex<LspManager>>>,
    // NEW:
    lsp_backend: Option<LspBackend>,
}

enum LspBackend {
    Daemon(DaemonClient),
    Direct(Arc<Mutex<LspManager>>),
}

impl Editor {
    pub async fn hover_impl(&mut self) -> Result<bool> {
        match &mut self.lsp_backend {
            Some(LspBackend::Daemon(client)) => {
                client.hover(uri, line, character).await
            }
            Some(LspBackend::Direct(lsp)) => {
                // Existing implementation
                let lsp = lsp.lock().await;
                lsp.hover(uri, line, character, language_id).await
            }
            None => Ok(false),
        }
    }
}
```

**Acceptance Criteria:**
- All LSP features work through daemon
- Fallback to direct mode works
- Performance improvement measurable

### 3.3 Project Root Detection

**File:** `src/daemon/manager.rs`

**Tasks:**
- [ ] Find project root from file path
- [ ] Support build.gradle, pom.xml, .git
- [ ] Handle nested projects correctly
- [ ] Handle standalone files (no project)

**Implementation:**
```rust
pub fn find_project_root(file_path: &Path) -> Option<PathBuf> {
    let mut current = file_path.parent()?;

    loop {
        // Check for project markers
        for marker in &["build.gradle", "build.gradle.kts", "pom.xml", ".git"] {
            if current.join(marker).exists() {
                return Some(current.to_path_buf());
            }
        }

        // Go up one level
        current = current.parent()?;
    }
}
```

**Acceptance Criteria:**
- Correctly finds project root
- Handles edge cases (nested, no project)
- Uses canonical paths (handles symlinks)

---

## Phase 4: Edge Cases and Robustness (Week 3)

### 4.1 Concurrent Access Handling

**Tasks:**
- [ ] File locking for daemon start
- [ ] Multiple clients can connect simultaneously
- [ ] Request handling is thread-safe
- [ ] No race conditions

**Implementation:**
```rust
// Use advisory file lock during daemon start
use fs2::FileExt;

async fn start_daemon_with_lock(daemon_dir: &Path) -> Result<()> {
    let lock_path = daemon_dir.join(".daemon.lock");
    let lock_file = std::fs::File::create(&lock_path)?;

    // Acquire exclusive lock
    lock_file.lock_exclusive()?;

    // Check again if daemon started while waiting
    if daemon_socket_exists() {
        lock_file.unlock()?;
        return Ok(());
    }

    // Start daemon
    start_daemon_process()?;

    lock_file.unlock()?;
    std::fs::remove_file(lock_path)?;

    Ok(())
}
```

**Acceptance Criteria:**
- Concurrent starts don't create multiple daemons
- Multiple clients can use daemon simultaneously
- All edge cases tested

### 4.2 Daemon Crash Recovery

**Tasks:**
- [ ] Detect daemon crash
- [ ] Auto-restart daemon
- [ ] Notify user of restart
- [ ] Preserve user work

**Implementation:**
```rust
impl DaemonClient {
    async fn send_request(&mut self, request: DaemonRequest) -> Result<DaemonResponse> {
        match self.send_request_internal(request.clone()).await {
            Ok(response) => Ok(response),
            Err(_) => {
                // Daemon might have crashed, try to restart
                warn!("Daemon connection failed, attempting restart");

                self.reconnect().await?;

                // Retry request once
                self.send_request_internal(request).await
            }
        }
    }

    async fn reconnect(&mut self) -> Result<()> {
        // Clean up old connection
        drop(self.stream.take());

        // Check if daemon is still running
        if !verify_daemon_alive(&self.daemon_dir).await? {
            // Daemon crashed, restart it
            cleanup_stale_daemon(&self.daemon_dir).await?;
            start_daemon(&self.project_root, &self.daemon_dir).await?;
        }

        // Reconnect
        self.stream = Some(UnixStream::connect(&self.socket_path).await?);
        Ok(())
    }
}
```

**Acceptance Criteria:**
- Detects daemon crash
- Restarts automatically
- User experience is seamless

### 4.3 Resource Limits and Quotas

**Tasks:**
- [ ] Set memory limits for daemon
- [ ] Limit number of concurrent connections
- [ ] Handle out-of-memory gracefully
- [ ] Clean up resources on shutdown

**Implementation:**
```rust
const MAX_MEMORY_MB: usize = 1024;  // 1GB
const MAX_CONNECTIONS: usize = 100;

impl LspDaemonServer {
    async fn run(&mut self) -> Result<()> {
        let mut active_connections = 0;

        loop {
            tokio::select! {
                result = listener.accept() => {
                    if active_connections >= MAX_CONNECTIONS {
                        warn!("Max connections reached, rejecting");
                        continue;
                    }

                    active_connections += 1;
                    // Handle connection...
                }

                // Check memory usage every minute
                _ = interval.tick() => {
                    let mem_usage = get_memory_usage()?;
                    if mem_usage > MAX_MEMORY_MB * 1024 * 1024 {
                        error!("Memory limit exceeded, shutting down");
                        break;
                    }
                }
            }
        }
    }
}
```

**Acceptance Criteria:**
- Memory limits enforced
- Connection limits enforced
- Graceful degradation under pressure

---

## Phase 5: Testing and Validation (Week 4)

### 5.1 Unit Tests

**Tasks:**
- [ ] Protocol serialization tests
- [ ] Project root detection tests
- [ ] Message handling tests
- [ ] Mock daemon/client tests

**Coverage Target:** >80% line coverage

### 5.2 Integration Tests

**Tasks:**
- [ ] End-to-end daemon lifecycle test
- [ ] Multi-client test
- [ ] Crash recovery test
- [ ] Performance benchmarks

**Test Scripts:**
- `tests/daemon/test_basic_lifecycle.sh`
- `tests/daemon/test_concurrent_access.sh`
- `tests/daemon/test_crash_recovery.sh`
- `tests/daemon/test_performance.sh`

### 5.3 Performance Benchmarks

**Metrics to Measure:**
- Time to first keystroke (target: <1s)
- Second open time (target: <5s)
- Request latency (target: p95 < 100ms)
- Memory usage (target: <1GB total)

**Benchmark Script:**
```bash
#!/bin/bash
# Performance benchmark

rm -rf ~/.cache/ovim/java

echo "=== Cold Start Benchmark ==="
time ovim Test.java < <(sleep 2 && echo ":q")

echo "=== Warm Start Benchmark ==="
START=$(date +%s%N)
ovim Test.java < <(sleep 2 && echo ":q")
END=$(date +%s%N)
ELAPSED=$(( (END - START) / 1000000 ))  # Convert to ms
echo "Time: ${ELAPSED}ms"

if [ "$ELAPSED" -lt 5000 ]; then
    echo "✅ PASS - Under 5 seconds"
else
    echo "❌ FAIL - Over 5 seconds"
fi
```

---

## Phase 6: Polish and Documentation (Week 4)

### 6.1 User-Facing Documentation

**Create:**
- User guide for daemon mode
- Troubleshooting guide
- FAQ

**Topics:**
- How daemon mode works
- Manual daemon management (`ovim --daemon stop`)
- What to do if daemon crashes
- How to check daemon status

### 6.2 Developer Documentation

**Create:**
- Architecture documentation
- API documentation
- Contribution guide for daemon features

### 6.3 Error Messages and Logging

**Tasks:**
- [ ] Clear error messages for all failure modes
- [ ] Helpful suggestions (e.g., "Run `ovim --daemon restart`")
- [ ] Structured logging with appropriate levels
- [ ] Log rotation for daemon logs

---

## Success Criteria

### Must Have
- [ ] Second file open in <5 seconds
- [ ] No process leaks
- [ ] No memory leaks
- [ ] Graceful error handling
- [ ] All edge case tests pass

### Should Have
- [ ] Sub-1s subsequent opens
- [ ] Clear user documentation
- [ ] Monitoring/debugging tools
- [ ] Platform support (Linux, Mac)

### Nice to Have
- [ ] Hot reload of jdtls
- [ ] Global daemon mode
- [ ] Prometheus metrics
- [ ] systemd integration

---

## Rollout Plan

### Week 1-2: Alpha (Internal Testing)
- Feature flag: `--use-daemon=false` (default: off)
- Test with small group
- Fix critical bugs

### Week 3: Beta (Opt-In)
- Feature flag: `--use-daemon=true` available
- Announce to community
- Gather feedback

### Week 4: GA (General Availability)
- Make daemon mode default
- Keep direct mode as fallback
- Monitor issues

### Week 5+: Cleanup
- Remove direct mode code path (if daemon stable)
- Optimize performance
- Add advanced features

---

## Risk Mitigation

| Risk | Impact | Mitigation |
|------|--------|------------|
| Daemon crashes frequently | High | Robust crash detection and auto-restart |
| Socket permission issues | Medium | Clear error messages, fallback to direct mode |
| Performance regression | High | Comprehensive benchmarks, revert if needed |
| Platform incompatibility | Medium | Test on multiple platforms, graceful degradation |
| Security vulnerabilities | High | Security audit, input sanitization |

---

## Dependencies

- `tokio` - Async runtime (already used)
- `serde_json` - Message serialization (already used)
- `fs2` - File locking
- `libc` - Process management

---

## Estimated Effort

- **Engineering:** 4 weeks (1 engineer)
- **Testing:** 1 week
- **Documentation:** 0.5 weeks
- **Total:** ~5-6 weeks

---

## Next Immediate Steps

1. Run `./test_quick_edit.sh` to baseline current performance
2. Create `src/daemon/` module structure
3. Implement basic protocol types
4. Write unit tests for protocol
5. Implement basic daemon server (ping/pong only)
6. Implement basic daemon client
7. Integration test: start daemon, ping, shutdown
8. Iterate...

**Ready to start implementation!**
