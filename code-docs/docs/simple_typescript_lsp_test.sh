#!/bin/bash

# Simple TypeScript LSP Test
# Demonstrates the exact messages and responses for hover functionality

echo "=== TypeScript Language Server Hover Test ==="
echo ""

# Create test file
cat > /tmp/test_ts_lsp.ts <<'TSFILE'
const greeting: string = "Hello";
function add(a: number, b: number): number {
    return a + b;
}
TSFILE

echo "Test file created: /tmp/test_ts_lsp.ts"
echo ""

# Helper function
create_message() {
    local message="$1"
    local content_length=${#message}
    printf "Content-Length: %d\r\n\r\n%s" "$content_length" "$message"
}

# Messages
INIT_MSG='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":"file:///tmp","capabilities":{"textDocument":{"hover":{"contentFormat":["markdown","plaintext"]}}},"initializationOptions":{}}}'

INITIALIZED_MSG='{"jsonrpc":"2.0","method":"initialized","params":{}}'

DIDOPEN_MSG='{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///tmp/test_ts_lsp.ts","languageId":"typescript","version":1,"text":"const greeting: string = \"Hello\";\nfunction add(a: number, b: number): number {\n    return a + b;\n}\n"}}}'

HOVER_MSG='{"jsonrpc":"2.0","id":2,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///tmp/test_ts_lsp.ts"},"position":{"line":0,"character":6}}}'

echo "Sending messages to typescript-language-server:"
echo "1. initialize"
echo "2. initialized"
echo "3. didOpen"
echo "4. hover (on 'greeting' variable at line 0, char 6)"
echo ""
echo "Response:"
echo "---"

{
    create_message "$INIT_MSG"
    sleep 0.1
    create_message "$INITIALIZED_MSG"
    sleep 0.1
    create_message "$DIDOPEN_MSG"
    sleep 0.1
    create_message "$HOVER_MSG"
    sleep 2
} | timeout 10 ~/.local/bin/typescript-language-server --stdio 2>&1

echo ""
echo "---"
echo ""
echo "Note: The hover response (id:2) should contain:"
echo '  "contents": {'
echo '    "kind": "markdown",'
echo '    "value": "\\n```typescript\\nconst greeting: string\\n```\\n"'
echo "  }"
