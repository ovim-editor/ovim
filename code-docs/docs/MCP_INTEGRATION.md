# Model Context Protocol (MCP) Integration

ovim implements the [Model Context Protocol](https://modelcontextprotocol.io/) specification, allowing AI assistants and other MCP clients to interact with the editor through a standardized JSON-RPC 2.0 interface.

## Overview

MCP enables AI applications to:
- Control the editor (send keystrokes, execute commands)
- Access buffer content and editor state
- Trigger LSP features (hover, go-to-definition)
- Read resources (buffer, snapshot, LSP status)

## Endpoint

**POST** `/mcp`

All MCP requests are sent to this single endpoint using JSON-RPC 2.0 format.

## HTTP Server Availability

The HTTP server (including the MCP endpoint) now runs in **both headless and UI modes**:

- **Headless mode**: Session info includes port number in `~/.cache/ovim/sessions/SESSION.json`
- **UI mode**: Port is printed to stderr on startup: `REST API server listening on http://127.0.0.1:PORT`

## Supported MCP Methods

### 1. `initialize`

Establishes protocol compatibility and negotiates capabilities.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "protocolVersion": "2025-03-26",
    "capabilities": {},
    "clientInfo": {
      "name": "my-client",
      "version": "1.0.0"
    }
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2025-03-26",
    "capabilities": {
      "tools": {},
      "resources": {}
    },
    "serverInfo": {
      "name": "ovim",
      "version": "0.1.0"
    }
  }
}
```

### 2. `tools/list`

Lists all available tools that can be called.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/list",
  "params": {}
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "tools": [
      {
        "name": "send_keys",
        "description": "Send key sequences to the editor (Vim keybindings)",
        "inputSchema": {
          "type": "object",
          "properties": {
            "keys": {
              "type": "string",
              "description": "Vim key sequence (e.g., 'gg' for top, 'dd' for delete line)"
            }
          },
          "required": ["keys"]
        }
      },
      {
        "name": "get_buffer",
        "description": "Get the current buffer content",
        "inputSchema": {
          "type": "object",
          "properties": {}
        }
      },
      ...
    ]
  }
}
```

**Available Tools:**

| Tool | Description | Arguments |
|------|-------------|-----------|
| `send_keys` | Send Vim key sequences | `keys: string` |
| `get_buffer` | Get current buffer content | none |
| `set_buffer` | Replace entire buffer content | `content: string` |
| `get_cursor` | Get cursor position | none |
| `execute_command` | Execute ex command | `command: string` |
| `lsp_hover` | Get LSP hover info at cursor | none |
| `lsp_goto_definition` | Jump to definition | none |

### 3. `tools/call`

Executes a specific tool.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "send_keys",
    "arguments": {
      "keys": "ggdG"
    }
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "Keys sent successfully"
      }
    ]
  }
}
```

**Error Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "Error: Failed to parse keys",
        "isError": true
      }
    ]
  }
}
```

### 4. `resources/list`

Lists all available resources.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "resources/list",
  "params": {}
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "result": {
    "resources": [
      {
        "uri": "ovim://buffer",
        "name": "Current Buffer",
        "description": "The current editor buffer content",
        "mimeType": "text/plain"
      },
      {
        "uri": "ovim://snapshot",
        "name": "Editor Snapshot",
        "description": "Complete editor state including buffer, cursor, mode, registers",
        "mimeType": "application/json"
      },
      {
        "uri": "ovim://lsp/status",
        "name": "LSP Status",
        "description": "Language server status information",
        "mimeType": "application/json"
      },
      {
        "uri": "file:///path/to/current/file.rs",
        "name": "Current File",
        "description": "The file being edited: /path/to/current/file.rs",
        "mimeType": "text/plain"
      }
    ]
  }
}
```

### 5. `resources/read`

Reads a specific resource by URI.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "resources/read",
  "params": {
    "uri": "ovim://buffer"
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "result": {
    "contents": [
      {
        "uri": "ovim://buffer",
        "mimeType": "text/plain",
        "text": "fn main() {\n    println!(\"Hello, world!\");\n}\n"
      }
    ]
  }
}
```

**Resource URIs:**

| URI | Description | MIME Type |
|-----|-------------|-----------|
| `ovim://buffer` | Current buffer content | `text/plain` |
| `ovim://snapshot` | Complete editor state (JSON) | `application/json` |
| `ovim://lsp/status` | LSP server status (JSON) | `application/json` |
| `file://PATH` | Current file being edited | `text/plain` |

### 6. `prompts/list`

Lists available prompt templates (currently empty).

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "method": "prompts/list",
  "params": {}
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "result": {
    "prompts": []
  }
}
```

## Error Handling

MCP uses standard JSON-RPC 2.0 error codes:

| Code | Message | Description |
|------|---------|-------------|
| -32700 | Parse error | Invalid JSON |
| -32600 | Invalid Request | Missing required fields |
| -32601 | Method not found | Unknown method |
| -32602 | Invalid params | Parameter validation failed |
| -32603 | Internal error | Server-side failure |

**Error Response Example:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32601,
    "message": "Method not found",
    "data": {
      "method": "unknown/method"
    }
  }
}
```

## Complete Example Workflow

```bash
# Start ovim in headless mode
./target/release/ovim test.rs --headless --session test
PORT=$(cat ~/Library/Caches/ovim/sessions/test.json | jq -r '.port')

# Initialize MCP connection
curl -X POST "http://127.0.0.1:$PORT/mcp" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}'

# Get buffer content
curl -X POST "http://127.0.0.1:$PORT/mcp" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_buffer","arguments":{}}}'

# Send keys to navigate and edit
curl -X POST "http://127.0.0.1:$PORT/mcp" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"send_keys","arguments":{"keys":"ggO// New comment<Esc>"}}}'

# Read the updated buffer
curl -X POST "http://127.0.0.1:$PORT/mcp" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":4,"method":"resources/read","params":{"uri":"ovim://buffer"}}'

# Get editor snapshot
curl -X POST "http://127.0.0.1:$PORT/mcp" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":5,"method":"resources/read","params":{"uri":"ovim://snapshot"}}'

# Cleanup
./ovim-ctl kill test
```

## Implementation Details

### Architecture

- **Module**: `src/api/mcp.rs` - JSON-RPC types and MCP method handlers
- **Handler**: `src/api/mcp_handler.rs` - Request dispatcher and tool execution
- **Route**: POST `/mcp` in `src/api/routes.rs`

### Thread Safety

All MCP requests are processed on the main editor thread via the existing `ApiRequest` channel system, ensuring thread-safe state mutations.

### Tool Execution

Tools are mapped to existing `ApiRequest` variants:
- `send_keys` → `ApiRequest::SendKeys`
- `get_buffer` → `ApiRequest::GetBuffer`
- `set_buffer` → `ApiRequest::SetBuffer`
- `execute_command` → `ApiRequest::ExecuteCommand`
- etc.

### Protocol Version

Current implementation supports MCP protocol version `2025-03-26`.

## Testing

See `src/api/mcp.rs` for unit tests of JSON-RPC message parsing and MCP method handlers.

Integration tests can be run using the example workflow above.

## Future Enhancements

- [ ] Implement `prompts/get` with useful editor prompts
- [ ] Add `prompts/list` with refactoring/code generation prompts
- [ ] Support for notifications (buffer changes, LSP diagnostics)
- [ ] Streaming support for large resources
- [ ] Batch request support (JSON-RPC batch mode)
