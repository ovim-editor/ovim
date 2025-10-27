# Rust-Analyzer Manual Testing Report

## Overview

This document provides a detailed report on manually testing rust-analyzer's LSP protocol implementation, including exact JSON-RPC messages, timing requirements, and response formats.

## Test Environment

- **rust-analyzer version**: 1.90.0 (1159e78 2025-09-14)
- **Workspace**: `/workspace` (ovim project)
- **Test file**: `/workspace/src/buffer/mod.rs`
- **Platform**: Linux (aarch64-unknown-linux-gnu)

## Key Findings

### 1. Protocol Requirements

#### Message Format
- **Headers**: Must use `\r\n` (CRLF) line endings, not just `\n`
- **Content-Length**: Required before each message
- **Format**: `Content-Length: {bytes}\r\n\r\n{json_payload}`

#### Example Message Send Function
```bash
send_message() {
    local message="$1"
    local length=${#message}
    printf "Content-Length: %d\r\n\r\n%s" "$length" "$message"
}
```

### 2. Initialization Sequence

#### Step 1: Initialize Request

**Request** (id: 1):
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "processId": null,
    "rootUri": "file:///workspace",
    "workspaceFolders": [{
      "uri": "file:///workspace",
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
}
```

**Response**:
- Returns server capabilities including:
  - `positionEncoding: "utf-16"` ⚠️ **CRITICAL**: Positions use UTF-16 code units, not UTF-8 bytes or characters
  - `textDocumentSync.change: 2` (Incremental sync)
  - `hoverProvider: true`
  - `definitionProvider: true`
  - Many other capabilities

**Timing**: Response arrives within ~500ms

#### Step 2: Initialized Notification

**Notification** (no response expected):
```json
{
  "jsonrpc": "2.0",
  "method": "initialized",
  "params": {}
}
```

**Timing**: Send immediately after receiving initialize response

### 3. Document Opening

#### textDocument/didOpen Notification

**Request**:
```json
{
  "jsonrpc": "2.0",
  "method": "textDocument/didOpen",
  "params": {
    "textDocument": {
      "uri": "file:///workspace/src/buffer/mod.rs",
      "languageId": "rust",
      "version": 1,
      "text": "<full file content>"
    }
  }
}
```

**Important Notes**:
- Must send the **full file content**, not partial
- `version` starts at 1 and increments with each change
- Server responds with `textDocument/publishDiagnostics` notification asynchronously

**Timing**:
- Diagnostics arrive within 100-500ms
- Full workspace indexing takes 10-15 seconds

### 4. Hover Requests

#### Critical Timing Discovery

**Problem**: Hover requests return `null` if sent too early
**Solution**: Wait 10-15 seconds after `didOpen` for workspace indexing to complete

#### Successful Hover Examples

##### Example 1: Hover on `use` keyword

**Request** (id: 10):
```json
{
  "jsonrpc": "2.0",
  "id": 10,
  "method": "textDocument/hover",
  "params": {
    "textDocument": {
      "uri": "file:///workspace/src/buffer/mod.rs"
    },
    "position": {
      "line": 8,
      "character": 18
    }
  }
}
```

**Note**: Position was clamped by rust-analyzer:
```
ERROR Position LineCol { line: 8, col: 18 } column exceeds line length 17, clamping it
```

**Response**:
```json
{
  "jsonrpc": "2.0",
  "id": 10,
  "result": {
    "contents": {
      "kind": "markdown",
      "value": "\n```rust\nuse\n```\n\n---\n\nImport or rename items from other crates or modules..."
    },
    "range": {
      "start": {"line": 9, "character": 0},
      "end": {"line": 9, "character": 3}
    }
  }
}
```

##### Example 2: Hover on `Buffer` struct

**Request** (id: 11):
```json
{
  "jsonrpc": "2.0",
  "id": 11,
  "method": "textDocument/hover",
  "params": {
    "textDocument": {
      "uri": "file:///workspace/src/buffer/mod.rs"
    },
    "position": {
      "line": 57,
      "character": 15
    }
  }
}
```

**Response**:
```json
{
  "jsonrpc": "2.0",
  "id": 11,
  "result": {
    "contents": {
      "kind": "markdown",
      "value": "\n```rust\novim::buffer\n```\n\n```rust\npub struct Buffer {\n    rope: Rope,\n    cursor: Cursor,\n    modified: bool,\n    file_path: Option<String>,\n    syntax: Option<SyntaxHighlighter>,\n    /* … */\n}\n```\n\n---\n\nsize = 552 (0x228), align = 0x8, needs Drop\n\n---\n\nRepresents a text buffer using a Rope data structure for efficient editing"
    },
    "range": {
      "start": {"line": 57, "character": 11},
      "end": {"line": 57, "character": 17}
    }
  }
}
```

**Hover Response Structure**:
- `contents.kind`: "markdown" or "plaintext"
- `contents.value`: The hover text (Markdown formatted)
- `range`: Optional range highlighting the symbol

**Timing**: Responses arrive within 100-500ms after indexing is complete

### 5. Shutdown Sequence

#### Shutdown Request

**Request** (id: 99):
```json
{
  "jsonrpc": "2.0",
  "id": 99,
  "method": "shutdown",
  "params": null
}
```

⚠️ **Important**: Use `null`, not `{}` for params!

**Response**:
```json
{
  "jsonrpc": "2.0",
  "id": 99,
  "result": null
}
```

#### Exit Notification

**Notification** (no response):
```json
{
  "jsonrpc": "2.0",
  "method": "exit"
}
```

## Critical Discoveries

### 1. Workspace Root Matters

✅ **With workspace root**: Hover works after 10-15 seconds
❌ **Without workspace root**: Hover returns `null` indefinitely

**Implication**: rust-analyzer needs:
- Valid `rootUri` pointing to Cargo project root
- `Cargo.toml` present in workspace root
- Time to index dependencies and build metadata

### 2. Position Encoding: UTF-16

The LSP spec allows servers to specify position encoding. Rust-analyzer uses **UTF-16 code units**.

**Impact**:
- For ASCII text: 1 character = 1 UTF-16 code unit ✅
- For emoji/non-BMP: 1 character may = 2 UTF-16 code units ⚠️

**Example**:
```rust
// Line: "let x = 😀;" (1-indexed in editor)
// Position of 'x' in UTF-8: byte 4, char 1
// Position of 'x' in UTF-16: code unit 4
// Position of ';' in UTF-8: byte 10, char 5
// Position of ';' in UTF-16: code unit 6 (emoji takes 2 code units)
```

### 3. Timing Requirements

| Operation | Minimum Wait | Recommended Wait | Notes |
|-----------|--------------|------------------|-------|
| After `initialize` | 500ms | 1-2s | Server startup |
| After `initialized` | 100ms | 1s | Background tasks start |
| After `didOpen` | 10s | 15s | **Critical for hover** |
| After `hover` request | 100ms | 2-3s | Response processing |

### 4. Null Responses

Hover returns `null` when:
1. **Workspace not indexed yet** (most common)
2. Position is outside valid text range
3. No symbol at cursor position
4. File not part of workspace (missing from Cargo.toml)

### 5. Initialization Options

```json
"initializationOptions": {
  "checkOnSave": {
    "enable": false  // Disable cargo check on save for faster testing
  },
  "cargo": {
    "loadOutDirsFromCheck": true  // Load build metadata
  }
}
```

## Testing Scripts

All test scripts are located in `/workspace`:
- `test_rust_analyzer_final.sh` - Full test with multiple hovers
- `test_ra_workspace.sh` - Workspace-aware test (recommended)
- `parse_lsp_output.py` - Python script to parse LSP responses

## Recommendations for ovim

### 1. Initial Document Load
```
1. Send initialize with rootUri = workspace root
2. Wait for initialize response
3. Send initialized notification
4. Wait 1 second
5. Send didOpen for file
6. Wait 10-15 seconds for initial indexing
7. Mark LSP as "ready" for hover/goto
```

### 2. Position Conversion

Implement UTF-16 code unit conversion:
```rust
fn char_to_utf16_offset(line: &str, char_offset: usize) -> usize {
    line.chars()
        .take(char_offset)
        .map(|c| c.len_utf16())
        .sum()
}

fn utf16_to_char_offset(line: &str, utf16_offset: usize) -> usize {
    let mut utf16_count = 0;
    for (char_idx, ch) in line.chars().enumerate() {
        if utf16_count >= utf16_offset {
            return char_idx;
        }
        utf16_count += ch.len_utf16();
    }
    line.chars().count()
}
```

### 3. Hover Request Pattern

```rust
// Before hover request, ensure:
// 1. LSP is initialized
// 2. Document is opened
// 3. Sufficient time has passed since didOpen (check timestamp)

// If recently opened (< 10s ago), show message:
// "LSP still indexing workspace..."

// Otherwise, send hover request with UTF-16 positions
```

### 4. Debugging Checklist

When hover returns null:
- [ ] Is workspace root set correctly?
- [ ] Does Cargo.toml exist in workspace root?
- [ ] Has 10+ seconds passed since didOpen?
- [ ] Is the file part of the Cargo workspace?
- [ ] Is the position converted to UTF-16?
- [ ] Is the position within line bounds?

## Example Full Session

See `/tmp/ra_workspace_output.log` for complete LSP message log from a successful session.

## References

- LSP Specification: https://microsoft.github.io/language-server-protocol/
- rust-analyzer User Manual: https://rust-analyzer.github.io/manual.html
- Position Encoding: https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#position
