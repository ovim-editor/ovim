#!/bin/bash

# Simple rust-analyzer test with a standalone file

set -e

WORKSPACE_ROOT="/workspace"
TEST_FILE="$WORKSPACE_ROOT/fixtures/simple_test.rs"
TEST_FILE_URI="file://$TEST_FILE"

echo "=== Simple Rust-Analyzer LSP Test ===" >&2
echo "Test file: $TEST_FILE" >&2

FILE_CONTENT=$(cat "$TEST_FILE")

send_message() {
    local message="$1"
    local length=${#message}
    printf "Content-Length: %d\r\n\r\n%s" "$length" "$message"
}

{
    echo ">>> Initialize" >&2
    send_message "$(jq -nc --arg root "file://$WORKSPACE_ROOT" '{
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
                    }
                }
            }
        }
    }')"
    sleep 2

    echo ">>> Initialized" >&2
    send_message '{"jsonrpc":"2.0","method":"initialized","params":{}}'
    sleep 1

    echo ">>> didOpen" >&2
    send_message "$(jq -nc --arg uri "$TEST_FILE_URI" --arg content "$FILE_CONTENT" '{
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
    }')"

    echo ">>> Waiting 8 seconds for indexing..." >&2
    sleep 8

    echo ">>> Hover on 'HashMap' at line 0, char 22" >&2
    send_message "$(jq -nc --arg uri "$TEST_FILE_URI" '{
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/hover",
        "params": {
            "textDocument": {"uri": $uri},
            "position": {"line": 0, "character": 22}
        }
    }')"
    sleep 3

    echo ">>> Hover on 'Point' at line 8, char 8" >&2
    send_message "$(jq -nc --arg uri "$TEST_FILE_URI" '{
        "jsonrpc": "2.0",
        "id": 3,
        "method": "textDocument/hover",
        "params": {
            "textDocument": {"uri": $uri},
            "position": {"line": 8, "character": 8}
        }
    }')"
    sleep 3

    echo ">>> Shutdown" >&2
    send_message '{"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}'
    sleep 1

    echo ">>> Exit" >&2
    send_message '{"jsonrpc":"2.0","method":"exit"}'
} | rust-analyzer 2>&1 | tee /tmp/simple_ra_output.log

echo "" >&2
echo "=== Extracting hover responses ===" >&2

# Extract responses with id 2 and 3
cat /tmp/simple_ra_output.log | grep -o '{"jsonrpc":"2.0","id":[0-9]*,"result":[^}]*}' | while read -r response; do
    echo "$response" | jq '.'
done
