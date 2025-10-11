# LSP Daemon Mode - Implementation Checklist

## Pre-Implementation Review Complete ✅

After thorough edge case analysis, here's the complete implementation checklist incorporating all critical safety measures.

---

## Phase 1: Safe Foundation (Week 1)

### 1.1 PID Management (🔴 CRITICAL - Security Issue)

- [ ] Create `DaemonPidInfo` struct with:
  - `pid: i32`
  - `start_time: SystemTime`
  - `cmd_hash: u64`

- [ ] Implement `verify_daemon_pid()`:
  - Check process exists
  - Verify start time matches (prevent PID reuse attack)
  - Verify command line hash matches
  - Return `Ok(Some(pid))` only if all checks pass

- [ ] Add platform-specific `get_process_start_time()`:
  - Linux: Read `/proc/{pid}/stat`
  - macOS: Use `proc_pidinfo()`
  - Windows: Use `GetProcessTimes()`

**Test:**
```bash
./tests/daemon/test_pid_verification.sh
```

### 1.2 Process Killing (🔴 CRITICAL - Reliability)

- [ ] Implement `kill_process_forcefully()`:
  ```rust
  enum ProcessKillStatus {
      Terminated,    // SIGTERM worked
      Killed,        // SIGKILL worked
      Zombie,        // Process is zombie (already dead)
      Stuck,         // Process in state D (uninterruptible sleep)
  }
  ```

- [ ] Kill escalation strategy:
  1. Send SIGTERM, wait 5 seconds
  2. If still alive, send SIGKILL, wait 2 seconds
  3. If still alive, check process state
  4. If state D, mark as rogue and work around
  5. If zombie, ignore (it's dead)

- [ ] Implement rogue process tracking:
  - Log rogue processes for monitoring
  - Use alternative workspace if stuck
  - Alert user to check system

**Test:**
```bash
./tests/daemon/test_rogue_process.sh
```

### 1.3 Race Condition Prevention (🔴 CRITICAL - Data Integrity)

- [ ] Implement file locking for daemon start:
  ```rust
  fn acquire_daemon_lock(daemon_dir: &Path) -> Result<FileLock>
  ```

- [ ] Lock timeout handling (stale lock cleanup):
  - Wait up to 30 seconds for lock
  - If timeout, assume stale and force break
  - Log warning about force break

- [ ] Atomic socket creation:
  - Use `bind()` on socket (fails if exists)
  - Double-check pattern after acquiring lock

- [ ] Add `ensure_single_daemon()` that:
  1. Acquires lock
  2. Double-checks daemon doesn't exist
  3. Starts daemon if needed
  4. Releases lock

**Test:**
```bash
./tests/daemon/test_concurrent_start.sh
```

---

## Phase 2: Robustness (Week 2)

### 2.1 Stale Daemon Detection (🟡 IMPORTANT)

- [ ] Implement comprehensive `check_daemon_staleness()`:
  - Check socket exists
  - Check PID file exists
  - Verify PID (with start time check)
  - Verify process is daemon (check cmdline)
  - Try to connect to socket (5 second timeout)
  - Send ping, expect pong (5 second timeout)

- [ ] Return `DaemonStatus` enum:
  - `NotExists` - No daemon
  - `Alive { pid }` - Healthy
  - `Stale { reason }` - Dead/corrupted
  - `Unresponsive { pid, reason }` - Alive but not responding

- [ ] Implement cleanup for each status:
  - `Stale`: Remove socket, PID file, start fresh
  - `Unresponsive`: Kill process, cleanup, start fresh

**Test:**
```bash
./tests/daemon/test_stale_detection.sh
```

### 2.2 Workspace Corruption (🟡 IMPORTANT)

- [ ] Implement `check_workspace_health()`:
  - Verify `.metadata/` exists
  - Check for corruption markers
  - Validate workspace version file

- [ ] Corruption detection during runtime:
  - Track jdtls restart count (>3 = corruption?)
  - Monitor timeout rate (>50% = corruption?)
  - Parse jdtls logs for corruption errors

- [ ] Recovery procedure:
  1. Backup corrupted workspace to `.corrupted.{timestamp}`
  2. Delete workspace directory
  3. Restart jdtls with fresh workspace
  4. Reset restart counter

- [ ] Notify user about workspace reset

**Test:**
```bash
./tests/daemon/test_workspace_corruption.sh
```

### 2.3 Socket Management (🟡 IMPORTANT)

- [ ] Handle socket creation failures:
  - Path too long → use hash instead
  - Permission denied → try alternative location
  - NFS mount → warn and use TCP socket

- [ ] Socket cleanup:
  - Always remove socket on daemon shutdown
  - Remove stale sockets on startup
  - Set correct permissions (600 - owner only)

- [ ] Connection timeout:
  - 5 second timeout on connect
  - Clear error message on failure

**Test:**
```bash
./tests/daemon/test_socket_edge_cases.sh
```

---

## Phase 3: Resource Management (Week 2)

### 3.1 Memory Limits (🟡 IMPORTANT)

- [ ] Implement `GlobalDaemonManager`:
  - Track all active daemons
  - Monitor total memory usage
  - Enforce limits

- [ ] Configuration:
  ```rust
  max_daemons: 5,
  max_total_memory_mb: 2048,
  ```

- [ ] LRU eviction:
  - Sort daemons by last activity
  - Kill least recently used when over limit
  - Warn user about eviction

- [ ] Per-daemon memory monitoring:
  - Read from `/proc/{pid}/status` (Linux)
  - Check every minute
  - Kill daemon if >1GB individually

**Test:**
```bash
./tests/daemon/test_memory_limits.sh
```

### 3.2 Activity Tracking (🟠 MODERATE)

- [ ] Per-connection tracking:
  ```rust
  active_connections: HashSet<ConnectionId>,
  connection_activity: HashMap<ConnectionId, Instant>,
  ```

- [ ] Update activity on every request

- [ ] Idle timeout calculation:
  - If any connection active → don't timeout
  - If all connections closed → check last activity
  - Timeout only if ALL connections idle for 30 min

- [ ] Alternative: Client heartbeat
  - Send ping every 60 seconds
  - Update activity on heartbeat

**Test:**
```bash
./tests/daemon/test_activity_tracking.sh
```

---

## Phase 4: Platform Support (Week 3)

### 4.1 Linux-Specific

- [ ] Use `/proc` for process info
- [ ] File locking with `flock()`
- [ ] systemd user session integration (optional)
- [ ] Handle different filesystems (ext4, btrfs, NFS)

### 4.2 macOS-Specific

- [ ] Use `libproc` for process info
- [ ] File locking compatibility
- [ ] Handle APFS filesystem
- [ ] Launchd integration (optional)

### 4.3 Cross-Platform

- [ ] Abstract process management:
  ```rust
  trait ProcessManager {
      fn get_start_time(pid: i32) -> Result<SystemTime>;
      fn kill_process(pid: i32, signal: Signal) -> Result<()>;
      fn process_exists(pid: i32) -> bool;
  }
  ```

- [ ] Platform detection and feature flags

**Test:**
```bash
# Run on both platforms
./tests/daemon/test_platform_linux.sh
./tests/daemon/test_platform_macos.sh
```

---

## Phase 5: Error Handling (Week 3)

### 5.1 Graceful Degradation

- [ ] Daemon mode fails → fall back to direct mode
- [ ] Clear error messages for each failure
- [ ] User can override with `--no-daemon` flag

### 5.2 Error Messages

- [ ] "Daemon failed to start: {reason}"
- [ ] "Falling back to direct mode (slower)"
- [ ] "jdtls workspace corrupted, rebuilding..."
- [ ] "Daemon evicted (memory limit), restart will create new one"

### 5.3 Logging

- [ ] Daemon logs to `~/.cache/ovim/daemons/{hash}/daemon.log`
- [ ] Structured logging with levels (info, warn, error)
- [ ] Log rotation (keep last 5 files, max 10MB each)
- [ ] User can check logs: `ovim --daemon logs`

---

## Phase 6: Testing (Week 4)

### 6.1 Unit Tests

- [ ] PID verification tests
- [ ] Process killing tests
- [ ] Staleness detection tests
- [ ] Workspace corruption tests
- [ ] Protocol serialization tests

**Coverage target: >80%**

### 6.2 Integration Tests

- [ ] Basic lifecycle test
- [ ] Concurrent start test
- [ ] Crash recovery test
- [ ] Workspace corruption recovery
- [ ] Memory limit enforcement
- [ ] Activity tracking test

### 6.3 Performance Tests

- [ ] Cold start benchmark (<90s)
- [ ] Warm start benchmark (<5s)
- [ ] Request latency (p95 <100ms)
- [ ] Memory usage (<1GB per daemon)
- [ ] Load test (50 concurrent clients)

### 6.4 Manual Testing

- [ ] Real Spring Boot project
- [ ] Open 10 files rapidly
- [ ] Simulate crash (kill -9)
- [ ] Corrupt workspace manually
- [ ] Test on slow disk/NFS
- [ ] Test with limited memory

---

## Pre-Release Checklist

### Security

- [ ] PID reuse attack prevented
- [ ] Socket permissions (owner-only)
- [ ] No arbitrary code execution
- [ ] Input sanitization (URIs, paths)
- [ ] No credential leaks in logs

### Reliability

- [ ] Rogue process handling
- [ ] Workspace corruption recovery
- [ ] Race condition prevention
- [ ] Graceful degradation
- [ ] Clean shutdown

### Performance

- [ ] <5s warm start (90th percentile)
- [ ] <100ms request latency (p95)
- [ ] <2GB total memory (5 daemons)
- [ ] No memory leaks
- [ ] No process leaks

### User Experience

- [ ] Clear error messages
- [ ] Progress indicators
- [ ] Help documentation
- [ ] FAQ for common issues
- [ ] `ovim --daemon status` command

---

## Documentation Checklist

### User Docs

- [ ] How daemon mode works
- [ ] Configuration options
- [ ] Troubleshooting guide
- [ ] FAQ (common issues)
- [ ] Performance expectations

### Developer Docs

- [ ] Architecture overview
- [ ] Protocol specification
- [ ] Edge case handling
- [ ] Test suite guide
- [ ] Contributing guide

---

## Risk Assessment

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| PID reuse kills wrong process | Low | Critical | Verify start time + cmdline |
| Rogue process can't be killed | Low | High | Work around with alt workspace |
| Race creates multiple daemons | Medium | High | File locking + double-check |
| Workspace corruption | Medium | Medium | Detection + auto-rebuild |
| Memory exhaustion | Medium | Medium | LRU eviction + limits |
| Daemon crashes frequently | Low | Medium | Auto-restart + corruption fix |

---

## Success Criteria

### Must Work
- ✅ No security issues (PID reuse, etc.)
- ✅ No data corruption (race conditions)
- ✅ Warm start <5s (90th percentile)
- ✅ All critical edge case tests pass
- ✅ Works on Linux and macOS

### Should Work
- 🎯 Memory usage <1GB per daemon
- 🎯 Handles 5+ concurrent projects
- 🎯 Auto-recovers from crashes
- 🎯 Clear error messages
- 🎯 Performance benchmarks meet targets

### Nice to Have
- 💡 systemd integration
- 💡 Prometheus metrics
- 💡 Hot reload
- 💡 Advanced monitoring

---

## Implementation Order (Recommended)

**Week 1: Safety First**
1. PID verification (prevent security issues)
2. Process killing (handle rogue processes)
3. File locking (prevent races)

**Week 2: Robustness**
4. Stale detection
5. Workspace corruption
6. Resource limits

**Week 3: Polish**
7. Error handling
8. Logging
9. Documentation

**Week 4: Testing**
10. All tests passing
11. Performance benchmarks
12. Manual testing on real projects

---

## Ready to Implement?

Before starting implementation, ensure:

- [ ] This checklist reviewed and understood
- [ ] Edge case document reviewed (`DAEMON_EDGE_CASES_DEEP_DIVE.md`)
- [ ] Implementation plan reviewed (`DAEMON_IMPLEMENTATION_PLAN.md`)
- [ ] Test suite structure understood
- [ ] Resources allocated (4-6 weeks)

**If all checked, ready to proceed with Phase 1!**
