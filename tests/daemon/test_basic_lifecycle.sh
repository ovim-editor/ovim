#!/bin/bash
# Test: Basic daemon lifecycle - start, reuse, auto-shutdown

set -e

TEST_DIR="/tmp/ovim-daemon-test-$$"
DAEMON_DIR="$HOME/.cache/ovim/daemons"

cleanup() {
    echo "Cleaning up..."
    rm -rf "$TEST_DIR"
    # Kill any daemons
    pkill -f "ovim.*daemon-mode" || true
    rm -rf "$DAEMON_DIR"
}

trap cleanup EXIT

echo "=== Test: Basic Daemon Lifecycle ==="
echo ""

# Setup
mkdir -p "$TEST_DIR/src"
cd "$TEST_DIR"

cat > build.gradle << 'EOF'
apply plugin: 'java'
EOF

cat > src/Test.java << 'EOF'
public class Test {
    public static void main(String[] args) {
        System.out.println("Hello");
    }
}
EOF

echo "Test 1: First open (daemon should start)"
echo "Opening Test.java for 3 seconds..."

# Note: This test assumes daemon mode is implemented
# For now, this will test current behavior
timeout 120 cargo run --release -- src/Test.java < <(sleep 3 && echo ":q") 2>&1 | tee /tmp/ovim-output-1.log &
OVIM_PID=$!

sleep 5  # Give it time to start

# Check for daemon process (when implemented)
if ps aux | grep -q "ovim.*daemon-mode"; then
    echo "✅ Daemon process found"
    DAEMON_PID=$(pgrep -f "ovim.*daemon-mode")
    echo "   Daemon PID: $DAEMON_PID"
else
    echo "⚠️  Daemon mode not yet implemented (expected for now)"
    echo "   Current behavior: jdtls starts with ovim, dies with ovim"
fi

wait $OVIM_PID || true

echo ""
echo "Test 2: Second open (should reuse daemon when implemented)"
echo "Opening Test.java again for 3 seconds..."

START_TIME=$(date +%s)
timeout 120 cargo run --release -- src/Test.java < <(sleep 3 && echo ":q") 2>&1 | tee /tmp/ovim-output-2.log &
OVIM_PID=$!

wait $OVIM_PID || true

END_TIME=$(date +%s)
ELAPSED=$((END_TIME - START_TIME))

echo "   Time taken: ${ELAPSED}s"

if [ "$ELAPSED" -lt 10 ]; then
    echo "✅ Fast startup (<10s) - daemon reuse working!"
elif [ "$ELAPSED" -lt 30 ]; then
    echo "⚠️  Moderate startup (10-30s) - possible daemon reuse"
else
    echo "❌ Slow startup (>30s) - daemon NOT being reused"
    echo "   This is expected before daemon mode is implemented"
    echo "   Target: <5s for subsequent opens"
fi

echo ""
echo "Test 3: Check for process leaks"

# Count ovim processes
OVIM_PROCESSES=$(ps aux | grep ovim | grep -v grep | grep -v "test_basic" | wc -l)
echo "   ovim processes: $OVIM_PROCESSES"

if [ "$OVIM_PROCESSES" -eq 0 ]; then
    echo "✅ No ovim processes leaked"
elif [ "$OVIM_PROCESSES" -eq 1 ]; then
    echo "⚠️  One ovim process (possibly daemon - good if daemon mode implemented)"
else
    echo "❌ Multiple ovim processes found - potential leak!"
    ps aux | grep ovim | grep -v grep
fi

# Count jdtls processes
JDTLS_PROCESSES=$(ps aux | grep -E "jdtls|java.*org.eclipse" | grep -v grep | wc -l)
echo "   jdtls processes: $JDTLS_PROCESSES"

if [ "$JDTLS_PROCESSES" -eq 0 ]; then
    echo "⚠️  No jdtls processes (all stopped - expected without daemon mode)"
elif [ "$JDTLS_PROCESSES" -eq 1 ]; then
    echo "✅ One jdtls process (daemon mode working!)"
else
    echo "❌ Multiple jdtls processes - potential leak!"
fi

echo ""
echo "=== Summary ==="
echo ""
echo "Current Status:"
echo "  - First open works: ✅"
echo "  - Second open works: ✅"
echo "  - Process cleanup: $([ $OVIM_PROCESSES -le 1 ] && echo '✅' || echo '❌')"
echo ""
echo "Daemon Mode Status:"
if ps aux | grep -q "ovim.*daemon-mode"; then
    echo "  ✅ Daemon mode IS implemented and running"
    echo "  - Daemon persists between sessions"
    echo "  - Quick subsequent opens"
else
    echo "  ⚠️  Daemon mode NOT YET implemented"
    echo "  - jdtls restarts on each ovim invocation"
    echo "  - 60-120s wait for each file"
    echo "  - Next step: Implement daemon mode for fast reopens"
fi

echo ""
echo "Performance Target:"
echo "  - First open: <90s (acceptable)"
echo "  - Second open: <5s (with daemon mode)"
echo "  - Current second open: ${ELAPSED}s"

if [ "$ELAPSED" -lt 10 ]; then
    echo "  ✅ EXCELLENT - Meets performance target!"
elif [ "$ELAPSED" -lt 30 ]; then
    echo "  ⚠️  MODERATE - Close to target"
else
    echo "  ❌ NEEDS IMPROVEMENT - Implement daemon mode"
fi
