#!/bin/bash
API_URL="http://127.0.0.1:${1:-8080}"

# Test $ motion
echo "Testing $ motion..."
curl -s -X PUT "$API_URL/buffer" -H "Content-Type: application/json" -d '{"content": "  hello world  "}' > /dev/null

# Get line length info
BUFFER=$(curl -s "$API_URL/buffer" | jq -r '.content')
echo "Buffer content: '$BUFFER'"
echo "Buffer length: ${#BUFFER}"

# Move to end of line with $
curl -s -X POST "$API_URL/keys" -H "Content-Type: application/json" -d '{"keys": "$"}' > /dev/null

# Get cursor position
CURSOR=$(curl -s "$API_URL/cursor")
echo "Cursor after $: $CURSOR"

# Also test what we think the line is
curl -s -X POST "$API_URL/keys" -H "Content-Type: application/json" -d '{"keys": "0"}' > /dev/null
for i in {0..20}; do
  POS=$(curl -s "$API_URL/cursor" | jq -r '.column')
  echo "Position $i: column $POS"
  curl -s -X POST "$API_URL/keys" -H "Content-Type: application/json" -d '{"keys": "l"}' > /dev/null
  NEW_POS=$(curl -s "$API_URL/cursor" | jq -r '.column')
  if [ "$POS" == "$NEW_POS" ]; then
    echo "Reached end at column $POS"
    break
  fi
done
