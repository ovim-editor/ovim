#!/bin/bash

# Start ovim in headless mode
cargo run -- test_syntax.rs --headless 2>&1 &
OVIM_PID=$!

# Wait for it to start
sleep 2

# Get the port
PORT=$(lsof -iTCP -sTCP:LISTEN -n -P 2>/dev/null | grep ovim | awk '{print $9}' | head -1 | sed 's/.*://')

if [ -z "$PORT" ]; then
    echo "Failed to find ovim port"
    kill $OVIM_PID 2>/dev/null
    exit 1
fi

echo "Found ovim on port $PORT"

# Trigger live grep
echo "Triggering live grep..."
curl -s -X POST http://127.0.0.1:$PORT/keys -H "Content-Type: application/json" -d '{"keys": " sg"}'

# Try to render
echo "Attempting to render..."
curl -s http://127.0.0.1:$PORT/render | jq -r '.ansi' | head -5

# Check if ovim is still running
sleep 1
if ps -p $OVIM_PID > /dev/null; then
    echo "ovim still running"
    kill $OVIM_PID
else
    echo "ovim crashed!"
fi
