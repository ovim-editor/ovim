# TypeScript Language Server Manual Test Report

## Test Environment
- **Language Server**: typescript-language-server v5.0.1
- **TypeScript Version**: 5.9.3 (bundled)
- **Location**: `~/.local/bin/typescript-language-server`
- **Test Date**: 2025-10-26

## Test File
**Path**: `/tmp/test_ts_lsp.ts`
```typescript
const greeting: string = "Hello";
function add(a: number, b: number): number {
    return a + b;
}
```

## Exact JSON-RPC Messages Required

### 1. Initialize Request
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "processId": null,
    "rootUri": "file:///tmp",
    "capabilities": {
      "textDocument": {
        "hover": {
          "contentFormat": ["markdown", "plaintext"]
        }
      }
    },
    "initializationOptions": {}
  }
}
```

**Response**: Initialize response with server capabilities (see below)

### 2. Initialized Notification
```json
{
  "jsonrpc": "2.0",
  "method": "initialized",
  "params": {}
}
```

**Response**: No response (notification only)

### 3. didOpen Notification
```json
{
  "jsonrpc": "2.0",
  "method": "textDocument/didOpen",
  "params": {
    "textDocument": {
      "uri": "file:///tmp/test_ts_lsp.ts",
      "languageId": "typescript",
      "version": 1,
      "text": "const greeting: string = \"Hello\";\nfunction add(a: number, b: number): number {\n    return a + b;\n}\n"
    }
  }
}
```

**Response**: No response (notification only)

### 3b. didChange Notification (optional - for content updates)
```json
{
  "jsonrpc": "2.0",
  "method": "textDocument/didChange",
  "params": {
    "textDocument": {
      "uri": "file:///tmp/test_ts_lsp.ts",
      "version": 2
    },
    "contentChanges": [
      {
        "text": "const greeting: string = \"Hello\";\nconst name: string = \"World\";\n"
      }
    ]
  }
}
```

**Response**: No response (notification only)

**Note**: Hover requests work immediately after didChange - no wait needed!

### 4. Hover Request (on "greeting" variable)
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "textDocument/hover",
  "params": {
    "textDocument": {
      "uri": "file:///tmp/test_ts_lsp.ts"
    },
    "position": {
      "line": 0,
      "character": 6
    }
  }
}
```

**Response**:
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "contents": {
      "kind": "markdown",
      "value": "\n```typescript\nconst greeting: string\n```\n"
    },
    "range": {
      "start": {
        "line": 0,
        "character": 6
      },
      "end": {
        "line": 0,
        "character": 14
      }
    }
  }
}
```

## Server Capabilities (from Initialize Response)

The server reports the following capabilities:

```json
{
  "textDocumentSync": 2,
  "completionProvider": {
    "triggerCharacters": [".", "\"", "'", "/", "@", "<"],
    "resolveProvider": true
  },
  "codeActionProvider": true,
  "codeLensProvider": {
    "resolveProvider": true
  },
  "definitionProvider": true,
  "documentFormattingProvider": true,
  "documentRangeFormattingProvider": true,
  "documentHighlightProvider": true,
  "documentSymbolProvider": true,
  "executeCommandProvider": {
    "commands": [
      "_typescript.applyRefactoring",
      "_typescript.configurePlugin",
      "_typescript.organizeImports",
      "_typescript.applyRenameFile",
      "_typescript.goToSourceDefinition",
      "typescript.tsserverRequest"
    ]
  },
  "hoverProvider": true,
  "inlayHintProvider": true,
  "linkedEditingRangeProvider": false,
  "renameProvider": true,
  "referencesProvider": true,
  "selectionRangeProvider": true,
  "signatureHelpProvider": {
    "triggerCharacters": ["(", ",", "<"],
    "retriggerCharacters": [")"]
  },
  "workspaceSymbolProvider": true,
  "implementationProvider": true,
  "typeDefinitionProvider": true,
  "foldingRangeProvider": true,
  "semanticTokensProvider": {
    "documentSelector": null,
    "legend": {
      "tokenTypes": [
        "class", "enum", "interface", "namespace", "typeParameter",
        "type", "parameter", "variable", "enumMember", "property",
        "function", "member"
      ],
      "tokenModifiers": [
        "declaration", "static", "async", "readonly",
        "defaultLibrary", "local"
      ]
    },
    "full": true,
    "range": true
  }
}
```

## Additional Test Results

### Hover on Function Declaration
**Position**: Line 1, character 9 (on "add")
```json
{
  "result": {
    "contents": {
      "kind": "markdown",
      "value": "\n```typescript\nfunction add(a: number, b: number): number\n```\n"
    },
    "range": {
      "start": {"line": 1, "character": 9},
      "end": {"line": 1, "character": 12}
    }
  }
}
```

### Hover on Function Parameter
**Position**: Line 1, character 13 (on parameter "a")
```json
{
  "result": {
    "contents": {
      "kind": "markdown",
      "value": "\n```typescript\n(parameter) a: number\n```\n"
    },
    "range": {
      "start": {"line": 1, "character": 13},
      "end": {"line": 1, "character": 14}
    }
  }
}
```

### Hover on Whitespace/Non-Identifier Positions
**Positions tested**:
- Line 0, character 0 (before "const")
- Line 0, character 18 (on colon)
- Line 0, character 30 (in string literal)
- Line 2, character 4 (on "return" keyword)

**Result**: All return `"result": null` (no error, just null result)

## Timing and Sequencing Requirements

### Wait Times
- **After initialize**: 100ms is sufficient
- **After initialized**: 100ms is sufficient
- **After didOpen**: **NO WAIT REQUIRED** - hover requests work immediately
- **After hover request**: ~100-500ms for response

### Critical Findings
1. **No wait needed after didOpen**: The TypeScript language server processes hover requests immediately, even if sent right after didOpen
2. **No wait needed after didChange**: Hover requests work immediately after didChange notifications - the server processes changes synchronously
3. **null vs error**: When hover information is not available, the server returns `null` in the result field, NOT an error response
4. **Markdown format**: All hover responses use markdown format with triple-backtick code blocks
5. **Range information**: Every successful hover includes a range indicating the exact span of the identifier

## Message Format Details

### Content-Length Header
All messages must be prefixed with:
```
Content-Length: <byte_count>\r\n\r\n<json_message>
```

Example:
```
Content-Length: 123\r\n\r\n{"jsonrpc":"2.0","id":1,...}
```

### Line Endings
- Headers use `\r\n` (CRLF)
- Double `\r\n` separates header from body
- JSON content can use `\n` for embedded newlines

## Server Notifications

The server may send unsolicited notifications:

### window/logMessage
```json
{
  "jsonrpc": "2.0",
  "method": "window/logMessage",
  "params": {
    "type": 3,
    "message": "Using Typescript version (bundled) 5.9.3 from path \"...\""
  }
}
```

### $/typescriptVersion
```json
{
  "jsonrpc": "2.0",
  "method": "$/typescriptVersion",
  "params": {
    "version": "5.9.3",
    "source": "bundled"
  }
}
```

## Initialization Options

The test used empty `initializationOptions: {}`, which works fine. The server doesn't require any specific initialization options for basic hover functionality.

## Conclusions

1. **Immediate hover works**: No need to wait after didOpen OR didChange before sending hover requests
2. **Synchronous processing**: The TypeScript language server processes document changes synchronously, making hover immediately available
3. **Simple initialization**: Minimal capabilities in initialize request are sufficient
4. **Predictable error handling**: No hover info = null result, not an error
5. **Rich hover content**: Server provides detailed markdown-formatted type information
6. **No special configuration needed**: Works out of the box with default settings

## Recommendations for ovim

1. **Don't add artificial delays**: No need to wait after didOpen or didChange for TypeScript files - the server handles requests immediately
2. **No debouncing needed for hover**: Unlike rust-analyzer, typescript-language-server doesn't need a debounce delay before hover requests
3. **Handle null results gracefully**: Show "No information available" when hover result is null
4. **Parse markdown responses**: All hover content is markdown-formatted with code blocks
5. **Use range information**: Highlight the relevant identifier using the returned range
6. **Trust synchronous behavior**: The server processes changes immediately, making hover data instantly available
