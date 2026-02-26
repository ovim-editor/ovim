/// MCP JSON-RPC request handler
use super::mcp::{self, JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use super::state::{ApiRequest, ApiState};
use axum::{
    extract::State,
    response::{IntoResponse, Json, Response},
    Json as JsonExtractor,
};
use serde_json::{json, Value};
use tokio::sync::oneshot;

/// Handler for POST /mcp
/// Processes MCP JSON-RPC 2.0 requests
pub async fn handle_mcp(
    State(state): State<ApiState>,
    JsonExtractor(request): JsonExtractor<JsonRpcRequest>,
) -> Response {
    // Validate JSON-RPC version
    if request.jsonrpc != "2.0" {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id.clone(),
            result: None,
            error: Some(JsonRpcError::invalid_request()),
        };
        return Json(response).into_response();
    }

    // Process the request based on method
    let result = match request.method.as_str() {
        "initialize" => mcp::handle_initialize(request.params),
        "tools/list" => mcp::handle_tools_list(),
        "tools/call" => handle_tool_call(state, request.params).await,
        "resources/list" => {
            // Get current file path from editor
            let file_path = get_current_file_path(&state).await;
            mcp::handle_resources_list(file_path.as_deref())
        }
        "resources/read" => handle_resource_read(state, request.params).await,
        "prompts/list" => mcp::handle_prompts_list(),
        _ => Err(JsonRpcError::method_not_found(&request.method)),
    };

    // Build response
    let response = match result {
        Ok(result_value) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id,
            result: Some(result_value),
            error: None,
        },
        Err(error) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id,
            result: None,
            error: Some(error),
        },
    };

    Json(response).into_response()
}

/// Handle tools/call method
async fn handle_tool_call(state: ApiState, params: Value) -> Result<Value, JsonRpcError> {
    let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'name' field"))?;

    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

    match tool_name {
        "send_keys" => {
            let keys = arguments
                .get("keys")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError::invalid_params("Missing 'keys' argument"))?;

            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::SendKeys(keys.to_string(), tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::SendKeysResult(result) = response {
                        // Return the context window as the response
                        let json_str = serde_json::to_string_pretty(&result.context)
                            .unwrap_or_else(|_| "{}".to_string());
                        Ok(mcp::tool_result(vec![mcp::text_content(&json_str)]))
                    } else if let super::state::ApiResponse::Error(err) = response {
                        Ok(mcp::tool_result(vec![mcp::error_content(&err.error)]))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to send keys")),
            }
        }
        "get_buffer" => {
            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::GetBuffer(tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::Buffer(buffer) = response {
                        Ok(mcp::tool_result(vec![mcp::text_content(&buffer.content)]))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to get buffer")),
            }
        }
        "set_buffer" => {
            let content = arguments
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError::invalid_params("Missing 'content' argument"))?;

            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::SetBuffer(content.to_string(), tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::Success(_) = response {
                        Ok(mcp::tool_result(vec![mcp::text_content(
                            "Buffer set successfully",
                        )]))
                    } else if let super::state::ApiResponse::Error(err) = response {
                        Ok(mcp::tool_result(vec![mcp::error_content(&err.error)]))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to set buffer")),
            }
        }
        "get_cursor" => {
            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::GetCursor(tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::Cursor(cursor) = response {
                        let text = format!("Line: {}, Column: {}", cursor.line, cursor.column);
                        Ok(mcp::tool_result(vec![mcp::text_content(&text)]))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to get cursor")),
            }
        }
        "execute_command" => {
            let command = arguments
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError::invalid_params("Missing 'command' argument"))?;

            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::ExecuteCommand(command.to_string(), tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::Success(success) = response {
                        let msg = success
                            .message
                            .unwrap_or_else(|| "Command executed successfully".to_string());
                        Ok(mcp::tool_result(vec![mcp::text_content(&msg)]))
                    } else if let super::state::ApiResponse::Error(err) = response {
                        Ok(mcp::tool_result(vec![mcp::error_content(&err.error)]))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to execute command")),
            }
        }
        "lsp_hover" => {
            // Ensure NORMAL mode before triggering hover
            let (mode_tx, mode_rx) = oneshot::channel();
            let _ = state
                .tx
                .send(ApiRequest::SetMode("NORMAL".to_string(), mode_tx));
            let _ = mode_rx.await;

            // Send K to trigger hover
            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::SendKeys("K".to_string(), tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;
            let _ = rx.await;

            // Poll snapshot: wait for mode to become HOVER (LSP response arrived).
            // Uses GetSnapshotLight to avoid serializing the entire buffer on each poll.
            let mut hover_text = None;
            for _ in 0..10 {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                let (snap_tx, snap_rx) = oneshot::channel();
                if state
                    .tx
                    .send(ApiRequest::GetSnapshotLight(snap_tx))
                    .is_err()
                {
                    break;
                }
                if let Ok(super::state::ApiResponse::Snapshot(snapshot)) = snap_rx.await {
                    if snapshot.mode.contains("HOVER") {
                        hover_text = snapshot.hover_info;
                        break;
                    }
                }
            }

            // Dismiss hover popup, return to NORMAL mode
            let (esc_tx, esc_rx) = oneshot::channel();
            let _ = state
                .tx
                .send(ApiRequest::SetMode("NORMAL".to_string(), esc_tx));
            let _ = esc_rx.await;

            match hover_text {
                Some(text) => Ok(mcp::tool_result(vec![mcp::text_content(&text)])),
                None => Ok(mcp::tool_result(vec![mcp::text_content(
                    "No hover information available at cursor position",
                )])),
            }
        }
        "lsp_goto_definition" => {
            // Ensure NORMAL mode before triggering goto definition
            let (mode_tx, mode_rx) = oneshot::channel();
            let _ = state
                .tx
                .send(ApiRequest::SetMode("NORMAL".to_string(), mode_tx));
            let _ = mode_rx.await;

            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::SendKeys("gd".to_string(), tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::SendKeysResult(result) = response {
                        let json_str = serde_json::to_string_pretty(&result.context)
                            .unwrap_or_else(|_| "{}".to_string());
                        Ok(mcp::tool_result(vec![mcp::text_content(&json_str)]))
                    } else if let super::state::ApiResponse::Error(err) = response {
                        Ok(mcp::tool_result(vec![mcp::error_content(&err.error)]))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error(
                    "Failed to trigger goto definition",
                )),
            }
        }
        "get_snapshot" => {
            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::GetSnapshot(tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::Snapshot(snapshot) = response {
                        let json_str = serde_json::to_string_pretty(&snapshot)
                            .unwrap_or_else(|_| "{}".to_string());
                        Ok(mcp::tool_result(vec![mcp::text_content(&json_str)]))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to get snapshot")),
            }
        }
        "get_health" => {
            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::GetHealth(tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::Health(health) = response {
                        let json_str = serde_json::to_string_pretty(&health)
                            .unwrap_or_else(|_| "{}".to_string());
                        Ok(mcp::tool_result(vec![mcp::text_content(&json_str)]))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to get health")),
            }
        }
        "get_lsp_status" => {
            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::GetLspStatus(tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::LspStatus(lsp_status) = response {
                        let json_str = serde_json::to_string_pretty(&lsp_status)
                            .unwrap_or_else(|_| "{}".to_string());
                        Ok(mcp::tool_result(vec![mcp::text_content(&json_str)]))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to get LSP status")),
            }
        }
        "list_sessions" => {
            // This tool doesn't require editor state, just return the list of all sessions
            match crate::session::SessionInfo::list_all() {
                Ok(sessions) => {
                    let json_str = serde_json::to_string_pretty(&sessions)
                        .unwrap_or_else(|_| "[]".to_string());
                    Ok(mcp::tool_result(vec![mcp::text_content(&json_str)]))
                }
                Err(e) => Err(JsonRpcError::internal_error(&format!(
                    "Failed to list sessions: {}",
                    e
                ))),
            }
        }
        "set_mode" => {
            let mode_str = arguments
                .get("mode")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError::invalid_params("Missing 'mode' argument"))?;

            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::SetMode(mode_str.to_string(), tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::Success(success) = response {
                        let msg = success
                            .message
                            .unwrap_or_else(|| format!("Mode set to {}", mode_str));
                        Ok(mcp::tool_result(vec![mcp::text_content(&msg)]))
                    } else if let super::state::ApiResponse::Error(err) = response {
                        Ok(mcp::tool_result(vec![mcp::error_content(&err.error)]))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to set mode")),
            }
        }
        "get_outline" => {
            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::GetOutline(tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::Outline(info) = response {
                        let json_str = serde_json::to_string_pretty(&info)
                            .unwrap_or_else(|_| "{}".to_string());
                        Ok(mcp::tool_result(vec![mcp::text_content(&json_str)]))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to get outline")),
            }
        }
        "search_symbol" => {
            let query = arguments
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError::invalid_params("Missing 'query' argument"))?;

            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::SearchSymbol(query.to_string(), tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::SymbolSearch(info) = response {
                        let json_str = serde_json::to_string_pretty(&info)
                            .unwrap_or_else(|_| "{}".to_string());
                        Ok(mcp::tool_result(vec![mcp::text_content(&json_str)]))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to search symbols")),
            }
        }
        "get_trace" => {
            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::GetTrace(tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::Trace(info) = response {
                        let json_str = serde_json::to_string_pretty(&info)
                            .unwrap_or_else(|_| "{}".to_string());
                        Ok(mcp::tool_result(vec![mcp::text_content(&json_str)]))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to get trace")),
            }
        }
        "get_diagnostics" => {
            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::GetDiagnostics(tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(super::state::ApiResponse::Diagnostics(info)) => {
                    let json_str =
                        serde_json::to_string_pretty(&info).unwrap_or_else(|_| "{}".to_string());
                    Ok(mcp::tool_result(vec![mcp::text_content(&json_str)]))
                }
                _ => Err(JsonRpcError::internal_error("Failed to get diagnostics")),
            }
        }
        "get_context_window" => {
            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::GetContextWindow(tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::ContextWindow(ctx) = response {
                        let json_str =
                            serde_json::to_string_pretty(&ctx).unwrap_or_else(|_| "{}".to_string());
                        Ok(mcp::tool_result(vec![mcp::text_content(&json_str)]))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to get context window")),
            }
        }
        _ => Err(JsonRpcError::invalid_params(&format!(
            "Unknown tool: {}",
            tool_name
        ))),
    }
}

/// Handle resources/read method
async fn handle_resource_read(state: ApiState, params: Value) -> Result<Value, JsonRpcError> {
    let uri = params
        .get("uri")
        .and_then(|v| v.as_str())
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'uri' field"))?;

    match uri {
        "ovim://context-window" => {
            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::GetContextWindow(tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::ContextWindow(ctx) = response {
                        Ok(json!({
                            "contents": [{
                                "uri": uri,
                                "mimeType": "text/plain",
                                "text": ctx.context
                            }]
                        }))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to get context window")),
            }
        }
        "ovim://buffer" => {
            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::GetBuffer(tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::Buffer(buffer) = response {
                        Ok(json!({
                            "contents": [{
                                "uri": uri,
                                "mimeType": "text/plain",
                                "text": buffer.content
                            }]
                        }))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to get buffer")),
            }
        }
        "ovim://snapshot" => {
            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::GetSnapshot(tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::Snapshot(snapshot) = response {
                        Ok(json!({
                            "contents": [{
                                "uri": uri,
                                "mimeType": "application/json",
                                "text": serde_json::to_string_pretty(&snapshot)
                                    .unwrap_or_else(|_| "{}".to_string())
                            }]
                        }))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to get snapshot")),
            }
        }
        "ovim://lsp/status" => {
            let (tx, rx) = oneshot::channel();
            state
                .tx
                .send(ApiRequest::GetLspStatus(tx))
                .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

            match rx.await {
                Ok(response) => {
                    if let super::state::ApiResponse::LspStatus(lsp_status) = response {
                        Ok(json!({
                            "contents": [{
                                "uri": uri,
                                "mimeType": "application/json",
                                "text": serde_json::to_string_pretty(&lsp_status)
                                    .unwrap_or_else(|_| "{}".to_string())
                            }]
                        }))
                    } else {
                        Err(JsonRpcError::internal_error("Unexpected response type"))
                    }
                }
                Err(_) => Err(JsonRpcError::internal_error("Failed to get LSP status")),
            }
        }
        _ if uri.starts_with("file://") => {
            // For file:// URIs, return the buffer content if it matches
            let file_path = get_current_file_path(&state).await;
            let expected_uri = file_path.map(|p| format!("file://{}", p));

            if Some(uri.to_string()) == expected_uri {
                let (tx, rx) = oneshot::channel();
                state
                    .tx
                    .send(ApiRequest::GetBuffer(tx))
                    .map_err(|_| JsonRpcError::internal_error("Editor not available"))?;

                match rx.await {
                    Ok(response) => {
                        if let super::state::ApiResponse::Buffer(buffer) = response {
                            Ok(json!({
                                "contents": [{
                                    "uri": uri,
                                    "mimeType": "text/plain",
                                    "text": buffer.content
                                }]
                            }))
                        } else {
                            Err(JsonRpcError::internal_error("Unexpected response type"))
                        }
                    }
                    Err(_) => Err(JsonRpcError::internal_error("Failed to get buffer")),
                }
            } else {
                Err(JsonRpcError::invalid_params(&format!(
                    "File not found: {}",
                    uri
                )))
            }
        }
        _ => Err(JsonRpcError::invalid_params(&format!(
            "Unknown resource URI: {}",
            uri
        ))),
    }
}

/// Get current file path from editor
async fn get_current_file_path(state: &ApiState) -> Option<String> {
    let (tx, rx) = oneshot::channel();
    state.tx.send(ApiRequest::GetBuffer(tx)).ok()?;

    match rx.await {
        Ok(super::state::ApiResponse::Buffer(buffer)) => buffer.file_path,
        _ => None,
    }
}
