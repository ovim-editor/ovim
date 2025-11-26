#!/bin/bash

# TypeScript Language Server Manual Testing Commands
# This script documents the exact commands used to test the TypeScript LSP
# Run individual sections to reproduce the tests

echo "=== TypeScript LSP Manual Test Commands ==="
echo ""

# Helper function to create JSON-RPC messages with proper headers
create_message() {
    local message="$1"
    local content_length=${#message}
    printf "Content-Length: %d\r\n\r\n%s" "$content_length" "$message"
}

# Test 1: Basic hover functionality
test_basic_hover() {
    echo "Test 1: Basic hover on variable"

    INIT_MSG='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":"file:///tmp","capabilities":{"textDocument":{"hover":{"contentFormat":["markdown","plaintext"]}}},"initializationOptions":{}}}'

    INITIALIZED_MSG='{"jsonrpc":"2.0","method":"initialized","params":{}}'

    DIDOPEN_MSG='{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///tmp/test_ts_lsp.ts","languageId":"typescript","version":1,"text":"const greeting: string = \"Hello\";\nfunction add(a: number, b: number): number {\n    return a + b;\n}\n"}}}'

    HOVER_MSG='{"jsonrpc":"2.0","id":2,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///tmp/test_ts_lsp.ts"},"position":{"line":0,"character":6}}}'

    {
        create_message "$INIT_MSG"
        sleep 0.1
        create_message "$INITIALIZED_MSG"
        sleep 0.1
        create_message "$DIDOPEN_MSG"
        sleep 1
        create_message "$HOVER_MSG"
        sleep 1
    } | ~/.local/bin/typescript-language-server --stdio 2>&1
}

# Test 2: Hover on function and parameters
test_function_hover() {
    echo "Test 2: Hover on function and parameters"

    INIT_MSG='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":"file:///tmp","capabilities":{"textDocument":{"hover":{"contentFormat":["markdown","plaintext"]}}},"initializationOptions":{}}}'

    INITIALIZED_MSG='{"jsonrpc":"2.0","method":"initialized","params":{}}'

    DIDOPEN_MSG='{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///tmp/test_ts_lsp.ts","languageId":"typescript","version":1,"text":"const greeting: string = \"Hello\";\nfunction add(a: number, b: number): number {\n    return a + b;\n}\n"}}}'

    HOVER_FUNC_MSG='{"jsonrpc":"2.0","id":2,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///tmp/test_ts_lsp.ts"},"position":{"line":1,"character":9}}}'

    HOVER_PARAM_MSG='{"jsonrpc":"2.0","id":3,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///tmp/test_ts_lsp.ts"},"position":{"line":1,"character":13}}}'

    {
        create_message "$INIT_MSG"
        sleep 0.1
        create_message "$INITIALIZED_MSG"
        sleep 0.1
        create_message "$DIDOPEN_MSG"
        sleep 1
        create_message "$HOVER_FUNC_MSG"
        sleep 0.5
        create_message "$HOVER_PARAM_MSG"
        sleep 1
    } | ~/.local/bin/typescript-language-server --stdio 2>&1
}

# Test 3: No wait after didOpen
test_no_wait() {
    echo "Test 3: Hover immediately after didOpen (no wait)"

    INIT_MSG='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":"file:///tmp","capabilities":{"textDocument":{"hover":{"contentFormat":["markdown","plaintext"]}}},"initializationOptions":{}}}'

    INITIALIZED_MSG='{"jsonrpc":"2.0","method":"initialized","params":{}}'

    DIDOPEN_MSG='{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///tmp/test_ts_lsp.ts","languageId":"typescript","version":1,"text":"const greeting: string = \"Hello\";\nfunction add(a: number, b: number): number {\n    return a + b;\n}\n"}}}'

    HOVER_MSG='{"jsonrpc":"2.0","id":2,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///tmp/test_ts_lsp.ts"},"position":{"line":0,"character":6}}}'

    {
        create_message "$INIT_MSG"
        sleep 0.1
        create_message "$INITIALIZED_MSG"
        sleep 0.1
        create_message "$DIDOPEN_MSG"
        # NO WAIT - hover immediately
        create_message "$HOVER_MSG"
        sleep 2
    } | timeout 10 ~/.local/bin/typescript-language-server --stdio 2>&1
}

# Test 4: didChange and immediate hover
test_didchange() {
    echo "Test 4: Hover after didChange (testing synchronous processing)"

    INIT_MSG='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":"file:///tmp","capabilities":{"textDocument":{"hover":{"contentFormat":["markdown","plaintext"]},"synchronization":{"didSave":true}}},"initializationOptions":{}}}'

    INITIALIZED_MSG='{"jsonrpc":"2.0","method":"initialized","params":{}}'

    DIDOPEN_MSG='{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///tmp/test_ts_lsp.ts","languageId":"typescript","version":1,"text":"const greeting: string = \"Hello\";\n"}}}'

    DIDCHANGE_MSG='{"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":"file:///tmp/test_ts_lsp.ts","version":2},"contentChanges":[{"text":"const greeting: string = \"Hello\";\nconst name: string = \"World\";\n"}]}}'

    HOVER_NEW_VAR='{"jsonrpc":"2.0","id":2,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///tmp/test_ts_lsp.ts"},"position":{"line":1,"character":6}}}'

    {
        create_message "$INIT_MSG"
        sleep 0.1
        create_message "$INITIALIZED_MSG"
        sleep 0.1
        create_message "$DIDOPEN_MSG"
        sleep 0.5
        create_message "$DIDCHANGE_MSG"
        # Hover IMMEDIATELY after didChange
        create_message "$HOVER_NEW_VAR"
        sleep 2
    } | timeout 15 ~/.local/bin/typescript-language-server --stdio 2>&1
}

# Test 5: Error cases (null results)
test_error_cases() {
    echo "Test 5: Hover on invalid positions (whitespace, punctuation, etc.)"

    INIT_MSG='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":"file:///tmp","capabilities":{"textDocument":{"hover":{"contentFormat":["markdown","plaintext"]}}},"initializationOptions":{}}}'

    INITIALIZED_MSG='{"jsonrpc":"2.0","method":"initialized","params":{}}'

    DIDOPEN_MSG='{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///tmp/test_ts_lsp.ts","languageId":"typescript","version":1,"text":"const greeting: string = \"Hello\";\nfunction add(a: number, b: number): number {\n    return a + b;\n}\n"}}}'

    # Hover on whitespace
    HOVER_WHITESPACE='{"jsonrpc":"2.0","id":2,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///tmp/test_ts_lsp.ts"},"position":{"line":0,"character":0}}}'

    # Hover on colon
    HOVER_COLON='{"jsonrpc":"2.0","id":3,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///tmp/test_ts_lsp.ts"},"position":{"line":0,"character":18}}}'

    {
        create_message "$INIT_MSG"
        sleep 0.1
        create_message "$INITIALIZED_MSG"
        sleep 0.1
        create_message "$DIDOPEN_MSG"
        sleep 0.5
        create_message "$HOVER_WHITESPACE"
        sleep 0.2
        create_message "$HOVER_COLON"
        sleep 2
    } | timeout 15 ~/.local/bin/typescript-language-server --stdio 2>&1
}

# Parse output helper
parse_output() {
    python3 - <<'EOF'
import re
import json
import sys

content = sys.stdin.read()

# Parse JSON-RPC messages
messages = []
pattern = r'Content-Length: (\d+)\r?\n\r?\n'
parts = re.split(pattern, content)

for i in range(1, len(parts), 2):
    if i+1 < len(parts):
        length = int(parts[i])
        message = parts[i+1][:length]
        try:
            msg_json = json.loads(message)
            messages.append(msg_json)
        except Exception as e:
            print(f"Failed to parse: {message[:100]}", file=sys.stderr)

# Print messages nicely
for i, msg in enumerate(messages):
    print(f"\n{'='*80}")
    print(f"Message {i+1}:")
    print('='*80)
    print(json.dumps(msg, indent=2))
EOF
}

# Run all tests
run_all_tests() {
    echo "Running all TypeScript LSP tests..."
    echo ""

    # Create test file
    cat > /tmp/test_ts_lsp.ts <<'TSFILE'
const greeting: string = "Hello";
function add(a: number, b: number): number {
    return a + b;
}
TSFILE

    echo "=== Test 1: Basic Hover ==="
    test_basic_hover | parse_output

    echo ""
    echo "=== Test 2: Function Hover ==="
    test_function_hover | parse_output

    echo ""
    echo "=== Test 3: No Wait After didOpen ==="
    test_no_wait | parse_output

    echo ""
    echo "=== Test 4: didChange and Immediate Hover ==="
    test_didchange | parse_output

    echo ""
    echo "=== Test 5: Error Cases (Null Results) ==="
    test_error_cases | parse_output
}

# Main
case "${1:-help}" in
    basic)
        test_basic_hover | parse_output
        ;;
    function)
        test_function_hover | parse_output
        ;;
    nowait)
        test_no_wait | parse_output
        ;;
    didchange)
        test_didchange | parse_output
        ;;
    errors)
        test_error_cases | parse_output
        ;;
    all)
        run_all_tests
        ;;
    help|*)
        echo "Usage: $0 {basic|function|nowait|didchange|errors|all}"
        echo ""
        echo "Tests:"
        echo "  basic     - Basic hover on a variable"
        echo "  function  - Hover on function and parameters"
        echo "  nowait    - Hover immediately after didOpen"
        echo "  didchange - Hover immediately after didChange"
        echo "  errors    - Test null results (whitespace, punctuation)"
        echo "  all       - Run all tests"
        ;;
esac
