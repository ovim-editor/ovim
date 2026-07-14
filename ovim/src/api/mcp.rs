/// MCP (Model Context Protocol) JSON-RPC 2.0 implementation
///
/// This module implements the Model Context Protocol specification
/// for exposing ovim's capabilities as an MCP server.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// JSON-RPC 2.0 Request
#[derive(Debug, Deserialize, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// JSON-RPC 2.0 Response
#[derive(Debug, Deserialize, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 Error
#[derive(Debug, Deserialize, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Standard JSON-RPC error codes
impl JsonRpcError {
    pub fn parse_error() -> Self {
        Self {
            code: -32700,
            message: "Parse error".to_string(),
            data: None,
        }
    }

    pub fn invalid_request() -> Self {
        Self {
            code: -32600,
            message: "Invalid Request".to_string(),
            data: None,
        }
    }

    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: "Method not found".to_string(),
            data: Some(json!({ "method": method })),
        }
    }

    pub fn invalid_params(message: &str) -> Self {
        Self {
            code: -32602,
            message: "Invalid params".to_string(),
            data: Some(json!({ "reason": message })),
        }
    }

    pub fn internal_error(message: &str) -> Self {
        Self {
            code: -32603,
            message: "Internal error".to_string(),
            data: Some(json!({ "reason": message })),
        }
    }
}

/// MCP Initialize parameters
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    pub client_info: ClientInfo,
}

/// Client capabilities
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    #[serde(default)]
    pub roots: Option<Value>,
    #[serde(default)]
    pub sampling: Option<Value>,
}

/// Client information
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

/// MCP Tool definition
#[derive(Debug, Serialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// MCP Resource definition
#[derive(Debug, Serialize)]
pub struct Resource {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "mimeType")]
    pub mime_type: Option<String>,
}

/// MCP Resource content
#[derive(Debug, Serialize)]
pub struct ResourceContent {
    pub uri: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

/// Handle MCP initialize request
pub fn handle_initialize(params: Value) -> Result<Value, JsonRpcError> {
    let _init_params: InitializeParams =
        serde_json::from_value(params).map_err(|e| JsonRpcError::invalid_params(&e.to_string()))?;

    // Return server capabilities
    Ok(json!({
        "protocolVersion": "2025-03-26",
        "capabilities": {
            "tools": {},
            "resources": {},
        },
        "serverInfo": {
            "name": "ovim",
            "version": env!("CARGO_PKG_VERSION")
        }
    }))
}

/// Get all available tools
pub fn get_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "send_keys".to_string(),
            description: "Send key sequences to the editor (Vim keybindings)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "keys": {
                        "type": "string",
                        "description": "Vim key sequence (e.g., 'gg' for top, 'dd' for delete line, 'iHello<Esc>' for insert)"
                    },
                    "session": {
                        "type": "string",
                        "description": "Optional: specify which session to use. If omitted, uses current session or auto-discovers if only one exists"
                    }
                },
                "required": ["keys"]
            }),
        },
        Tool {
            name: "paste".to_string(),
            description: "Paste literal text as one bracketed-paste event".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Literal text to paste, including multiline content"
                    },
                    "session": {
                        "type": "string",
                        "description": "Optional session name"
                    }
                },
                "required": ["text"]
            }),
        },
        Tool {
            name: "resize".to_string(),
            description: "Resize the editor's logical viewport".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "width": { "type": "integer", "minimum": 10, "maximum": 500 },
                    "height": { "type": "integer", "minimum": 3, "maximum": 200 },
                    "session": {
                        "type": "string",
                        "description": "Optional session name"
                    }
                },
                "required": ["width", "height"]
            }),
        },
        Tool {
            name: "get_buffer".to_string(),
            description: "Get the current buffer content".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session": {
                        "type": "string",
                        "description": "Optional: specify which session to use"
                    }
                }
            }),
        },
        Tool {
            name: "set_buffer".to_string(),
            description: "Replace the entire buffer content".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The new content for the buffer"
                    },
                    "session": {
                        "type": "string",
                        "description": "Optional: specify which session to use"
                    }
                },
                "required": ["content"]
            }),
        },
        Tool {
            name: "get_cursor".to_string(),
            description: "Get the current cursor position".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session": {
                        "type": "string",
                        "description": "Optional: specify which session to use"
                    }
                }
            }),
        },
        Tool {
            name: "execute_command".to_string(),
            description: "Execute an ex command (e.g., ':w' to save, ':q' to quit)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Ex command without the leading colon"
                    },
                    "session": {
                        "type": "string",
                        "description": "Optional: specify which session to use"
                    }
                },
                "required": ["command"]
            }),
        },
        Tool {
            name: "lsp_hover".to_string(),
            description: "Get LSP hover information at cursor position".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session": {
                        "type": "string",
                        "description": "Optional: specify which session to use"
                    }
                }
            }),
        },
        Tool {
            name: "lsp_goto_definition".to_string(),
            description: "Jump to definition using LSP".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session": {
                        "type": "string",
                        "description": "Optional: specify which session to use"
                    }
                }
            }),
        },
        Tool {
            name: "get_snapshot".to_string(),
            description: "Get complete editor state (buffer, cursor, mode, registers, marks)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session": {
                        "type": "string",
                        "description": "Optional: specify which session to use"
                    }
                }
            }),
        },
        Tool {
            name: "get_health".to_string(),
            description: "Get session health and status information".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session": {
                        "type": "string",
                        "description": "Optional: specify which session to use"
                    }
                }
            }),
        },
        Tool {
            name: "list_sessions".to_string(),
            description: "List all active ovim sessions".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        Tool {
            name: "get_lsp_status".to_string(),
            description: "Get LSP server status and information".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session": {
                        "type": "string",
                        "description": "Optional: specify which session to use"
                    }
                }
            }),
        },
        Tool {
            name: "set_mode".to_string(),
            description: "Set the editor mode (NORMAL, INSERT, VISUAL, VISUAL_LINE, VISUAL_BLOCK, COMMAND, SEARCH, PICKER)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "mode": {
                        "type": "string",
                        "description": "Target mode: NORMAL, INSERT, VISUAL, VISUAL_LINE, VISUAL_BLOCK, COMMAND, SEARCH, PICKER"
                    },
                    "session": {
                        "type": "string",
                        "description": "Optional: specify which session to use"
                    }
                },
                "required": ["mode"]
            }),
        },
        Tool {
            name: "get_outline".to_string(),
            description: "Get a structural outline (table of contents) of the current document with symbol names, kinds, and line ranges".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session": {
                        "type": "string",
                        "description": "Optional: specify which session to use"
                    }
                }
            }),
        },
        Tool {
            name: "search_symbol".to_string(),
            description: "Search for symbols across the workspace by name (fuzzy). Returns up to 50 results with file, line, and kind".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Symbol name or partial name to search for"
                    },
                    "session": {
                        "type": "string",
                        "description": "Optional: specify which session to use"
                    }
                },
                "required": ["query"]
            }),
        },
        Tool {
            name: "get_trace".to_string(),
            description: "Get call hierarchy for the symbol at cursor: who calls it (incoming) and what it calls (outgoing)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session": {
                        "type": "string",
                        "description": "Optional: specify which session to use"
                    }
                }
            }),
        },
        Tool {
            name: "get_diagnostics".to_string(),
            description: "Get LSP diagnostics (errors, warnings) for the current file".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session": {
                        "type": "string",
                        "description": "Optional: specify which session to use"
                    }
                }
            }),
        },
        Tool {
            name: "get_context_window".to_string(),
            description: "Get a 21-line context window around cursor (10 above, current, 10 below) with line numbers and cursor position".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session": {
                        "type": "string",
                        "description": "Optional: specify which session to use"
                    }
                }
            }),
        },
    ]
}

/// Handle tools/list request
pub fn handle_tools_list() -> Result<Value, JsonRpcError> {
    let tools = get_tools();
    Ok(json!({ "tools": tools }))
}

/// Get all available resources
pub fn get_resources(file_path: Option<&str>) -> Vec<Resource> {
    let mut resources = vec![
        Resource {
            uri: "ovim://context-window".to_string(),
            name: "Context Window".to_string(),
            description: Some(
                "21-line context around cursor (10 above, current, 10 below) - AI-optimized"
                    .to_string(),
            ),
            mime_type: Some("text/plain".to_string()),
        },
        Resource {
            uri: "ovim://buffer".to_string(),
            name: "Current Buffer".to_string(),
            description: Some("The current editor buffer content".to_string()),
            mime_type: Some("text/plain".to_string()),
        },
        Resource {
            uri: "ovim://snapshot".to_string(),
            name: "Editor Snapshot".to_string(),
            description: Some(
                "Complete editor state including buffer, cursor, mode, registers".to_string(),
            ),
            mime_type: Some("application/json".to_string()),
        },
        Resource {
            uri: "ovim://lsp/status".to_string(),
            name: "LSP Status".to_string(),
            description: Some("Language server status information".to_string()),
            mime_type: Some("application/json".to_string()),
        },
    ];

    // Add current file as a resource if available
    if let Some(path) = file_path {
        resources.push(Resource {
            uri: format!("file://{}", path),
            name: "Current File".to_string(),
            description: Some(format!("The file being edited: {}", path)),
            mime_type: Some("text/plain".to_string()),
        });
    }

    resources
}

/// Handle resources/list request
pub fn handle_resources_list(file_path: Option<&str>) -> Result<Value, JsonRpcError> {
    let resources = get_resources(file_path);
    Ok(json!({ "resources": resources }))
}

/// Handle prompts/list request (optional, return empty for now)
pub fn handle_prompts_list() -> Result<Value, JsonRpcError> {
    Ok(json!({ "prompts": [] }))
}

/// Create tool call result
pub fn tool_result(content: Vec<Value>) -> Value {
    json!({
        "content": content
    })
}

/// Create text content for tool result
pub fn text_content(text: &str) -> Value {
    json!({
        "type": "text",
        "text": text
    })
}

/// Create error content for tool result
pub fn error_content(error: &str) -> Value {
    json!({
        "type": "text",
        "text": format!("Error: {}", error),
        "isError": true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_rpc_request_parsing() {
        let json = r#"{
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }"#;

        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "initialize");
    }

    #[test]
    fn test_tools_list() {
        let tools = get_tools();
        assert!(!tools.is_empty());
        assert!(tools.iter().any(|t| t.name == "send_keys"));
        assert!(tools.iter().any(|t| t.name == "get_buffer"));
    }

    #[test]
    fn test_resources_list() {
        let resources = get_resources(Some("/path/to/file.rs"));
        assert!(!resources.is_empty());
        assert!(resources.iter().any(|r| r.uri == "ovim://buffer"));
        assert!(resources.iter().any(|r| r.uri.starts_with("file://")));
    }
}
