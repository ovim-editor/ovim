# LSP Daemon Edge Cases - Comprehensive Test Suite

## Test Categories

1. **Lifecycle Tests** - Daemon start/stop/restart
2. **Concurrency Tests** - Multiple clients, race conditions
3. **Failure Tests** - Crashes, corruption, resource exhaustion
4. **Project Tests** - Multi-project, nested projects, no project
5. **Security Tests** - Permissions, ownership, injection
6. **Performance Tests** - Stress, memory leaks, latency
7. **Platform Tests** - Linux, Mac, different file systems

---

## 1. Lifecycle Tests

### Test 1.1: Normal Start/Stop

```bash
#!/bin/bash
# Test: Daemon starts on first open, stops after idle timeout

cd /tmp/daemon-test-1
mkdir -p src

cat > src/Test.java << 'EOF'
public class Test { }
EOF

# First open - daemon should start
ovim src/Test.java < <(sleep 2 && echo ":q")

# Check daemon is running
DAEMON_PID=$(cat ~/.cache/ovim/daemons/*/daemon.pid)
ps -p $DAEMON_PID || exit 1  # Should exist

# Second open - should reuse daemon
ovim src/Test.java < <(sleep 2 && echo ":q")

# Check same daemon
DAEMON_PID2=$(cat ~/.cache/ovim/daemons/*/daemon.pid)
[ "$DAEMON_PID" == "$DAEMON_PID2" ] || exit 1  # Should be same

# Wait for idle timeout (simulate with --idle-timeout=5 for testing)
sleep 10

# Check daemon stopped
ps -p $DAEMON_PID && exit 1  # Should NOT exist
```

**Expected:** ✅ Daemon starts, reused, then auto-stops

### Test 1.2: Manual Shutdown

```bash
# Start daemon
ovim src/Test.java < <(sleep 2 && echo ":q")

# Manual shutdown
ovim --daemon stop

# Check daemon stopped
ps aux | grep "ovim.*daemon-mode" | grep -v grep && exit 1

# Socket and PID files removed
[ -f ~/.cache/ovim/daemons/*/daemon.sock ] && exit 1
[ -f ~/.cache/ovim/daemons/*/daemon.pid ] && exit 1
```

**Expected:** ✅ Daemon stops cleanly

### Test 1.3: Daemon Restart

```bash
# Start daemon
ovim src/Test.java < <(sleep 2 && echo ":q")

DAEMON_PID=$(cat ~/.cache/ovim/daemons/*/daemon.pid)

# Restart daemon
ovim --daemon restart

# Check new PID
DAEMON_PID2=$(cat ~/.cache/ovim/daemons/*/daemon.pid)
[ "$DAEMON_PID" != "$DAEMON_PID2" ] || exit 1  # Should be different

# Old daemon stopped
ps -p $DAEMON_PID && exit 1  # Should NOT exist

# New daemon running
ps -p $DAEMON_PID2 || exit 1  # Should exist
```

**Expected:** ✅ Old daemon stops, new daemon starts

### Test 1.4: Activity Prevents Timeout

```bash
# Start daemon with 10s idle timeout
ovim --idle-timeout=10 src/Test.java < <(sleep 2 && echo ":q")

# Wait 6 seconds
sleep 6

# Open again (resets timeout)
ovim src/Test.java < <(sleep 2 && echo ":q")

# Wait another 6 seconds (total 12s since first, but 6s since last)
sleep 6

# Daemon should still be alive (timeout resets on activity)
DAEMON_PID=$(cat ~/.cache/ovim/daemons/*/daemon.pid)
ps -p $DAEMON_PID || exit 1  # Should still exist

# Wait full 10s with no activity
sleep 11

# Now daemon should be stopped
ps -p $DAEMON_PID && exit 1  # Should NOT exist
```

**Expected:** ✅ Activity resets idle timeout

---

## 2. Concurrency Tests

### Test 2.1: Concurrent Client Connections

```bash
# Start 5 ovim instances simultaneously
for i in {1..5}; do
    ovim src/Test.java < <(sleep 5 && echo ":q") &
done

wait

# All should use same daemon
DAEMON_COUNT=$(ps aux | grep "ovim.*daemon-mode" | grep -v grep | wc -l)
[ "$DAEMON_COUNT" -eq 1 ] || exit 1  # Should be exactly 1 daemon
```

**Expected:** ✅ All clients connect to same daemon

### Test 2.2: Race Condition on Daemon Start

```bash
# Start 10 ovim instances at exact same time (all try to start daemon)
for i in {1..10}; do
    (ovim src/Test.java < <(sleep 2 && echo ":q")) &
done

wait

# Should only have 1 daemon (not 10)
DAEMON_COUNT=$(ps aux | grep "ovim.*daemon-mode" | grep -v grep | wc -l)
[ "$DAEMON_COUNT" -eq 1 ] || exit 1

# All PID files should have same PID
PID1=$(cat ~/.cache/ovim/daemons/*/daemon.pid)
# Verify no duplicate daemon directories
DAEMON_DIRS=$(find ~/.cache/ovim/daemons -name "daemon.pid" | wc -l)
[ "$DAEMON_DIRS" -eq 1 ] || exit 1
```

**Expected:** ✅ Only one daemon starts despite race

**Implementation hint:** Use file locking or atomic socket creation

### Test 2.3: Interleaved Requests

```rust
// Rust test: Send multiple requests concurrently from same client

#[tokio::test]
async fn test_interleaved_requests() {
    let mut client = DaemonClient::connect_or_start(&project_root).await.unwrap();

    // Send 100 hover requests concurrently
    let mut tasks = vec![];
    for i in 0..100 {
        let mut c = client.clone();
        tasks.push(tokio::spawn(async move {
            c.hover("file:///test.java", i % 50, 0).await
        }));
    }

    // All should complete without errors
    for task in tasks {
        assert!(task.await.is_ok());
    }
}
```

**Expected:** ✅ All requests complete correctly

---

## 3. Failure Tests

### Test 3.1: Daemon Crash

```bash
# Start daemon
ovim src/Test.java < <(sleep 2 && echo ":q")

DAEMON_PID=$(cat ~/.cache/ovim/daemons/*/daemon.pid)

# Kill daemon unexpectedly
kill -9 $DAEMON_PID

# Try to use ovim again - should detect crash and restart daemon
ovim src/Test.java < <(sleep 2 && echo ":q")

# Should have new daemon
DAEMON_PID2=$(cat ~/.cache/ovim/daemons/*/daemon.pid)
[ "$DAEMON_PID" != "$DAEMON_PID2" ] || exit 1  # Different PID
ps -p $DAEMON_PID2 || exit 1  # New daemon running
```

**Expected:** ✅ Detects crash, auto-restarts daemon

### Test 3.2: jdtls Crash Inside Daemon

```bash
# Start daemon
ovim src/Test.java < <(sleep 2 && echo ":q")

# Find jdtls PID
JDTLS_PID=$(cat ~/.cache/ovim/daemons/*/jdtls.pid)

# Kill jdtls
kill -9 $JDTLS_PID

# Try to use LSP features - should detect and restart jdtls
ovim src/Test.java
# Press K for hover - should work after restart

# New jdtls should be running
JDTLS_PID2=$(pgrep -f "jdt.ls")
[ -n "$JDTLS_PID2" ] || exit 1
```

**Expected:** ✅ Detects jdtls crash, restarts it

### Test 3.3: Stale Socket File

```bash
# Create fake socket file
SOCKET_PATH=~/.cache/ovim/daemons/abc123/daemon.sock
mkdir -p $(dirname $SOCKET_PATH)
touch $SOCKET_PATH

# Create fake PID file with dead PID
echo "99999" > $(dirname $SOCKET_PATH)/daemon.pid

# Try to open file - should detect stale socket and clean up
ovim src/Test.java < <(sleep 2 && echo ":q")

# Should have started new daemon successfully
ps aux | grep "ovim.*daemon-mode" | grep -v grep || exit 1
```

**Expected:** ✅ Cleans up stale socket, starts fresh

### Test 3.4: Corrupted Workspace

```bash
# Start daemon
ovim src/Test.java < <(sleep 60 && echo ":q") &
sleep 30  # Wait for initialization

# Corrupt workspace
WORKSPACE=$(find ~/.cache/ovim/daemons/*/workspace -type d | head -1)
rm -rf $WORKSPACE/.metadata

# Next request should detect corruption and restart
ovim src/Test.java < <(sleep 2 && echo ":q")
```

**Expected:** ✅ Detects corruption, recreates workspace

### Test 3.5: Out of Disk Space

```bash
# Fill up disk (use tmpfs for testing)
mount -t tmpfs -o size=100M tmpfs /tmp/small-disk
cd /tmp/small-disk

# Create large files to fill disk
dd if=/dev/zero of=filler bs=1M count=95

# Try to start daemon (socket creation should fail)
ovim src/Test.java 2>&1 | tee output.log

# Should show clear error message
grep -i "disk full\|no space" output.log || exit 1

# Should gracefully fall back to non-daemon mode
# OR show helpful error message
```

**Expected:** ✅ Graceful error, doesn't crash

### Test 3.6: OOM (Out of Memory)

```rust
// Rust test: Simulate memory pressure

#[tokio::test]
async fn test_memory_limit() {
    // Set RLIMIT_AS to 512MB
    use libc::{setrlimit, rlimit, RLIMIT_AS};

    let limit = rlimit {
        rlim_cur: 512 * 1024 * 1024,
        rlim_max: 512 * 1024 * 1024,
    };

    unsafe {
        setrlimit(RLIMIT_AS, &limit);
    }

    // Start daemon - should handle OOM gracefully
    let result = DaemonServer::start(project_root).await;

    // Should either:
    // A) Start successfully within memory limit
    // B) Fail gracefully with clear error
    assert!(result.is_ok() || result.err().unwrap().to_string().contains("memory"));
}
```

**Expected:** ✅ Graceful handling, clear error

---

## 4. Project Tests

### Test 4.1: Multiple Separate Projects

```bash
# Create 2 projects
mkdir -p /tmp/project-a/src
mkdir -p /tmp/project-b/src

cd /tmp/project-a
cat > build.gradle << 'EOF'
apply plugin: 'java'
EOF

cd /tmp/project-b
cat > pom.xml << 'EOF'
<project></project>
EOF

# Open file in project A
cd /tmp/project-a
ovim src/A.java < <(sleep 2 && echo ":q")

# Open file in project B
cd /tmp/project-b
ovim src/B.java < <(sleep 2 && echo ":q")

# Should have 2 separate daemons
DAEMON_COUNT=$(ps aux | grep "ovim.*daemon-mode" | grep -v grep | wc -l)
[ "$DAEMON_COUNT" -eq 2 ] || exit 1  # Should be 2 daemons
```

**Expected:** ✅ Separate daemons for separate projects

### Test 4.2: Nested Projects

```bash
# Create nested structure
mkdir -p /tmp/parent/child

cd /tmp/parent
cat > build.gradle << 'EOF'
apply plugin: 'java'
EOF

cd /tmp/parent/child
cat > build.gradle << 'EOF'
apply plugin: 'java'
EOF

# Open file in child - should use child's project
cd /tmp/parent/child
ovim src/Child.java < <(sleep 2 && echo ":q")

# Check which project daemon is for
DAEMON_DIR=$(find ~/.cache/ovim/daemons -type d -name "*" | head -1)
# Daemon should be for /tmp/parent/child, not /tmp/parent
```

**Expected:** ✅ Uses innermost project (child)

### Test 4.3: No Project (Standalone File)

```bash
# Create standalone file outside any project
cd /tmp
cat > Standalone.java << 'EOF'
public class Standalone { }
EOF

# Open standalone file
ovim Standalone.java < <(sleep 2 && echo ":q")

# Should either:
# A) Create temp project daemon
# B) Use global daemon
# C) Run without daemon (acceptable for single files)

# Verify it works (can edit, syntax highlighting)
```

**Expected:** ✅ Works reasonably, even without project

### Test 4.4: Moving Between Projects

```bash
# Open file in project A
cd /tmp/project-a
ovim src/A.java < <(sleep 2 && echo ":q")

# Open file in project B
cd /tmp/project-b
ovim src/B.java < <(sleep 2 && echo ":q")

# Return to project A
cd /tmp/project-a
ovim src/A.java < <(sleep 2 && echo ":q")

# Should reuse project A's daemon (still alive)
# Not create 3rd daemon
DAEMON_COUNT=$(ps aux | grep "ovim.*daemon-mode" | grep -v grep | wc -l)
[ "$DAEMON_COUNT" -eq 2 ] || exit 1  # Still just 2 daemons
```

**Expected:** ✅ Reuses correct daemon per project

### Test 4.5: Symlink Project Root

```bash
mkdir -p /tmp/real-project/src
cat > /tmp/real-project/build.gradle << 'EOF'
apply plugin: 'java'
EOF

# Create symlink
ln -s /tmp/real-project /tmp/linked-project

# Open via real path
cd /tmp/real-project
ovim src/A.java < <(sleep 2 && echo ":q")

DAEMON_PID1=$(cat ~/.cache/ovim/daemons/*/daemon.pid)

# Open via symlink
cd /tmp/linked-project
ovim src/A.java < <(sleep 2 && echo ":q")

# Should use SAME daemon (canonical path matching)
DAEMON_PID2=$(cat ~/.cache/ovim/daemons/*/daemon.pid)
[ "$DAEMON_PID1" == "$DAEMON_PID2" ] || exit 1
```

**Expected:** ✅ Canonical path matching, same daemon

---

## 5. Security Tests

### Test 5.1: Socket Permissions

```bash
# Start daemon
ovim src/Test.java < <(sleep 2 && echo ":q")

# Check socket permissions (should be user-only)
SOCKET=$(find ~/.cache/ovim/daemons -name "daemon.sock" | head -1)
PERMS=$(stat -c %a $SOCKET)

# Should be 600 or 700 (user only)
[[ "$PERMS" =~ ^[67]00$ ]] || exit 1
```

**Expected:** ✅ Socket only accessible by owner

### Test 5.2: PID File Ownership

```bash
# Start daemon
ovim src/Test.java < <(sleep 2 && echo ":q")

# Check PID file ownership
PID_FILE=$(find ~/.cache/ovim/daemons -name "daemon.pid" | head -1)
OWNER=$(stat -c %U $PID_FILE)
CURRENT_USER=$(whoami)

[ "$OWNER" == "$CURRENT_USER" ] || exit 1
```

**Expected:** ✅ PID file owned by current user

### Test 5.3: Request Injection

```rust
// Rust test: Try to inject malicious requests

#[tokio::test]
async fn test_request_injection() {
    let mut client = DaemonClient::connect_or_start(&project_root).await.unwrap();

    // Try to inject shell commands in URI
    let result = client.hover(
        "file:///../../../etc/passwd; rm -rf /",
        0,
        0
    ).await;

    // Should sanitize or reject
    assert!(result.is_err() || result.unwrap().is_none());

    // Verify no shell execution occurred
    assert!(std::path::Path::new("/etc/passwd").exists());
}
```

**Expected:** ✅ Input sanitization, no command injection

### Test 5.4: Path Traversal

```bash
# Try to access files outside project
ovim src/Test.java

# Inside editor, try to request hover for file outside project
# (e.g., ../../../../etc/passwd)

# Daemon should reject or sandbox the request
```

**Expected:** ✅ Path traversal prevented

---

## 6. Performance Tests

### Test 6.1: Rapid Open/Close (Stress Test)

```bash
#!/bin/bash
# Open and close 100 times rapidly

for i in {1..100}; do
    echo "Iteration $i"
    ovim src/Test.java < <(sleep 0.1 && echo ":q")
done

# Check for resource leaks
DAEMON_COUNT=$(ps aux | grep "ovim.*daemon-mode" | grep -v grep | wc -l)
[ "$DAEMON_COUNT" -eq 1 ] || exit 1  # Should still be 1 daemon

# Check socket count
SOCKET_COUNT=$(find ~/.cache/ovim/daemons -name "daemon.sock" | wc -l)
[ "$SOCKET_COUNT" -eq 1 ] || exit 1  # Should be 1 socket

# Check for zombie processes
ZOMBIE_COUNT=$(ps aux | grep ovim | grep "<defunct>" | wc -l)
[ "$ZOMBIE_COUNT" -eq 0 ] || exit 1  # No zombies
```

**Expected:** ✅ No leaks, no zombies, still responsive

### Test 6.2: Memory Leak Detection

```bash
#!/bin/bash
# Monitor memory usage over time

ovim src/Test.java < <(sleep 2 && echo ":q")
DAEMON_PID=$(cat ~/.cache/ovim/daemons/*/daemon.pid)

# Record initial memory
MEM1=$(ps -p $DAEMON_PID -o rss= | awk '{print $1}')

# Perform 1000 operations
for i in {1..1000}; do
    ovim src/Test.java < <(echo "K" && sleep 0.1 && echo ":q")
done

# Record final memory
MEM2=$(ps -p $DAEMON_PID -o rss= | awk '{print $1}')

# Memory should not grow by more than 50MB
MEM_GROWTH=$((MEM2 - MEM1))
[ "$MEM_GROWTH" -lt 51200 ] || exit 1  # Less than 50MB growth
```

**Expected:** ✅ Bounded memory growth

### Test 6.3: Latency Test

```rust
// Rust test: Measure request latency

#[tokio::test]
async fn test_latency() {
    let mut client = DaemonClient::connect_or_start(&project_root).await.unwrap();

    // Measure 100 hover requests
    let mut latencies = vec![];

    for _ in 0..100 {
        let start = Instant::now();
        client.hover("file:///test.java", 0, 0).await.unwrap();
        let elapsed = start.elapsed();
        latencies.push(elapsed.as_millis());
    }

    // Calculate p50, p95, p99
    latencies.sort();
    let p50 = latencies[50];
    let p95 = latencies[95];
    let p99 = latencies[99];

    // Assertions
    assert!(p50 < 50, "p50 latency should be < 50ms");
    assert!(p95 < 200, "p95 latency should be < 200ms");
    assert!(p99 < 500, "p99 latency should be < 500ms");
}
```

**Expected:** ✅ Low latency, consistent

### Test 6.4: Concurrent Clients (Load Test)

```bash
#!/bin/bash
# 50 concurrent clients

for i in {1..50}; do
    (
        for j in {1..10}; do
            ovim src/Test.java < <(echo "K" && sleep 0.1 && echo ":q")
        done
    ) &
done

wait

# Daemon should handle all requests without errors
# Check logs for errors
grep -i error ~/.cache/ovim/daemons/*/daemon.log && exit 1
```

**Expected:** ✅ Handles load, no errors

---

## 7. Platform Tests

### Test 7.1: Linux

```bash
# Test on Ubuntu, Fedora, Arch
uname -a

ovim src/Test.java < <(sleep 2 && echo ":q")

# Check daemon running
ps aux | grep "ovim.*daemon-mode" | grep -v grep || exit 1
```

**Expected:** ✅ Works on major Linux distros

### Test 7.2: macOS

```bash
# Test on macOS (Darwin)
uname -a

ovim src/Test.java < <(sleep 2 && echo ":q")

# Check daemon running
ps aux | grep "ovim.*daemon-mode" | grep -v grep || exit 1
```

**Expected:** ✅ Works on macOS

### Test 7.3: Different Filesystems

```bash
# Test on ext4
mount | grep ext4 && ovim src/Test.java

# Test on btrfs
mount | grep btrfs && ovim src/Test.java

# Test on NFS
mount | grep nfs && ovim src/Test.java

# Test on tmpfs
mount | grep tmpfs && ovim src/Test.java
```

**Expected:** ✅ Works on various filesystems

---

## Test Automation

### Run All Tests

```bash
#!/bin/bash
# run_all_daemon_tests.sh

set -e

echo "=== LSP Daemon Test Suite ==="

# Lifecycle
echo "Running lifecycle tests..."
./tests/daemon/test_lifecycle.sh

# Concurrency
echo "Running concurrency tests..."
./tests/daemon/test_concurrency.sh

# Failures
echo "Running failure tests..."
./tests/daemon/test_failures.sh

# Projects
echo "Running project tests..."
./tests/daemon/test_projects.sh

# Security
echo "Running security tests..."
./tests/daemon/test_security.sh

# Performance
echo "Running performance tests..."
./tests/daemon/test_performance.sh

# Platform
echo "Running platform tests..."
./tests/daemon/test_platform.sh

echo "=== All Tests Passed! ==="
```

### Continuous Integration

```yaml
# .github/workflows/daemon-tests.yml

name: Daemon Tests

on: [push, pull_request]

jobs:
  test-daemon:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]

    steps:
    - uses: actions/checkout@v2

    - name: Install Rust
      uses: actions-rs/toolchain@v1

    - name: Install Java
      uses: actions/setup-java@v2
      with:
        java-version: '17'

    - name: Run daemon tests
      run: ./run_all_daemon_tests.sh
```

## Success Criteria

All tests must pass:
- ✅ All lifecycle tests
- ✅ All concurrency tests
- ✅ All failure recovery tests
- ✅ All project isolation tests
- ✅ All security tests
- ✅ All performance benchmarks
- ✅ All platform tests

**Zero tolerance for:**
- Process leaks
- Memory leaks
- Security vulnerabilities
- Data corruption
- Race conditions

## Test Coverage Target

- **Line coverage:** >80%
- **Branch coverage:** >75%
- **Edge case coverage:** 100% of identified edge cases
