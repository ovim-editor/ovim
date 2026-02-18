#!/usr/bin/env bash

set -euo pipefail
API_URL="http://127.0.0.1:46769"

echo "=== Testing ; and , (find repeat) ==="

# Setup: Create buffer with multiple 'h' characters
curl -s -X PUT "$API_URL/v1/buffer" -H "Content-Type: application/json" \
  -d '{"content": "hello there hello world hello again"}' > /dev/null

# Move to start
curl -s -X POST "$API_URL/v1/keys" -H "Content-Type: application/json" \
  -d '{"keys": "gg0"}' > /dev/null

echo "Initial buffer: 'hello there hello world hello again'"
CURSOR=$(curl -s "$API_URL/v1/cursor" | jq -r '"\(.line),\(.column)"')
echo "Initial cursor: $CURSOR"

# Find 'h' forward
echo ""
echo "Sending 'fh' (find 'h' forward)..."
curl -s -X POST "$API_URL/v1/keys" -H "Content-Type: application/json" \
  -d '{"keys": "fh"}' > /dev/null
CURSOR=$(curl -s "$API_URL/v1/cursor" | jq -r '"\(.line),\(.column)"')
echo "After 'fh': cursor at $CURSOR (expected: 0,0 - first 'h' in hello)"

# Repeat with ;
echo ""
echo "Sending ';' (repeat find forward)..."
curl -s -X POST "$API_URL/v1/keys" -H "Content-Type: application/json" \
  -d '{"keys": ";"}' > /dev/null
CURSOR=$(curl -s "$API_URL/v1/cursor" | jq -r '"\(.line),\(.column)"')
echo "After ';': cursor at $CURSOR (expected: 0,12 - 'h' in second hello)"

# Repeat again
echo ""
echo "Sending ';' again..."
curl -s -X POST "$API_URL/v1/keys" -H "Content-Type: application/json" \
  -d '{"keys": ";"}' > /dev/null
CURSOR=$(curl -s "$API_URL/v1/cursor" | jq -r '"\(.line),\(.column)"')
echo "After second ';': cursor at $CURSOR (expected: 0,24 - 'h' in third hello)"

# Reverse with ,
echo ""
echo "Sending ',' (repeat find backward)..."
curl -s -X POST "$API_URL/v1/keys" -H "Content-Type: application/json" \
  -d '{"keys": ","}' > /dev/null
CURSOR=$(curl -s "$API_URL/v1/cursor" | jq -r '"\(.line),\(.column)"')
echo "After ',': cursor at $CURSOR (expected: 0,12 - back to second 'h')"

# Test with 't' (till)
echo ""
echo ""
echo "=== Testing with 't' (till) ==="
curl -s -X POST "$API_URL/v1/keys" -H "Content-Type: application/json" \
  -d '{"keys": "0"}' > /dev/null
echo "Reset to start of line"

echo "Sending 'tw' (till 'w')..."
curl -s -X POST "$API_URL/v1/keys" -H "Content-Type: application/json" \
  -d '{"keys": "tw"}' > /dev/null
CURSOR=$(curl -s "$API_URL/v1/cursor" | jq -r '"\(.line),\(.column)"')
echo "After 'tw': cursor at $CURSOR (expected: 0,17 - one before 'w' in world)"

echo "Sending ';' to repeat..."
curl -s -X POST "$API_URL/v1/keys" -H "Content-Type: application/json" \
  -d '{"keys": ";"}' > /dev/null
CURSOR=$(curl -s "$API_URL/v1/cursor" | jq -r '"\(.line),\(.column)"')
echo "After ';': cursor at $CURSOR (should find no more 'w's or stay put)"

echo ""
echo "=== Getting full snapshot for debugging ==="
curl -s "$API_URL/v1/snapshot" | jq '{mode: .mode, cursor: .cursor, buffer_preview: (.buffer.content[0:60])}'
