/// MCP (Model Context Protocol) stdio server
///
/// Implements a JSON-RPC 2.0 server that communicates via stdin/stdout.
/// Claude Code spawns this process and sends MCP requests/responses.
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use crate::api::mcp::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use crate::client::OvimClient;
use crate::session::SessionInfo;

/// Main MCP server loop
pub fn run_mcp_server(workspace_dir: PathBuf) -> Result<()> {
    eprintln!("[MCP] Starting ovim MCP server in workspace: {}", workspace_dir.display());

    let stdin = io::stdin();
    let mut reader = io::BufReader::new(stdin.lock());
    let mut stdout = io::stdout();
    let mut buffer = String::new();

    // Track current session context
    let mut current_session: Option<SessionInfo> = None;

    loop {
        buffer.clear();

        // Read one line from stdin
        match reader.read_line(&mut buffer) {
            Ok(0) => {
                // EOF - graceful shutdown
                eprintln!("[MCP] EOF received, shutting down");
                break;
            }
            Ok(_) => {
                let line = buffer.trim();
                if line.is_empty() {
                    continue;
                }

                // Parse JSON-RPC request
                match serde_json::from_str::<JsonRpcRequest>(line) {
                    Ok(request) => {
                        eprintln!("[MCP] Request: {} (id: {:?})", request.method, request.id);

                        // Handle request and get response
                        let response = handle_request(
                            &request,
                            &workspace_dir,
                            &mut current_session,
                        );

                        // Send response if this was a regular request (not notification)
                        if request.id.is_some() {
                            match serde_json::to_string(&response) {
                                Ok(json) => {
                                    if writeln!(stdout, "{}", json).is_err() {
                                        eprintln!("[MCP] Failed to write response");
                                        break;
                                    }
                                    let _ = stdout.flush();
                                }
                                Err(e) => {
                                    eprintln!("[MCP] Failed to serialize response: {}", e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[MCP] Parse error: {}", e);

                        // Send parse error response
                        let error_response = JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: None,
                            result: None,
                            error: Some(JsonRpcError::parse_error()),
                        };

                        if let Ok(json) = serde_json::to_string(&error_response) {
                            let _ = writeln!(stdout, "{}", json);
                            let _ = stdout.flush();
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("[MCP] Read error: {}", e);
                break;
            }
        }
    }

    eprintln!("[MCP] Server shutdown complete");
    Ok(())
}

/// Handle a single MCP JSON-RPC request
fn handle_request(
    request: &JsonRpcRequest,
    workspace_dir: &PathBuf,
    current_session: &mut Option<SessionInfo>,
) -> JsonRpcResponse {
    let (result, error) = match request.method.as_str() {
        "initialize" => match handle_initialize(&request.params) {
            Ok(r) => (Some(r), None),
            Err(e) => (None, Some(e)),
        },
        "tools/list" => match handle_tools_list() {
            Ok(r) => (Some(r), None),
            Err(e) => (None, Some(e)),
        },
        "tools/call" => match handle_tool_call(&request.params, workspace_dir, current_session) {
            Ok(r) => (Some(r), None),
            Err(e) => (None, Some(e)),
        },
        "resources/list" => match handle_resources_list() {
            Ok(r) => (Some(r), None),
            Err(e) => (None, Some(e)),
        },
        "resources/read" => match handle_resources_read(&request.params, current_session) {
            Ok(r) => (Some(r), None),
            Err(e) => (None, Some(e)),
        },
        "prompts/list" => match handle_prompts_list() {
            Ok(r) => (Some(r), None),
            Err(e) => (None, Some(e)),
        },
        _ => (None, Some(JsonRpcError::method_not_found(&request.method))),
    };

    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: request.id.clone(),
        result,
        error,
    }
}

/// Handle initialize request
fn handle_initialize(params: &Value) -> Result<Value, JsonRpcError> {
    let _init_params: crate::api::mcp::InitializeParams = serde_json::from_value(params.clone())
        .map_err(|e| JsonRpcError::invalid_params(&e.to_string()))?;

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

/// Handle tools/list request
fn handle_tools_list() -> Result<Value, JsonRpcError> {
    let tools = crate::api::mcp::get_tools();
    Ok(json!({ "tools": tools }))
}

/// Handle tools/call request
fn handle_tool_call(
    params: &Value,
    workspace_dir: &PathBuf,
    current_session: &mut Option<SessionInfo>,
) -> Result<Value, JsonRpcError> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'name' field"))?;

    let default_args = json!({});
    let arguments = params.get("arguments").unwrap_or(&default_args);

    // Extract optional session parameter
    let preferred_session = arguments.get("session").and_then(|v| v.as_str());

    // Try to use preferred session if specified
    let session = if let Some(session_name) = preferred_session {
        SessionInfo::read(session_name)
            .map_err(|e| JsonRpcError::internal_error(&format!("Session '{}' not found: {}", session_name, e)))?
    } else {
        // Get or discover session (auto-discovery with smart behavior)
        get_or_discover_session(workspace_dir, current_session)
            .map_err(|e| JsonRpcError::internal_error(&e))?
    };

    *current_session = Some(session.clone());
    let client = OvimClient::new(&session);

    match name {
        "send_keys" => {
            let keys = arguments
                .get("keys")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError::invalid_params("Missing 'keys' field"))?;

            client
                .send_keys(keys)
                .map_err(|e| JsonRpcError::internal_error(&e.to_string()))?;

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": format!("Sent keys: {}", keys)
                    }
                ]
            }))
        }

        "get_buffer" => {
            let buffer = client
                .get_buffer()
                .map_err(|e| JsonRpcError::internal_error(&e.to_string()))?;

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": buffer.content
                    }
                ]
            }))
        }

        "set_buffer" => {
            let content = arguments
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError::invalid_params("Missing 'content' field"))?;

            client
                .set_buffer(content)
                .map_err(|e| JsonRpcError::internal_error(&e.to_string()))?;

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": "Buffer updated"
                    }
                ]
            }))
        }

        "get_cursor" => {
            let snapshot = client
                .get_snapshot()
                .map_err(|e| JsonRpcError::internal_error(&e.to_string()))?;

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": format!("Line: {}, Column: {}", snapshot.cursor.line, snapshot.cursor.column)
                    }
                ]
            }))
        }

        "execute_command" => {
            let command = arguments
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError::invalid_params("Missing 'command' field"))?;

            let result = client
                .execute_command(command)
                .map_err(|e| JsonRpcError::internal_error(&e.to_string()))?;

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": result
                    }
                ]
            }))
        }

        "lsp_hover" => {
            let snapshot = client
                .get_snapshot()
                .map_err(|e| JsonRpcError::internal_error(&e.to_string()))?;

            // Get LSP status to show hover capability
            let lsp_status = client
                .get_lsp_status()
                .map_err(|e| JsonRpcError::internal_error(&e.to_string()))?;

            if lsp_status.servers.is_empty() {
                return Ok(json!({
                    "content": [
                        {
                            "type": "text",
                            "text": "No LSP server running"
                        }
                    ]
                }));
            }

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": format!("LSP servers available: {}",
                            lsp_status.servers.iter()
                                .map(|s| s.language.clone())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                ]
            }))
        }

        "lsp_goto_definition" => {
            let lsp_status = client
                .get_lsp_status()
                .map_err(|e| JsonRpcError::internal_error(&e.to_string()))?;

            if lsp_status.servers.is_empty() {
                return Ok(json!({
                    "content": [
                        {
                            "type": "text",
                            "text": "No LSP server running"
                        }
                    ]
                }));
            }

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": "LSP goto definition not yet implemented via MCP"
                    }
                ]
            }))
        }

        "get_snapshot" => {
            let snapshot = client
                .get_snapshot()
                .map_err(|e| JsonRpcError::internal_error(&e.to_string()))?;

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": serde_json::to_string_pretty(&snapshot)
                            .unwrap_or_else(|_| "Failed to serialize snapshot".to_string())
                    }
                ]
            }))
        }

        "get_health" => {
            let health = client
                .get_health()
                .map_err(|e| JsonRpcError::internal_error(&e.to_string()))?;

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": format!("Status: {}, Ready: {}, Uptime: {}s",
                            health.status, health.ready, health.uptime_seconds)
                    }
                ]
            }))
        }

        "list_sessions" => {
            let sessions = SessionInfo::list_all()
                .map_err(|e| JsonRpcError::internal_error(&e.to_string()))?;

            let session_list = sessions
                .iter()
                .map(|s| format!("{} (PID: {}, port: {})", s.session_name, s.pid, s.port))
                .collect::<Vec<_>>()
                .join("\n");

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": if session_list.is_empty() {
                            "No active sessions".to_string()
                        } else {
                            session_list
                        }
                    }
                ]
            }))
        }

        "get_lsp_status" => {
            let lsp_status = client
                .get_lsp_status()
                .map_err(|e| JsonRpcError::internal_error(&e.to_string()))?;

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": serde_json::to_string_pretty(&lsp_status)
                            .unwrap_or_else(|_| "Failed to serialize LSP status".to_string())
                    }
                ]
            }))
        }

        "set_mode" => {
            let mode = arguments
                .get("mode")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError::invalid_params("Missing 'mode' field"))?;

            client
                .set_mode(mode)
                .map_err(|e| JsonRpcError::internal_error(&e.to_string()))?;

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": format!("Mode set to {}", mode.to_uppercase())
                    }
                ]
            }))
        }

        _ => Err(JsonRpcError::method_not_found(name)),
    }
}

/// Handle resources/list request
fn handle_resources_list() -> Result<Value, JsonRpcError> {
    let resources = crate::api::mcp::get_resources(None);
    Ok(json!({ "resources": resources }))
}

/// Handle resources/read request
fn handle_resources_read(
    params: &Value,
    current_session: &mut Option<SessionInfo>,
) -> Result<Value, JsonRpcError> {
    let uri = params
        .get("uri")
        .and_then(|v| v.as_str())
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'uri' field"))?;

    if let Some(session) = current_session {
        let client = OvimClient::new(session);

        match uri {
            "ovim://buffer" => {
                let buffer = client
                    .get_buffer()
                    .map_err(|e| JsonRpcError::internal_error(&e.to_string()))?;

                return Ok(json!({
                    "contents": [
                        {
                            "uri": uri,
                            "mimeType": "text/plain",
                            "text": buffer.content
                        }
                    ]
                }));
            }
            "ovim://snapshot" => {
                let snapshot = client
                    .get_snapshot()
                    .map_err(|e| JsonRpcError::internal_error(&e.to_string()))?;

                return Ok(json!({
                    "contents": [
                        {
                            "uri": uri,
                            "mimeType": "application/json",
                            "text": serde_json::to_string_pretty(&snapshot)
                                .unwrap_or_else(|_| "{}".to_string())
                        }
                    ]
                }));
            }
            "ovim://lsp/status" => {
                let lsp_status = client
                    .get_lsp_status()
                    .map_err(|e| JsonRpcError::internal_error(&e.to_string()))?;

                return Ok(json!({
                    "contents": [
                        {
                            "uri": uri,
                            "mimeType": "application/json",
                            "text": serde_json::to_string_pretty(&lsp_status)
                                .unwrap_or_else(|_| "{}".to_string())
                        }
                    ]
                }));
            }
            _ => {}
        }
    }

    Err(JsonRpcError::internal_error("Resource not available"))
}

/// Handle prompts/list request (empty for now)
fn handle_prompts_list() -> Result<Value, JsonRpcError> {
    Ok(json!({ "prompts": [] }))
}

/// Get or discover session for tool calls
fn get_or_discover_session(
    workspace_dir: &PathBuf,
    current_session: &Option<SessionInfo>,
) -> Result<SessionInfo, String> {
    // If explicit session preference provided, try to find it
    // (Note: Currently not used, but structure allows for it in future)

    // If we have a current session from previous call, use it
    if let Some(session) = current_session {
        return Ok(session.clone());
    }

    // Try to discover sessions
    let all_sessions = SessionInfo::list_all().unwrap_or_default();

    match all_sessions.len() {
        0 => {
            Err("No active ovim sessions found. Start one with: ovim --headless --session default".to_string())
        }
        1 => {
            // Single session found, use it
            Ok(all_sessions[0].clone())
        }
        _ => {
            // Multiple sessions found - require explicit session specification
            let session_list = all_sessions
                .iter()
                .map(|s| format!("  - {}", s.session_name))
                .collect::<Vec<_>>()
                .join("\n");
            Err(format!(
                "Multiple ovim sessions found. Please specify which session to use by providing a 'session' parameter in your tool call.\n\nAvailable sessions:\n{}",
                session_list
            ))
        }
    }
}
