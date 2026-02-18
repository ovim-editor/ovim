#!/usr/bin/env bash

# Test rust-analyzer with actual workspace file and longer wait time

set -euo pipefail

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ovim-ra-workspace.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT
OUTPUT_LOG="$TMP_DIR/ra_workspace_output.log"

WORKSPACE_ROOT="/workspace"
TEST_FILE="$WORKSPACE_ROOT/src/buffer/mod.rs"
TEST_FILE_URI="file://$TEST_FILE"

echo "=== Testing with Workspace Context ===" >&2
echo "Workspace: $WORKSPACE_ROOT" >&2
echo "Test file: $TEST_FILE" >&2
echo "" >&2

FILE_CONTENT=$(cat "$TEST_FILE")

send_message() {
    local message="$1"
    local length=${#message}
    printf "Content-Length: %d\r\n\r\n%s" "$length" "$message"
}

{
    echo ">>> Step 1: Initialize with full capabilities" >&2
    send_message "$(jq -nc --arg root "file://$WORKSPACE_ROOT" '{
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "processId": null,
            "rootUri": $root,
            "workspaceFolders": [{
                "uri": $root,
                "name": "ovim"
            }],
            "capabilities": {
                "workspace": {
                    "configuration": true,
                    "workspaceFolders": true
                },
                "textDocument": {
                    "hover": {
                        "dynamicRegistration": false,
                        "contentFormat": ["markdown", "plaintext"]
                    },
                    "synchronization": {
                        "dynamicRegistration": false,
                        "willSave": false,
                        "didSave": true,
                        "willSaveWaitUntil": false
                    }
                }
            },
            "initializationOptions": {
                "checkOnSave": {
                    "enable": false
                },
                "cargo": {
                    "loadOutDirsFromCheck": true
                }
            }
        }
    }')"

    echo ">>> Waiting for initialize response..." >&2
    sleep 3

    echo ">>> Step 2: Send initialized notification" >&2
    send_message '{"jsonrpc":"2.0","method":"initialized","params":{}}'

    echo ">>> Waiting for server to start indexing..." >&2
    sleep 2

    echo ">>> Step 3: Send didOpen for buffer/mod.rs" >&2
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

    echo ">>> Waiting 15 seconds for full workspace indexing..." >&2
    sleep 15

    # Test on line 8: "use ropey::Rope;"
    # The word "Rope" starts at character 14
    echo ">>> Step 4: Hover on 'Rope' (line 8, char 18)" >&2
    send_message "$(jq -nc --arg uri "$TEST_FILE_URI" '{
        "jsonrpc": "2.0",
        "id": 10,
        "method": "textDocument/hover",
        "params": {
            "textDocument": {"uri": $uri},
            "position": {"line": 8, "character": 18}
        }
    }')"

    echo ">>> Waiting for hover response..." >&2
    sleep 3

    # Test on line 57: "pub struct Buffer {"
    # The word "Buffer" starts around character 11
    echo ">>> Step 5: Hover on 'Buffer' (line 57, char 15)" >&2
    send_message "$(jq -nc --arg uri "$TEST_FILE_URI" '{
        "jsonrpc": "2.0",
        "id": 11,
        "method": "textDocument/hover",
        "params": {
            "textDocument": {"uri": $uri},
            "position": {"line": 57, "character": 15}
        }
    }')"

    echo ">>> Waiting for hover response..." >&2
    sleep 3

    # Test on std library function
    echo ">>> Step 6: Hover on 'PathBuf' (line 10, char 30)" >&2
    send_message "$(jq -nc --arg uri "$TEST_FILE_URI" '{
        "jsonrpc": "2.0",
        "id": 12,
        "method": "textDocument/hover",
        "params": {
            "textDocument": {"uri": $uri},
            "position": {"line": 10, "character": 30}
        }
    }')"

    echo ">>> Waiting for hover response..." >&2
    sleep 3

    echo ">>> Step 7: Shutdown" >&2
    send_message '{"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}'
    sleep 1

    echo ">>> Step 8: Exit" >&2
    send_message '{"jsonrpc":"2.0","method":"exit"}'
} | rust-analyzer 2>&1 | tee "$OUTPUT_LOG"

echo "" >&2
echo "=== Test complete ===" >&2
echo "Parsing results..." >&2
python3 /workspace/parse_lsp_output.py "$OUTPUT_LOG"
