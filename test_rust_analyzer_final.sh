#!/bin/bash

# Comprehensive rust-analyzer LSP test
# Tests: initialize, didOpen, hover on known symbols

set -e

WORKSPACE_ROOT="/workspace"
TEST_FILE="$WORKSPACE_ROOT/src/buffer/mod.rs"
TEST_FILE_URI="file://$TEST_FILE"

echo "=== Rust-Analyzer LSP Manual Test ===" >&2
echo "Workspace: $WORKSPACE_ROOT" >&2
echo "Test file: $TEST_FILE" >&2
echo "" >&2

# Read the FULL file content (rust-analyzer needs the complete file for accurate hover)
FILE_CONTENT=$(cat "$TEST_FILE")

# Function to send a JSON-RPC message with Content-Length header
send_message() {
    local message="$1"
    local length=${#message}
    printf "Content-Length: %d\r\n\r\n%s" "$length" "$message"
}

# Create initialize request
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
            "initializationOptions": {
                "checkOnSave": {
                    "enable": false
                }
            }
        }
    }'
}

# Create initialized notification
create_initialized() {
    jq -nc '{
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    }'
}

# Create didOpen notification
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

# Create hover request
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

# Create shutdown request (params should be null, not {})
create_shutdown() {
    jq -nc '{
        "jsonrpc": "2.0",
        "id": 99,
        "method": "shutdown",
        "params": null
    }'
}

# Create exit notification
create_exit() {
    jq -nc '{
        "jsonrpc": "2.0",
        "method": "exit"
    }'
}

echo "=== Starting rust-analyzer ===" >&2

# Send all messages to rust-analyzer
{
    echo ">>> Step 1: Initialize" >&2
    send_message "$(create_initialize)"
    sleep 2

    echo ">>> Step 2: Initialized notification" >&2
    send_message "$(create_initialized)"
    sleep 1

    echo ">>> Step 3: Open document" >&2
    send_message "$(create_did_open "$FILE_CONTENT")"

    echo ">>> Waiting 5 seconds for workspace indexing..." >&2
    sleep 5

    echo ">>> Step 4: Hover on 'Rope' at line 9, char 24" >&2
    # Line 9 is: "use ropey::Rope;"
    # Character 24 should be on "Rope"
    send_message "$(create_hover 9 24)"
    sleep 2

    echo ">>> Step 5: Hover on 'Buffer' at line 58, char 15" >&2
    # Line 58 is: "pub struct Buffer {"
    # Character 15 should be on "Buffer"
    send_message "$(create_hover 58 15)"
    sleep 2

    echo ">>> Step 6: Hover on 'Cursor' at line 62, char 15" >&2
    # Line 62 is: "    cursor: Cursor,"
    # Character 15 should be on "Cursor"
    send_message "$(create_hover 62 15)"
    sleep 2

    echo ">>> Step 7: Shutdown" >&2
    send_message "$(create_shutdown)"
    sleep 1

    echo ">>> Step 8: Exit" >&2
    send_message "$(create_exit)"
    sleep 1
} | rust-analyzer 2>&1 | tee /tmp/ra_full_output.log

echo "" >&2
echo "=== Test complete ===" >&2
echo "Full output saved to /tmp/ra_full_output.log" >&2
echo "" >&2
echo "=== Parsing responses ===" >&2

# Parse and display responses
echo "Looking for hover responses..." >&2
grep -A 5 '"id":2' /tmp/ra_full_output.log | head -20 || echo "No hover response found for request 2"
