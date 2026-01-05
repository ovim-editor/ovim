# Ovim REST API Documentation

## Overview

The Ovim REST API provides programmatic access to the editor's functionality via HTTP. This API is the foundation for:
- Headless mode control via CLI
- Model Context Protocol (MCP) integration
- AI coding assistant tooling
- Custom editor automation

## Versioning

The Ovim REST API uses **path-based versioning** to ensure forward compatibility and smooth migration paths.

### Current Version: v1

Base URL: `http://127.0.0.1:<PORT>/v1`

All endpoints are available under the `/v1/` prefix. This is the **recommended** way to use the API.

### Version Policy

**Backward Compatibility**: Within a major version (v1, v2), we maintain backward compatibility. This means:
- New endpoints can be added
- New fields can be added to responses
- Optional parameters can be added to requests
- No fields will be removed or changed in incompatible ways

**Deprecation Period**: When endpoints are deprecated:
- They continue working for at least 6 months
- Response headers include:
  - `X-API-Deprecation`: Human-readable deprecation message
  - `Sunset`: RFC 7234 sunset date indicating when the endpoint will be removed
- Migration guide is provided in release notes

**Breaking Changes**: Require a new major version (v2, v3). Breaking changes include:
- Removing fields from responses
- Changing field types or semantics
- Removing endpoints
- Changing required parameters

### Legacy Routes (Deprecated)

For backward compatibility, unversioned routes (`/buffer`, `/health`, etc.) currently work but are **deprecated** and will be removed in ovim v1.0.

**Migration**: Update your clients to use `/v1/` prefix:

```bash
# Old (deprecated) - will be removed in v1.0
curl http://127.0.0.1:3000/buffer

# New (recommended)
curl http://127.0.0.1:3000/v1/buffer
```

When using legacy routes, you'll receive deprecation headers:
```
X-API-Deprecation: Unversioned API paths are deprecated. Use /v1/* instead.
Sunset: Wed, 01 Jul 2026 00:00:00 GMT
```

## Authentication

Currently, the API has no authentication. It binds to `127.0.0.1` (localhost only) for security.

**Note**: Do not expose the API port publicly without adding authentication. Future versions may include API key or token-based authentication.

## Endpoints

### GET /v1/health

Check editor health and LSP readiness.

**Response**:
```json
{
  "status": "ok",
  "lsp_ready": true,
  "lsp_servers": ["rust-analyzer"]
}
```

**Status Codes**:
- `200 OK`: Editor is healthy
- `503 Service Unavailable`: Editor is not ready

---

### GET /v1/snapshot

Get complete editor state including buffer content, cursor position, mode, registers, and marks.

**Response**:
```json
{
  "buffer": {
    "content": "fn main() {\n    println!(\"Hello\");\n}",
    "file_path": "/path/to/main.rs",
    "modified": false,
    "line_count": 3
  },
  "cursor": {
    "line": 1,
    "column": 4
  },
  "mode": "NORMAL",
  "visual_selection": null,
  "registers": {},
  "marks": {},
  "picker": null
}
```

---

### GET /v1/buffer

Get current buffer content.

**Response**:
```json
{
  "content": "fn main() {\n    println!(\"Hello\");\n}",
  "file_path": "/path/to/main.rs",
  "modified": false,
  "line_count": 3
}
```

---

### PUT /v1/buffer

Replace entire buffer content.

**Request Body**:
```json
{
  "content": "fn main() {\n    println!(\"World\");\n}"
}
```

**Response**:
```json
{
  "success": true
}
```

---

### GET /v1/cursor

Get current cursor position.

**Response**:
```json
{
  "line": 1,
  "column": 4
}
```

---

### GET /v1/mode

Get current editor mode.

**Response**:
```json
{
  "mode": "NORMAL"
}
```

Possible modes: `NORMAL`, `INSERT`, `VISUAL`, `VISUAL_LINE`, `VISUAL_BLOCK`, `COMMAND`

---

### POST /v1/mode

Set editor mode.

**Request Body**:
```json
{
  "mode": "INSERT"
}
```

**Response**:
```json
{
  "success": true,
  "mode": "INSERT"
}
```

---

### POST /v1/keys

Send key sequences to the editor (simulates user input).

**Request Body**:
```json
{
  "keys": "ggdd"
}
```

**Escape Sequences**:
- `\e` - Escape key
- `\c` - Ctrl+C
- `\n` - Enter/newline
- `\\` - Literal backslash

**Examples**:
```json
{"keys": "iHello World\e"}  // Enter insert mode, type, escape
{"keys": "/pattern\n"}        // Search for pattern and confirm
{"keys": "gg=G"}              // Format entire file
```

**Response**:
```json
{
  "success": true,
  "keys_sent": "ggdd"
}
```

---

### POST /v1/command

Execute ex command (`:` commands in Vim).

**Request Body**:
```json
{
  "command": "w"
}
```

**Examples**:
- `{"command": "w"}` - Write file
- `{"command": "q"}` - Quit
- `{"command": "set number"}` - Enable line numbers
- `{"command": "200"}` - Jump to line 200

**Response**:
```json
{
  "success": true
}
```

---

### GET /v1/render

Get ANSI-rendered view of the editor (for terminal display).

**Response**:
```json
{
  "render": "\u001b[0m  1 \u001b[0m\u001b[38;5;203mfn\u001b[0m..."
}
```

---

### GET /v1/lsp/status

Get status of all language servers.

**Response**:
```json
{
  "servers": [
    {
      "name": "rust-analyzer",
      "status": "Running",
      "pid": 12345,
      "pending_requests": 0
    }
  ]
}
```

**Server Status Values**:
- `Starting`: Server is initializing
- `Running`: Server is active and ready
- `Failed`: Server failed to start or crashed
- `Stopped`: Server has been stopped

---

### GET /v1/metrics

Get performance metrics.

**Response**:
```json
{
  "buffer_size_bytes": 1024,
  "buffer_line_count": 42,
  "render_count": 156,
  "last_render_ms": 2.5,
  "uptime_seconds": 3600
}
```

---

### POST /v1/mcp

Model Context Protocol (MCP) JSON-RPC 2.0 endpoint.

See [MCP_INTEGRATION.md](MCP_INTEGRATION.md) for complete MCP documentation.

**Request Body** (JSON-RPC 2.0):
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/list",
  "params": {}
}
```

**Response**:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "tools": [...]
  }
}
```

## Error Responses

All endpoints return structured error responses on failure:

```json
{
  "error": "Description of what went wrong"
}
```

**Common HTTP Status Codes**:
- `200 OK`: Request succeeded
- `400 Bad Request`: Invalid request body or parameters
- `404 Not Found`: Endpoint doesn't exist
- `500 Internal Server Error`: Unexpected error occurred
- `503 Service Unavailable`: Editor is not ready (check `/v1/health`)

## Rate Limiting

Currently, there is no rate limiting. Since the API binds to localhost, it's assumed to be used by trusted local clients.

## Best Practices

### Use Versioned Endpoints

Always use `/v1/` prefix in production code:

```bash
# Good
curl http://127.0.0.1:3000/v1/buffer

# Avoid
curl http://127.0.0.1:3000/buffer
```

### Check Health Before Operations

Before performing complex operations, check if the editor is ready:

```bash
curl http://127.0.0.1:3000/v1/health
```

### Use Atomic Operations When Possible

Instead of multiple `/v1/keys` requests, batch operations:

```bash
# Less efficient - multiple requests
curl -X POST http://127.0.0.1:3000/v1/keys -d '{"keys":"gg"}'
curl -X POST http://127.0.0.1:3000/v1/keys -d '{"keys":"dd"}'

# Better - single request
curl -X POST http://127.0.0.1:3000/v1/keys -d '{"keys":"ggdd"}'
```

### Handle Deprecation Headers

Check response headers for deprecation warnings:

```bash
# Check headers
curl -I http://127.0.0.1:3000/buffer

# Look for:
# X-API-Deprecation: Unversioned API paths are deprecated. Use /v1/* instead.
# Sunset: Wed, 01 Jul 2026 00:00:00 GMT
```

## Design Rationale

### Why Path-Based Versioning?

We chose path-based versioning (`/v1/resource`) over header-based versioning (`Accept: application/vnd.ovim.v1+json`) for several reasons:

1. **Discoverability**: Version is visible in the URL - easier to debug and understand
2. **Simplicity**: No need to remember custom header formats
3. **Cacheability**: Different versions can be cached separately by proxies
4. **Tool Support**: Works naturally with curl, Postman, and all HTTP clients
5. **Industry Standard**: Used by most major APIs (GitHub, Stripe, Twitter, etc.)

### Why Support Legacy Routes?

Backward compatibility is critical for:
- Existing scripts and tools that don't expect breaking changes
- Gradual migration without forcing immediate updates
- Reducing friction for early adopters

The deprecation period (6 months with headers) gives ample time for migration.

### Future Versions

When v2 becomes necessary (for breaking changes), the API will look like:

```
/v1/buffer  - Still supported (deprecated)
/v2/buffer  - New version with breaking changes
```

Both versions will coexist during the transition period.

## Examples

### Get Editor State
```bash
# Get full snapshot
curl http://127.0.0.1:3000/v1/snapshot | jq '.'

# Get just buffer content
curl http://127.0.0.1:3000/v1/buffer | jq '.content'

# Get cursor position
curl http://127.0.0.1:3000/v1/cursor | jq '.'
```

### Edit Buffer
```bash
# Send key sequence
curl -X POST http://127.0.0.1:3000/v1/keys \
  -H "Content-Type: application/json" \
  -d '{"keys":"ggdG"}'  # Delete all lines

# Replace buffer
curl -X PUT http://127.0.0.1:3000/v1/buffer \
  -H "Content-Type: application/json" \
  -d '{"content":"New content"}'
```

### LSP Operations
```bash
# Check LSP status
curl http://127.0.0.1:3000/v1/lsp/status | jq '.'

# Use MCP to get hover info
curl -X POST http://127.0.0.1:3000/v1/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"lsp_hover"}}'
```

## Changelog

### v1 (Current)
- Initial stable API release
- All core editor operations supported
- MCP integration
- Deprecation headers for legacy routes

## See Also

- [MCP_INTEGRATION.md](MCP_INTEGRATION.md) - Complete MCP documentation
- [CLI_SUBCOMMANDS.md](CLI_SUBCOMMANDS.md) - CLI reference (uses this API)
- [AI_WORKFLOWS.md](AI_WORKFLOWS.md) - AI integration patterns
- [ARCHITECTURE.md](ARCHITECTURE.md) - Internal architecture details
