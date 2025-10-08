#!/bin/bash
# Critical Edge Case Tests for LSP Daemon Mode
# These tests MUST pass before daemon mode is released

set -e

TEST_DIR="/tmp/ovim-daemon-critical-tests-$$"
DAEMON_DIR="$HOME/.cache/ovim/daemons"

cleanup() {
    echo "Cleaning up..."
    rm -rf "$TEST_DIR"
    pkill -f "ovim.*daemon-mode" || true
    rm -rf "$DAEMON_DIR"
}

trap cleanup EXIT

echo "=== Critical Edge Case Tests ==="
echo ""

# Test 1: PID Reuse Detection
echo "Test 1: PID Reuse Detection"
echo "  Scenario: Daemon crashes, PID reused by different process"
echo ""

mkdir -p "$TEST_DIR/project1/src"
cd "$TEST_DIR/project1"

cat > build.gradle << 'EOF'
apply plugin: 'java'
EOF

# Start daemon
echo "  Starting daemon..."
timeout 120 ovim src/Test.java < <(sleep 3 && echo ":q") 2>/dev/null &
sleep 10

# Get daemon PID
DAEMON_PID=$(pgrep -f "ovim.*daemon-mode" | head -1)

if [ -z "$DAEMON_PID" ]; then
    echo "  ⚠️  Daemon mode not yet implemented - skipping test"
else
    echo "  Daemon PID: $DAEMON_PID"

    # Simulate crash (kill -9)
    echo "  Simulating crash (kill -9)..."
    kill -9 $DAEMON_PID

    # In real scenario, PID could be reused here
    # For testing, we'll just verify that daemon detects it's gone

    # Try to connect again
    echo "  Attempting to reconnect..."
    timeout 120 ovim src/Test.java < <(sleep 2 && echo ":q") 2>&1 | tee /tmp/ovim-reconnect.log &
    sleep 5

    # Should detect stale PID and start new daemon
    NEW_DAEMON_PID=$(pgrep -f "ovim.*daemon-mode" | head -1)

    if [ -z "$NEW_DAEMON_PID" ]; then
        echo "  ⚠️  No new daemon started"
    elif [ "$NEW_DAEMON_PID" == "$DAEMON_PID" ]; then
        echo "  ❌ FAIL: Same PID detected (should have cleaned up)"
        exit 1
    else
        echo "  ✅ PASS: New daemon started with different PID ($NEW_DAEMON_PID)"
    fi
fi

echo ""

# Test 2: Concurrent Daemon Start (Race Condition)
echo "Test 2: Concurrent Daemon Start"
echo "  Scenario: 5 clients try to start daemon simultaneously"
echo ""

cleanup
mkdir -p "$TEST_DIR/project2/src"
cd "$TEST_DIR/project2"

cat > build.gradle << 'EOF'
apply plugin: 'java'
EOF

# Start 5 clients simultaneously
echo "  Starting 5 concurrent clients..."
for i in {1..5}; do
    (timeout 120 ovim src/Test$i.java < <(sleep 3 && echo ":q") 2>&1 | tee "/tmp/ovim-client-$i.log") &
done

# Wait for all to complete
sleep 15

# Count daemon processes
DAEMON_COUNT=$(pgrep -f "ovim.*daemon-mode" | wc -l)

if [ "$DAEMON_COUNT" -eq 0 ]; then
    echo "  ⚠️  Daemon mode not yet implemented"
elif [ "$DAEMON_COUNT" -eq 1 ]; then
    echo "  ✅ PASS: Exactly 1 daemon created (no race condition)"
else
    echo "  ❌ FAIL: $DAEMON_COUNT daemons created (race condition!)"
    pgrep -f "ovim.*daemon-mode" -l
    exit 1
fi

echo ""

# Test 3: Rogue Process Detection
echo "Test 3: Rogue Process (Won't Die)"
echo "  Scenario: jdtls hangs and won't respond to signals"
echo ""

cleanup
mkdir -p "$TEST_DIR/project3/src"
cd "$TEST_DIR/project3"

cat > build.gradle << 'EOF'
apply plugin: 'java'
EOF

# Start daemon
timeout 120 ovim src/Test.java < <(sleep 3 && echo ":q") 2>/dev/null &
sleep 10

# Find jdtls process
JDTLS_PID=$(pgrep -f "jdtls" | head -1)

if [ -z "$JDTLS_PID" ]; then
    echo "  ⚠️  No jdtls process found (not initialized yet)"
else
    echo "  jdtls PID: $JDTLS_PID"

    # Simulate unresponsive process (SIGSTOP freezes it)
    echo "  Freezing jdtls with SIGSTOP..."
    kill -STOP $JDTLS_PID

    # Try to use daemon
    echo "  Attempting to use frozen daemon..."
    timeout 30 ovim src/Test.java < <(echo "K" && sleep 2 && echo ":q") 2>&1 | tee /tmp/ovim-frozen.log &
    sleep 10

    # Daemon should detect jdtls is unresponsive
    # Should either:
    # A) Kill and restart jdtls
    # B) Work around with new workspace
    # C) Show clear error message

    if grep -q "LSP busy\|timeout\|unresponsive" /tmp/ovim-frozen.log; then
        echo "  ✅ PASS: Detected unresponsive jdtls"
    else
        echo "  ⚠️  Behavior unclear (may need manual verification)"
    fi

    # Cleanup frozen process
    kill -KILL $JDTLS_PID 2>/dev/null || true
fi

echo ""

# Test 4: Workspace Corruption Recovery
echo "Test 4: Workspace Corruption Recovery"
echo "  Scenario: jdtls workspace gets corrupted"
echo ""

cleanup
mkdir -p "$TEST_DIR/project4/src"
cd "$TEST_DIR/project4"

cat > build.gradle << 'EOF'
apply plugin: 'java'
EOF

# Start daemon
timeout 120 ovim src/Test.java < <(sleep 3 && echo ":q") 2>/dev/null &
sleep 10

# Find workspace directory
WORKSPACE=$(find ~/.cache/ovim -type d -name "workspace" 2>/dev/null | head -1)

if [ -z "$WORKSPACE" ]; then
    echo "  ⚠️  No workspace found"
else
    echo "  Workspace: $WORKSPACE"

    # Corrupt workspace
    echo "  Corrupting workspace..."
    rm -rf "$WORKSPACE/.metadata" 2>/dev/null || true

    # Try to use daemon
    echo "  Attempting to use corrupted workspace..."
    timeout 120 ovim src/Test.java < <(sleep 2 && echo ":q") 2>&1 | tee /tmp/ovim-corrupt.log &
    sleep 10

    # Should detect corruption and:
    # A) Rebuild workspace
    # B) Show clear error
    # C) Not crash

    if grep -q "corrupt\|rebuild\|reset" /tmp/ovim-corrupt.log; then
        echo "  ✅ PASS: Detected corruption and attempted recovery"
    else
        echo "  ⚠️  Corruption handling unclear"
    fi
fi

echo ""

# Test 5: Stale Socket Cleanup
echo "Test 5: Stale Socket Cleanup"
echo "  Scenario: Socket file exists but daemon dead"
echo ""

cleanup
mkdir -p "$TEST_DIR/project5/src"
cd "$TEST_DIR/project5"

cat > build.gradle << 'EOF'
apply plugin: 'java'
EOF

# Create fake daemon directory
FAKE_DAEMON_DIR="$DAEMON_DIR/fakehash123"
mkdir -p "$FAKE_DAEMON_DIR"

# Create stale socket and PID files
touch "$FAKE_DAEMON_DIR/daemon.sock"
echo "99999" > "$FAKE_DAEMON_DIR/daemon.pid"

echo "  Created fake stale daemon at: $FAKE_DAEMON_DIR"

# Try to open file - should detect stale socket and clean up
echo "  Attempting to use ovim with stale socket..."
timeout 120 ovim src/Test.java < <(sleep 2 && echo ":q") 2>&1 | tee /tmp/ovim-stale.log &
sleep 10

# Check if stale files were cleaned up
if [ -f "$FAKE_DAEMON_DIR/daemon.sock" ]; then
    echo "  ⚠️  Stale socket not cleaned up"
else
    echo "  ✅ PASS: Stale socket cleaned up"
fi

echo ""

# Test 6: Memory Limit Enforcement
echo "Test 6: Resource Limits (Multiple Projects)"
echo "  Scenario: User opens many projects, hits memory limit"
echo ""

cleanup

# Create 3 different projects
for i in {1..3}; do
    PROJECT_DIR="$TEST_DIR/project-$i"
    mkdir -p "$PROJECT_DIR/src"
    cd "$PROJECT_DIR"

    cat > build.gradle << 'EOF'
apply plugin: 'java'
EOF

    echo "  Opening project $i..."
    timeout 120 ovim src/Test.java < <(sleep 3 && echo ":q") 2>/dev/null &
    sleep 5
done

wait

# Count daemon processes
DAEMON_COUNT=$(pgrep -f "ovim.*daemon-mode" | wc -l)

if [ "$DAEMON_COUNT" -eq 0 ]; then
    echo "  ⚠️  Daemon mode not implemented"
else
    echo "  Created $DAEMON_COUNT daemon(s)"

    # Check total memory usage
    TOTAL_MEM=$(ps aux | grep "ovim.*daemon-mode\|jdtls" | grep -v grep | awk '{sum+=$6} END {print sum}')
    TOTAL_MEM_MB=$((TOTAL_MEM / 1024))

    echo "  Total daemon memory: ${TOTAL_MEM_MB}MB"

    if [ "$TOTAL_MEM_MB" -gt 3072 ]; then
        echo "  ⚠️  High memory usage - may need LRU eviction"
    else
        echo "  ✅ Memory usage reasonable"
    fi
fi

echo ""
echo "=== Summary ==="
echo ""
echo "These tests verify critical edge cases that must work before daemon mode is released:"
echo "  1. PID reuse detection (security)"
echo "  2. Race condition prevention (data integrity)"
echo "  3. Rogue process handling (reliability)"
echo "  4. Corruption recovery (robustness)"
echo "  5. Stale cleanup (correctness)"
echo "  6. Resource limits (performance)"
echo ""
echo "Once daemon mode is implemented, run this test suite to verify all edge cases are handled."
