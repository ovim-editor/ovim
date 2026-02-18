#!/usr/bin/env bash

# Script to manually test rust-analyzer LSP protocol
# Uses proper JSON-RPC communication with Content-Length headers

set -euo pipefail

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ovim-ra-v2.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT
OUTPUT_LOG="$TMP_DIR/ra_output.log"

WORKSPACE_ROOT="/workspace"
TEST_FILE="$WORKSPACE_ROOT/src/buffer/mod.rs"
TEST_FILE_URI="file://$TEST_FILE"

echo "=== Testing rust-analyzer LSP Protocol ===" >&2
echo "Workspace: $WORKSPACE_ROOT" >&2
echo "Test file: $TEST_FILE" >&2
echo "Test file URI: $TEST_FILE_URI" >&2
echo "" >&2

# Read the file content (first 50 lines to keep it manageable)
FILE_CONTENT=$(head -50 "$TEST_FILE")

# Function to send a JSON-RPC message with Content-Length header
send_message() {
    local message="$1"
    local length=${#message}
    echo "Content-Length: $length"
    echo ""
    echo -n "$message"
}

# Function to create initialize request
create_initialize() {
    jq -nc --arg root "file://$WORKSPACE_ROOT" '{
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "processId": null,
            "rootUri": $root,
            "capabilities": {
                "textDocument": {
                    "hover": {
                        "contentFormat": ["markdown", "plaintext"]
                    },
                    "synchronization": {
                        "didSave": true
                    }
                }
            },
            "initializationOptions": {}
        }
    }'
}

# Function to create initialized notification
create_initialized() {
    jq -nc '{
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    }'
}

# Function to create didOpen notification
create_did_open() {
    local content="$1"
    jq -nc --arg uri "$TEST_FILE_URI" --arg content "$content" '{
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
    }'
}

# Function to create hover request
create_hover() {
    local line="$1"
    local char="$2"
    jq -nc --arg uri "$TEST_FILE_URI" --argjson line "$line" --argjson char "$char" '{
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/hover",
        "params": {
            "textDocument": {
                "uri": $uri
            },
            "position": {
                "line": $line,
                "character": $char
            }
        }
    }'
}

# Function to create shutdown request
create_shutdown() {
    jq -nc '{
        "jsonrpc": "2.0",
        "id": 3,
        "method": "shutdown",
        "params": {}
    }'
}

# Function to create exit notification
create_exit() {
    jq -nc '{
        "jsonrpc": "2.0",
        "method": "exit"
    }'
}

echo "=== Starting rust-analyzer ===" >&2

# Send all messages to rust-analyzer
{
    echo ">>> Sending initialize request" >&2
    send_message "$(create_initialize)"

    sleep 2

    echo ">>> Sending initialized notification" >&2
    send_message "$(create_initialized)"

    sleep 1

    echo ">>> Sending textDocument/didOpen notification" >&2
    send_message "$(create_did_open "$FILE_CONTENT")"

    sleep 3

    echo ">>> Sending textDocument/hover request (line 5, char 20)" >&2
    send_message "$(create_hover 5 20)"

    sleep 3

    echo ">>> Sending shutdown request" >&2
    send_message "$(create_shutdown)"

    sleep 1

    echo ">>> Sending exit notification" >&2
    send_message "$(create_exit)"

    sleep 1
} | rust-analyzer 2>&1 | tee "$OUTPUT_LOG"

echo "" >&2
echo "=== Test complete ===" >&2
echo "Output saved to $OUTPUT_LOG" >&2
