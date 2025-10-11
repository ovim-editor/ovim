# LSP Daemon Mode - Edge Cases Deep Dive

## Critical Issues to Address

After deep analysis, here are the edge cases that could break daemon mode if not handled correctly.

---

## 1. Rogue jdtls Processes 🔴 CRITICAL

### Problem: jdtls Won't Die

**Scenario:**
```bash
# jdtls hangs in uninterruptible disk I/O
# Process state: D (uninterruptible sleep)

$ kill -TERM 1234   # Doesn't work
$ kill -KILL 1234   # Still doesn't work!
$ ps aux | grep jdtls
user  1234  D  ... jdtls    # Still there!
```

**Impact:**
- Blocks daemon restart
- Wastes resources (500MB+ RAM)
- Socket/workspace locked
- User can't work

**Detection:**
```rust
async fn kill_process_forcefully(pid: i32) -> Result<ProcessKillStatus> {
    // Try SIGTERM first (graceful)
    unsafe { libc::kill(pid, libc::SIGTERM) };

    for _ in 0..10 {  // Wait up to 5 seconds
        tokio::time::sleep(Duration::from_millis(500)).await;

        if !process_exists(pid) {
            return Ok(ProcessKillStatus::Terminated);
        }
    }

    // Escalate to SIGKILL
    unsafe { libc::kill(pid, libc::SIGKILL) };

    for _ in 0..4 {  // Wait up to 2 seconds
        tokio::time::sleep(Duration::from_millis(500)).await;

        if !process_exists(pid) {
            return Ok(ProcessKillStatus::Killed);
        }
    }

    // Still alive? Check if zombie
    let state = get_process_state(pid)?;
    if state == 'Z' {
        // Zombie - it's dead, parent needs to wait()
        return Ok(ProcessKillStatus::Zombie);
    }

    if state == 'D' {
        // Uninterruptible sleep - can't kill
        return Ok(ProcessKillStatus::Stuck);
    }

    // Process truly stuck
    Err(anyhow::anyhow!("Process {} won't die (state: {})", pid, state))
}
```

**Recovery Strategy:**
```rust
match kill_process_forcefully(jdtls_pid).await {
    Ok(ProcessKillStatus::Terminated | ProcessKillStatus::Killed) => {
        // Normal case
        start_new_jdtls().await?;
    }

    Ok(ProcessKillStatus::Zombie) => {
        // Zombie - it's dead but not reaped
        // Just ignore it and start new instance
        warn!("Old jdtls is zombie, starting new instance");
        start_new_jdtls().await?;
    }

    Ok(ProcessKillStatus::Stuck) => {
        // Process in uninterruptible sleep
        // Can't kill it, must work around
        error!("jdtls stuck in state D (disk I/O?), cannot kill");
        error!("Will start new instance with different workspace");

        // Use alternative workspace
        let alt_workspace = format!("{}.{}", workspace_dir, timestamp());
        start_new_jdtls_with_workspace(&alt_workspace).await?;

        // Mark old process as "rogue" for monitoring
        record_rogue_process(jdtls_pid, "uninterruptible_sleep");
    }

    Err(e) => {
        error!("Failed to kill jdtls: {}", e);
        // Fall back to direct mode
        return Err(e);
    }
}
```

**Test Case:**
```bash
# Simulate uninterruptible sleep
kill -STOP $JDTLS_PID  # Freeze process
# Daemon should detect and work around
```

---

## 2. PID Reuse Problem 🔴 CRITICAL

### Problem: Wrong Process Kill

**Scenario:**
```bash
# Daemon starts with PID 1234, writes PID file
# Daemon crashes
# New unrelated process gets PID 1234 (vim, firefox, whatever)
# ovim reads PID file, sees 1234
# ovim kills PID 1234 → KILLS WRONG PROCESS!
```

**This is a CRITICAL security/safety issue!**

**Solution: Store PID + Verification Info**

```rust
#[derive(Serialize, Deserialize)]
struct DaemonPidInfo {
    pid: i32,
    start_time: SystemTime,  // Process start time
    cmd_hash: u64,           // Hash of command line
}

fn write_pid_file(pid: i32, daemon_dir: &Path) -> Result<()> {
    let start_time = get_process_start_time(pid)?;
    let cmd_line = get_process_cmdline(pid)?;
    let cmd_hash = hash(&cmd_line);

    let info = DaemonPidInfo {
        pid,
        start_time,
        cmd_hash,
    };

    let json = serde_json::to_string(&info)?;
    std::fs::write(daemon_dir.join("daemon.pid"), json)?;
    Ok(())
}

fn verify_daemon_pid(daemon_dir: &Path) -> Result<Option<i32>> {
    let json = std::fs::read_to_string(daemon_dir.join("daemon.pid"))?;
    let info: DaemonPidInfo = serde_json::from_str(&json)?;

    // Check if process exists
    if !process_exists(info.pid) {
        return Ok(None);
    }

    // Verify start time matches
    let current_start_time = get_process_start_time(info.pid)?;
    if current_start_time != info.start_time {
        warn!("PID {} reused by different process", info.pid);
        return Ok(None);
    }

    // Verify command line matches
    let current_cmd = get_process_cmdline(info.pid)?;
    let current_hash = hash(&current_cmd);
    if current_hash != info.cmd_hash {
        warn!("PID {} is different process", info.pid);
        return Ok(None);
    }

    // All checks passed - this is our daemon
    Ok(Some(info.pid))
}

fn get_process_start_time(pid: i32) -> Result<SystemTime> {
    // Read /proc/{pid}/stat
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid))?;
    let fields: Vec<&str> = stat.split_whitespace().collect();

    // Field 22 is start time in jiffies since boot
    let start_jiffies: u64 = fields[21].parse()?;

    // Convert to SystemTime
    // ... (platform-specific conversion)

    Ok(start_time)
}
```

**Test Case:**
```rust
#[test]
fn test_pid_reuse_detection() {
    // Start daemon, write PID file
    let pid = start_daemon();

    // Simulate daemon crash
    kill_daemon(pid);

    // Simulate PID reuse - start different process with same PID
    // (need to use mocking or specific test harness)

    // Try to verify - should detect mismatch
    assert!(verify_daemon_pid().is_none());
}
```

---

## 3. Multiple Daemons for Same Project 🔴 CRITICAL

### Problem: Race Condition on Daemon Start

**Scenario:**
```bash
# Terminal 1
$ ovim File1.java &  # Starts daemon creation

# Terminal 2 (simultaneously)
$ ovim File2.java &  # Also starts daemon creation

# Result: TWO daemons for same project!
# Both jdtls instances conflict on workspace
# Corruption, crashes, wasted resources
```

**Current Design:** File locking - but needs improvement

**Robust Solution: Atomic Socket Creation**

```rust
async fn ensure_single_daemon(daemon_dir: &Path) -> Result<DaemonLock> {
    let lock_file = daemon_dir.join(".daemon.lock");
    let socket_path = daemon_dir.join("daemon.sock");

    // Create lock file with exclusive access
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&lock_file)?;

    // Acquire exclusive lock (blocks if another process has it)
    lock.try_lock_exclusive()?;

    // Double-check: Did daemon start while we waited for lock?
    if socket_exists(&socket_path) {
        // Verify it's responsive
        if ping_daemon(&socket_path).await.is_ok() {
            // Daemon is alive and working
            lock.unlock()?;
            return Ok(DaemonLock::Existing);
        }

        // Socket exists but daemon not responsive - clean up
        cleanup_stale_socket(&socket_path).await?;
    }

    // We have lock and no daemon exists - start it
    start_daemon_process(daemon_dir).await?;

    // Keep lock until daemon is fully started
    wait_for_daemon_ready(&socket_path).await?;

    // Release lock
    lock.unlock()?;
    std::fs::remove_file(lock_file)?;

    Ok(DaemonLock::Created)
}
```

**Edge Case: Stale Lock File**

What if process crashes while holding lock?

```rust
use fs2::FileExt;

fn acquire_lock_with_timeout(lock_file: &Path, timeout: Duration) -> Result<File> {
    let file = File::create(lock_file)?;

    let start = Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(_) => return Ok(file),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // Lock held by another process

                if start.elapsed() > timeout {
                    // Timeout - assume stale lock
                    warn!("Lock timeout - assuming stale");

                    // Force break lock (dangerous but necessary)
                    std::fs::remove_file(lock_file)?;
                    let file = File::create(lock_file)?;
                    file.lock_exclusive()?;
                    return Ok(file);
                }

                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e.into()),
        }
    }
}
```

**Test Case:**
```rust
#[tokio::test]
async fn test_concurrent_daemon_start() {
    let daemon_dir = PathBuf::from("/tmp/test-daemon");

    // Start 10 concurrent clients
    let mut tasks = vec![];
    for _ in 0..10 {
        let dir = daemon_dir.clone();
        tasks.push(tokio::spawn(async move {
            ensure_single_daemon(&dir).await
        }));
    }

    // Wait for all to complete
    let results: Vec<_> = futures::future::join_all(tasks).await;

    // Exactly one should have Created, rest should see Existing
    let created_count = results.iter()
        .filter(|r| matches!(r, Ok(Ok(DaemonLock::Created))))
        .count();

    assert_eq!(created_count, 1, "Exactly one daemon should be created");

    // Verify only one daemon process exists
    let daemon_count = count_daemon_processes();
    assert_eq!(daemon_count, 1);
}
```

---

## 4. Stale Daemon Detection 🟡 IMPORTANT

### Problem: Daemon Dead but Files Remain

**Scenarios:**
1. System crash → daemon killed, socket/PID files remain
2. kill -9 daemon → no cleanup
3. Disk full → partial cleanup
4. NFS mount dead → can't access files

**Comprehensive Staleness Check:**

```rust
async fn check_daemon_staleness(daemon_dir: &Path) -> DaemonStatus {
    let socket_path = daemon_dir.join("daemon.sock");
    let pid_file = daemon_dir.join("daemon.pid");

    // Check 1: Do files exist?
    let socket_exists = socket_path.exists();
    let pid_exists = pid_file.exists();

    if !socket_exists && !pid_exists {
        return DaemonStatus::NotExists;
    }

    // Check 2: Is PID valid?
    let pid = match verify_daemon_pid(daemon_dir).await {
        Ok(Some(pid)) => pid,
        Ok(None) => {
            // PID file exists but process doesn't match
            return DaemonStatus::Stale {
                reason: "PID mismatch or reused",
            };
        }
        Err(e) => {
            return DaemonStatus::Stale {
                reason: format!("Cannot read PID: {}", e),
            };
        }
    };

    // Check 3: Is process actually our daemon?
    match verify_process_is_daemon(pid) {
        Ok(true) => {}  // Good
        Ok(false) => {
            return DaemonStatus::Stale {
                reason: "Process is not ovim daemon",
            };
        }
        Err(e) => {
            return DaemonStatus::Stale {
                reason: format!("Cannot verify process: {}", e),
            };
        }
    }

    // Check 4: Can we connect to socket?
    match tokio::time::timeout(
        Duration::from_secs(5),
        UnixStream::connect(&socket_path)
    ).await {
        Ok(Ok(_)) => {}  // Good - connected
        Ok(Err(e)) => {
            return DaemonStatus::Stale {
                reason: format!("Socket exists but cannot connect: {}", e),
            };
        }
        Err(_) => {
            return DaemonStatus::Stale {
                reason: "Socket connection timeout",
            };
        }
    }

    // Check 5: Is daemon responsive? (ping/pong)
    match ping_daemon_with_timeout(&socket_path, Duration::from_secs(5)).await {
        Ok(_) => DaemonStatus::Alive { pid },
        Err(e) => DaemonStatus::Unresponsive {
            pid,
            reason: format!("Ping failed: {}", e),
        },
    }
}

enum DaemonStatus {
    NotExists,
    Alive { pid: i32 },
    Stale { reason: String },
    Unresponsive { pid: i32, reason: String },
}
```

**Recovery Actions:**

```rust
match check_daemon_staleness(daemon_dir).await {
    DaemonStatus::Alive { pid } => {
        // All good - connect to it
        connect_to_daemon(socket_path).await
    }

    DaemonStatus::NotExists => {
        // No daemon - start fresh
        start_new_daemon(daemon_dir).await
    }

    DaemonStatus::Stale { reason } => {
        warn!("Stale daemon detected: {}", reason);
        cleanup_stale_daemon(daemon_dir).await?;
        start_new_daemon(daemon_dir).await
    }

    DaemonStatus::Unresponsive { pid, reason } => {
        warn!("Daemon {} unresponsive: {}", pid, reason);

        // Try to kill it
        kill_process_forcefully(pid).await?;
        cleanup_stale_daemon(daemon_dir).await?;
        start_new_daemon(daemon_dir).await
    }
}
```

---

## 5. Workspace Corruption 🟡 IMPORTANT

### Problem: jdtls Workspace Gets Corrupted

**Causes:**
- jdtls crash during metadata write
- Disk full
- Multiple jdtls instances (if race happens)
- User manually modifies workspace
- Filesystem issues

**Detection:**

```rust
async fn check_workspace_health(workspace_dir: &Path) -> WorkspaceHealth {
    // Check 1: Does workspace exist?
    if !workspace_dir.exists() {
        return WorkspaceHealth::Missing;
    }

    // Check 2: Can we read metadata?
    let metadata_dir = workspace_dir.join(".metadata");
    if !metadata_dir.exists() {
        return WorkspaceHealth::Incomplete;
    }

    // Check 3: Check for known corruption indicators
    let markers = [
        ".metadata/.lock",
        ".metadata/.version",
        ".metadata/.plugins/org.eclipse.core.resources",
    ];

    for marker in &markers {
        let path = workspace_dir.join(marker);
        if !path.exists() {
            return WorkspaceHealth::Corrupted {
                reason: format!("Missing {}", marker),
            };
        }
    }

    // Check 4: Try to read version file
    match std::fs::read_to_string(metadata_dir.join(".version")) {
        Ok(content) => {
            if content.trim().is_empty() {
                return WorkspaceHealth::Corrupted {
                    reason: "Empty version file",
                };
            }
        }
        Err(e) => {
            return WorkspaceHealth::Corrupted {
                reason: format!("Cannot read version: {}", e),
            };
        }
    }

    WorkspaceHealth::Healthy
}

enum WorkspaceHealth {
    Healthy,
    Missing,
    Incomplete,
    Corrupted { reason: String },
}
```

**Recovery:**

```rust
async fn handle_workspace_corruption(
    workspace_dir: &Path,
    daemon_dir: &Path
) -> Result<()> {
    // Backup corrupted workspace for debugging
    let backup_dir = daemon_dir.join(format!(
        "workspace.corrupted.{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()
    ));

    warn!("Backing up corrupted workspace to {}", backup_dir.display());
    tokio::fs::rename(workspace_dir, &backup_dir).await?;

    // Create fresh workspace
    tokio::fs::create_dir_all(workspace_dir).await?;

    // Start jdtls with fresh workspace
    info!("Starting jdtls with fresh workspace");

    Ok(())
}
```

**Corruption Detection During Runtime:**

```rust
impl DaemonServer {
    async fn detect_jdtls_corruption(&self) -> bool {
        // Symptoms of corruption:

        // 1. jdtls restarting repeatedly
        if self.jdtls_restart_count.load(Ordering::SeqCst) > 3 {
            warn!("jdtls restarted {} times - possible corruption",
                  self.jdtls_restart_count.load(Ordering::SeqCst));
            return true;
        }

        // 2. All requests timing out
        let timeout_rate = self.metrics.timeout_rate();
        if timeout_rate > 0.5 {  // 50%+ timeouts
            warn!("High timeout rate ({}) - possible corruption", timeout_rate);
            return true;
        }

        // 3. jdtls logs show corruption errors
        if let Ok(logs) = read_jdtls_logs(&self.daemon_dir) {
            if logs.contains("workspace") && logs.contains("corrupt") {
                warn!("jdtls logs indicate corruption");
                return true;
            }
        }

        false
    }

    async fn handle_detected_corruption(&mut self) -> Result<()> {
        error!("Workspace corruption detected - rebuilding");

        // Stop jdtls
        self.stop_jdtls().await?;

        // Recover workspace
        handle_workspace_corruption(&self.workspace_dir, &self.daemon_dir).await?;

        // Restart jdtls
        self.start_jdtls().await?;

        // Reset restart counter
        self.jdtls_restart_count.store(0, Ordering::SeqCst);

        Ok(())
    }
}
```

---

## 6. Resource Management 🟡 IMPORTANT

### Problem: Too Many Daemons

**Scenario:**
```bash
# User works on 10 different projects in a day
# Each daemon uses 500MB
# Total: 5GB of daemons!
# Most are idle but within 30min timeout
```

**Solution: Global Daemon Manager**

```rust
struct GlobalDaemonManager {
    max_daemons: usize,  // Default: 5
    max_total_memory_mb: usize,  // Default: 2048 (2GB)
    daemons: HashMap<String, DaemonInfo>,
}

struct DaemonInfo {
    project_hash: String,
    pid: i32,
    last_activity: Instant,
    memory_mb: usize,
}

impl GlobalDaemonManager {
    async fn enforce_limits(&mut self) -> Result<()> {
        // Get all daemon info
        self.refresh_daemon_info().await?;

        // Check memory limit
        let total_memory: usize = self.daemons.values()
            .map(|d| d.memory_mb)
            .sum();

        if total_memory > self.max_total_memory_mb {
            warn!("Total daemon memory {} > limit {}",
                  total_memory, self.max_total_memory_mb);

            // Kill least recently used daemons
            self.evict_lru_daemons(total_memory - self.max_total_memory_mb).await?;
        }

        // Check daemon count
        if self.daemons.len() > self.max_daemons {
            warn!("Daemon count {} > limit {}",
                  self.daemons.len(), self.max_daemons);

            let to_evict = self.daemons.len() - self.max_daemons;
            self.evict_n_lru_daemons(to_evict).await?;
        }

        Ok(())
    }

    async fn evict_lru_daemons(&mut self, memory_to_free_mb: usize) -> Result<()> {
        // Sort by last activity (least recent first)
        let mut daemons: Vec<_> = self.daemons.values().collect();
        daemons.sort_by_key(|d| d.last_activity);

        let mut freed = 0;
        for daemon in daemons {
            if freed >= memory_to_free_mb {
                break;
            }

            info!("Evicting daemon for project {} (LRU)", daemon.project_hash);
            self.stop_daemon(&daemon.project_hash).await?;
            freed += daemon.memory_mb;
        }

        Ok(())
    }
}
```

**User Configuration:**

```toml
# ~/.config/ovim/daemon.toml

[daemon]
max_daemons = 5
max_total_memory_mb = 2048
idle_timeout_minutes = 30
enable_lru_eviction = true
```

---

## 7. Activity Tracking Bugs 🟠 MODERATE

### Problem: Daemon Times Out While Client Active

**Scenario:**
```bash
# Terminal 1
$ ovim File1.java
# Work for 15 minutes, close

# Terminal 2
$ ovim File2.java
# Work for 20 minutes (total 35 min since daemon start)
# Daemon times out! (30 min idle from Terminal 1 perspective)
# Connection lost!
```

**Current Design Flaw:** Per-daemon activity, not per-connection

**Solution: Per-Connection Heartbeat**

```rust
struct DaemonServer {
    active_connections: Arc<Mutex<HashSet<ConnectionId>>>,
    connection_activity: Arc<Mutex<HashMap<ConnectionId, Instant>>>,
}

impl DaemonServer {
    async fn handle_client(&self, stream: UnixStream, conn_id: ConnectionId) {
        // Register connection
        {
            let mut conns = self.active_connections.lock().await;
            conns.insert(conn_id);
        }

        // Update activity
        self.update_connection_activity(conn_id).await;

        // Process requests...

        // Unregister on disconnect
        {
            let mut conns = self.active_connections.lock().await;
            conns.remove(&conn_id);
        }
    }

    async fn update_connection_activity(&self, conn_id: ConnectionId) {
        let mut activity = self.connection_activity.lock().await;
        activity.insert(conn_id, Instant::now());
    }

    async fn should_shutdown_idle(&self) -> bool {
        // Check if ANY connection is active
        let conns = self.active_connections.lock().await;

        if !conns.is_empty() {
            // Have active connections - don't shutdown
            return false;
        }

        // No active connections - check last activity
        let activity = self.connection_activity.lock().await;

        if let Some(last) = activity.values().max() {
            last.elapsed() > self.idle_timeout
        } else {
            // No activity recorded - use daemon start time
            self.start_time.elapsed() > self.idle_timeout
        }
    }
}
```

**Alternative: Client Heartbeat**

```rust
// Client sends periodic heartbeat
impl DaemonClient {
    async fn start_heartbeat(&self) {
        let mut interval = tokio::time::interval(Duration::from_secs(60));

        loop {
            interval.tick().await;

            if let Err(e) = self.send_ping().await {
                warn!("Heartbeat failed: {}", e);
                // Reconnect...
                break;
            }
        }
    }
}
```

---

## 8. Cleanup on System Events 🟠 MODERATE

### Problem: Daemons Survive Logout/Shutdown

**Issues:**
1. User logs out → daemons keep running (waste resources)
2. System shutdown → daemons not cleaned up
3. SSH disconnect → daemon orphaned

**Solution: Session Tracking**

```rust
async fn detect_session_end() -> bool {
    // Check if parent session still exists

    // Method 1: Check SSH_CONNECTION
    if let Ok(ssh_conn) = std::env::var("SSH_CONNECTION") {
        // We're in SSH session - check if connection alive
        // (This is tricky - might need to check parent process)
    }

    // Method 2: Check if controlling terminal exists
    use nix::unistd::tcgetpgrp;
    match tcgetpgrp(0) {
        Ok(_) => false,  // Terminal exists
        Err(_) => true,   // Terminal gone - session ended
    }
}

impl DaemonServer {
    async fn run(&mut self) -> Result<()> {
        let mut session_check = tokio::time::interval(Duration::from_secs(60));

        loop {
            tokio::select! {
                // ... handle connections ...

                _ = session_check.tick() => {
                    if detect_session_end().await {
                        info!("Session ended - shutting down daemon");
                        break;
                    }
                }
            }
        }

        self.cleanup().await
    }
}
```

**Better: Use systemd User Session (Linux)**

```rust
// When starting daemon
fn start_daemon_with_systemd(daemon_dir: &Path) -> Result<()> {
    // Use systemd-run to tie daemon to user session
    Command::new("systemd-run")
        .arg("--user")
        .arg("--scope")
        .arg("--")
        .arg(current_exe())
        .arg("--daemon-mode")
        .arg("--daemon-dir").arg(daemon_dir)
        .spawn()?;

    Ok(())
}
```

This ensures daemon stops on logout automatically.

---

## Summary: Priority Matrix

| Issue | Priority | Impact if Not Fixed | Difficulty |
|-------|----------|---------------------|------------|
| Rogue jdtls processes | 🔴 Critical | Users can't work, wasted resources | High |
| PID reuse | 🔴 Critical | Kill wrong process (security issue!) | Medium |
| Multiple daemons race | 🔴 Critical | Corruption, crashes | Medium |
| Stale daemon detection | 🟡 Important | Failed starts, confusion | Low |
| Workspace corruption | 🟡 Important | Lost work, constant reinitialization | Medium |
| Resource limits | 🟡 Important | Memory exhaustion | Low |
| Activity tracking | 🟠 Moderate | Unexpected timeouts | Low |
| Session cleanup | 🟠 Moderate | Wasted resources | Medium |

---

## Implementation Checklist

### Must Have (Before Release)
- [ ] PID verification (start time + cmd hash)
- [ ] Robust process killing (SIGTERM → SIGKILL → detect stuck)
- [ ] File locking for daemon start (prevent race)
- [ ] Comprehensive staleness checks
- [ ] Workspace corruption detection and recovery
- [ ] Global resource limits (max daemons, max memory)

### Should Have (Soon After)
- [ ] Per-connection activity tracking
- [ ] Session end detection
- [ ] Automatic rogue process cleanup
- [ ] Health monitoring and metrics
- [ ] Maximum daemon lifetime (24h)

### Nice to Have (Future)
- [ ] systemd integration
- [ ] Prometheus metrics
- [ ] User notification of evictions
- [ ] Workspace corruption analysis

---

## Testing Strategy

Each edge case needs specific tests:

```bash
tests/daemon/
├── test_rogue_process.sh        # jdtls won't die
├── test_pid_reuse.sh            # PID verification
├── test_concurrent_start.sh     # Race condition
├── test_stale_detection.sh      # Various staleness scenarios
├── test_workspace_corrupt.sh    # Corruption recovery
├── test_resource_limits.sh      # Memory/count limits
├── test_activity_tracking.sh    # Multi-connection timeout
└── test_session_cleanup.sh      # Logout scenarios
```

This edge case analysis should be incorporated into the implementation plan to ensure robustness from day one.
