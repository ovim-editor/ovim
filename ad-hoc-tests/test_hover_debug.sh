#!/bin/bash
set -e

# Kill any existing test session
./ovim-ctl kill hover_test 2>/dev/null || true
sleep 1

# Start ovim in headless mode with a Rust file
echo "Starting ovim in headless mode..."
./target/release/ovim src/editor/mod.rs --headless --session hover_test 2>&1 &
OVIM_PID=$!

# Wait for session to be ready
echo "Waiting for session to start..."
sleep 2

# Wait for LSP to be ready
echo "Waiting for LSP to be ready..."
timeout 30 ./ovim-ctl wait hover_test 30 || {
    echo "LSP failed to initialize"
    ./ovim-ctl kill hover_test
    exit 1
}

echo "LSP is ready, navigating to hover test position..."

# Navigate to a known position with hover info
# Go to line 2000 which should have some function definitions
./ovim-ctl send hover_test "2000G"
sleep 0.5

# Move to a word
./ovim-ctl send hover_test "w"
sleep 0.5

# Trigger hover with K
echo "Triggering hover..."
./ovim-ctl send hover_test "K"

# Wait for hover to process
sleep 2

# Get the snapshot to see if hover worked
echo "Getting snapshot..."
curl -s http://127.0.0.1:$(cat ~/.cache/ovim/sessions/hover_test.json | grep -o '"port":[0-9]*' | cut -d: -f2)/snapshot | jq '.hover_info'

# Cleanup
echo "Cleaning up..."
./ovim-ctl kill hover_test

echo "Test complete!"
