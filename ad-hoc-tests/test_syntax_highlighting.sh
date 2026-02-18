#!/usr/bin/env bash

set -euo pipefail
# Test script to verify syntax highlighting is working

# Start ovim in headless mode
timeout 10 ./target/debug/ovim test_syntax.rs --headless &
OVIM_PID=$!

# Wait for server to start
sleep 2

# Get the port from ps output
PORT=$(lsof -p $OVIM_PID -a -i TCP -s TCP:LISTEN 2>/dev/null | grep LISTEN | awk '{print $9}' | cut -d: -f2 | head -1)

if [ -z "$PORT" ]; then
    echo "Failed to find port"
    kill $OVIM_PID 2>/dev/null
    exit 1
fi

echo "Server running on port $PORT"

# Fetch the render output
RENDER=$(curl -s http://127.0.0.1:$PORT/v1/render | jq -r '.ansi')

# Check if ANSI color codes are present
if echo "$RENDER" | grep -q '\x1b\[.*m'; then
    echo "✓ ANSI color codes found in output"
    echo "$RENDER" | head -20
else
    echo "✗ No ANSI color codes found - syntax highlighting may not be working"
    echo "$RENDER" | head -20
fi

# Check for specific colors that should be in Rust syntax highlighting
# Magenta (35) for keywords, Blue (34) for functions, etc.
if echo "$RENDER" | grep -q '\x1b\[.*35.*m'; then
    echo "✓ Found magenta color code (likely keywords)"
else
    echo "✗ No magenta color codes (expected for 'fn', 'let', 'impl', etc.)"
fi

# Clean up
kill $OVIM_PID 2>/dev/null

