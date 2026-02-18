#!/usr/bin/env bash
set -euo pipefail

SESSION="hover_fix_test"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ovim-hover-fix.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT
LOG_FILE="$TMP_DIR/hover_test.log"

# Kill any existing test session
./ovim-ctl kill $SESSION 2>/dev/null || true
sleep 1

echo "=== Testing Hover Fix ==="
echo ""

# Start ovim in headless mode with a Rust file
echo "1. Starting ovim in headless mode..."
./target/release/ovim src/editor/mod.rs --headless --session $SESSION 2>&1 | tee "$LOG_FILE" &
OVIM_PID=$!

# Wait for session to be ready
echo "2. Waiting for session to start..."
sleep 3

# Get the port
if [ ! -f ~/.cache/ovim/sessions/$SESSION.json ]; then
    echo "ERROR: Session file not found"
    kill $OVIM_PID 2>/dev/null || true
    exit 1
fi

PORT=$(cat ~/.cache/ovim/sessions/$SESSION.json | grep -o '"port":[0-9]*' | cut -d: -f2)
echo "   Session running on port: $PORT"

# Wait for LSP to be ready
echo "3. Waiting for LSP to be ready..."
timeout 60 ./ovim-ctl wait $SESSION 60 || {
    echo "ERROR: LSP failed to initialize within 60 seconds"
    ./ovim-ctl kill $SESSION
    exit 1
}

echo "4. LSP is ready! Checking LSP status..."
curl -s "http://127.0.0.1:$PORT/v1/lsp/status" | jq '.'

echo ""
echo "5. Testing hover functionality..."
echo "   a. Navigating to line 100 (should have some code)..."
./ovim-ctl send $SESSION "100G"
sleep 0.5

echo "   b. Moving to a word..."
./ovim-ctl send $SESSION "w"
sleep 0.5

echo "   c. Getting initial snapshot..."
BEFORE=$(curl -s "http://127.0.0.1:$PORT/v1/snapshot")
echo "      Cursor position: $(echo $BEFORE | jq -r '.cursor')"
echo "      Mode: $(echo $BEFORE | jq -r '.mode')"

echo "   d. Triggering hover with K..."
./ovim-ctl send $SESSION "K"
sleep 2

echo "   e. Getting snapshot after hover..."
AFTER=$(curl -s "http://127.0.0.1:$PORT/v1/snapshot")
HOVER_INFO=$(echo $AFTER | jq -r '.hover_info')

if [ "$HOVER_INFO" != "null" ] && [ "$HOVER_INFO" != "" ]; then
    echo "   ✓ SUCCESS: Hover information received!"
    echo "   Hover content preview:"
    echo "$HOVER_INFO" | head -20
else
    echo "   ✗ FAILED: No hover information received"
    echo "   Full snapshot:"
    echo $AFTER | jq '.'
fi

echo ""
echo "6. Testing that LSP is still responsive (no blocking)..."
echo "   Checking LSP status again..."
LSP_STATUS=$(curl -s "http://127.0.0.1:$PORT/v1/lsp/status")
echo $LSP_STATUS | jq '.'

if echo $LSP_STATUS | jq -e '.servers | length > 0' > /dev/null; then
    echo "   ✓ SUCCESS: LSP is still responsive!"
else
    echo "   ✗ FAILED: LSP appears to be blocked"
fi

echo ""
echo "7. Cleaning up..."
./ovim-ctl kill $SESSION

echo ""
echo "=== Test Complete ==="
