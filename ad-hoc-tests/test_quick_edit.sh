#!/bin/bash
# Quick test: Does ovim work for the "rapid open/close" workflow?

set -e

echo "=== ovim Quick Edit Workflow Test ==="
echo ""

# Create test project
TEST_DIR="/tmp/ovim-quick-edit-test"
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR/src"

# Create 3 simple Java files
cat > "$TEST_DIR/src/File1.java" << 'EOF'
public class File1 {
    public static void main(String[] args) {
        System.out.println("File 1");
    }
}
EOF

cat > "$TEST_DIR/src/File2.java" << 'EOF'
public class File2 {
    public void method() {
        String x = "test";
    }
}
EOF

cat > "$TEST_DIR/src/File3.java" << 'EOF'
public class File3 {
    private int value = 42;
}
EOF

cd "$TEST_DIR"

echo "Test Setup:"
echo "  - Created 3 Java files in $TEST_DIR"
echo "  - Testing rapid open/close cycle"
echo ""

# Test 1: First open (will be slow - jdtls init)
echo "=== Test 1: First Open (File1.java) ==="
echo "Expected: 60-120 seconds (jdtls initialization)"
echo "Command: ovim src/File1.java (will auto-quit after 3 seconds)"
echo ""
START1=$(date +%s)
timeout 120 cargo run --release -- src/File1.java < <(sleep 3 && echo ":q") 2>&1 | grep -E "Java:|Error|Ready" || true
END1=$(date +%s)
TIME1=$((END1 - START1))
echo "⏱️  Time: ${TIME1}s"
echo ""

# Wait for jdtls to settle
sleep 2

# Check for jdtls process
echo "=== Checking for jdtls process ==="
JDTLS_COUNT=$(ps aux | grep -E "jdtls|java.*org.eclipse" | grep -v grep | wc -l)
if [ "$JDTLS_COUNT" -gt 0 ]; then
    echo "✅ jdtls is running ($JDTLS_COUNT processes)"
    ps aux | grep -E "jdtls|java.*org.eclipse" | grep -v grep
else
    echo "❌ jdtls is NOT running (may have exited with ovim)"
fi
echo ""

# Test 2: Second open (should this be fast?)
echo "=== Test 2: Second Open (File2.java) ==="
echo "Expected: <5 seconds if jdtls is reused, 60-120s if restarted"
echo "Command: ovim src/File2.java (will auto-quit after 3 seconds)"
echo ""
START2=$(date +%s)
timeout 120 cargo run --release -- src/File2.java < <(sleep 3 && echo ":q") 2>&1 | grep -E "Java:|Error|Ready" || true
END2=$(date +%s)
TIME2=$((END2 - START2))
echo "⏱️  Time: ${TIME2}s"
echo ""

# Test 3: Third open
echo "=== Test 3: Third Open (File3.java) ==="
echo "Expected: <5 seconds if jdtls is reused"
echo "Command: ovim src/File3.java (will auto-quit after 3 seconds)"
echo ""
START3=$(date +%s)
timeout 120 cargo run --release -- src/File3.java < <(sleep 3 && echo ":q") 2>&1 | grep -E "Java:|Error|Ready" || true
END3=$(date +%s)
TIME3=$((END3 - START3))
echo "⏱️  Time: ${TIME3}s"
echo ""

# Check for process leaks
echo "=== Checking for Process Leaks ==="
OVIM_COUNT=$(ps aux | grep ovim | grep -v grep | wc -l)
JDTLS_COUNT=$(ps aux | grep -E "jdtls|java.*org.eclipse" | grep -v grep | wc -l)
echo "ovim processes: $OVIM_COUNT (should be 0)"
echo "jdtls processes: $JDTLS_COUNT"
echo ""

# Summary
echo "=== SUMMARY ==="
echo ""
echo "Open Times:"
echo "  1st file: ${TIME1}s"
echo "  2nd file: ${TIME2}s"
echo "  3rd file: ${TIME3}s"
echo ""

# Analysis
if [ "$TIME2" -lt 10 ] && [ "$TIME3" -lt 10 ]; then
    echo "✅ EXCELLENT: Subsequent opens are fast (<10s)"
    echo "   ovim is reusing jdtls - perfect for quick edit workflow!"
elif [ "$TIME2" -gt 50 ] || [ "$TIME3" -gt 50 ]; then
    echo "❌ PROBLEM: Subsequent opens are slow (>50s)"
    echo "   ovim is restarting jdtls each time - terrible for quick edits"
    echo "   Need to implement LSP daemon/reuse mode"
else
    echo "⚠️  MODERATE: Subsequent opens take ${TIME2}s/${TIME3}s"
    echo "   Could be better. Target: <5s"
fi
echo ""

# Recommendations
echo "=== RECOMMENDATIONS ==="
echo ""
if [ "$TIME2" -gt 30 ]; then
    echo "CRITICAL: Implement jdtls daemon mode or LSP reuse"
    echo "  - Keep jdtls alive between editor sessions"
    echo "  - Detect same project and reuse LSP instance"
    echo "  - Target: <5s for 2nd/3rd opens"
fi

if [ "$OVIM_COUNT" -gt 0 ]; then
    echo "WARNING: Zombie ovim processes detected"
    echo "  - Fix process cleanup on exit"
fi

echo ""
echo "Test complete! Files in: $TEST_DIR"
echo "To test manually: cd $TEST_DIR && cargo run --release -- src/File1.java"
