#!/usr/bin/env bash

# Script to manually test rust-analyzer LSP protocol
# This helps understand exactly what messages rust-analyzer expects and returns

set -euo pipefail

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ovim-ra-test.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT
OUTPUT_LOG="$TMP_DIR/ra_test_output.log"

WORKSPACE_ROOT="/workspace"
TEST_FILE="$WORKSPACE_ROOT/src/buffer/mod.rs"
TEST_FILE_URI="file://$TEST_FILE"

echo "=== Testing rust-analyzer LSP Protocol ==="
echo "Workspace: $WORKSPACE_ROOT"
echo "Test file: $TEST_FILE"
echo "Test file URI: $TEST_FILE_URI"
echo ""

# Read the first 100 lines of the test file for didOpen
FILE_CONTENT=$(head -100 "$TEST_FILE" | jq -Rs .)

echo "=== Starting rust-analyzer ==="
echo ""

# Start rust-analyzer in the background, capturing stderr for diagnostics
rust-analyzer 2>&1 | {
    echo "=== Step 1: Sending initialize request ==="
    cat <<EOF
Content-Length: 1024

{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":"file://$WORKSPACE_ROOT","capabilities":{"textDocument":{"hover":{"contentFormat":["markdown","plaintext"]},"synchronization":{"didSave":true}}},"initializationOptions":{}}}
EOF

    echo ""
    echo "=== Waiting for initialize response ==="
    # Read response
    read -t 5 response || true
    echo "Response: $response"

    echo ""
    echo "=== Step 2: Sending initialized notification ==="
    cat <<EOF
Content-Length: 59

{"jsonrpc":"2.0","method":"initialized","params":{}}
EOF

    sleep 1

    echo ""
    echo "=== Step 3: Sending textDocument/didOpen notification ==="
    # This is a notification, so no response expected
    CONTENT_JSON=$(echo "$FILE_CONTENT" | jq -c .)
    DID_OPEN=$(jq -nc --arg uri "$TEST_FILE_URI" --argjson content "$CONTENT_JSON" '{
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": $uri,
                "languageId": "rust",
                "version": 1,
                "text": $content
            }
        }
    }')
    DID_OPEN_LEN=$(echo -n "$DID_OPEN" | wc -c | tr -d ' ')
    echo "Content-Length: $DID_OPEN_LEN"
    echo ""
    echo "$DID_OPEN"

    echo ""
    echo "=== Waiting for LSP to process file (3 seconds) ==="
    sleep 3

    echo ""
    echo "=== Step 4: Sending textDocument/hover request ==="
    # Request hover at line 5, character 10 (should be on "ChangeManager" or similar)
    HOVER_REQUEST=$(jq -nc --arg uri "$TEST_FILE_URI" '{
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/hover",
        "params": {
            "textDocument": {
                "uri": $uri
            },
            "position": {
                "line": 5,
                "character": 20
            }
        }
    }')
    HOVER_LEN=$(echo -n "$HOVER_REQUEST" | wc -c | tr -d ' ')
    echo "Content-Length: $HOVER_LEN"
    echo ""
    echo "$HOVER_REQUEST"

    echo ""
    echo "=== Waiting for hover response (5 seconds) ==="
    sleep 5

    echo ""
    echo "=== Step 5: Sending shutdown request ==="
    cat <<EOF
Content-Length: 56

{"jsonrpc":"2.0","id":3,"method":"shutdown","params":{}}
EOF

    sleep 1

    echo ""
    echo "=== Step 6: Sending exit notification ==="
    cat <<EOF
Content-Length: 41

{"jsonrpc":"2.0","method":"exit","params":{}}
EOF

    sleep 1
} | tee "$OUTPUT_LOG"

echo ""
echo "=== Test complete ==="
echo "Full output saved to $OUTPUT_LOG"
