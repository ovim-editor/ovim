use super::state::{ApiRequest, ApiState};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    Json as JsonExtractor,
};
use serde::Deserialize;
use tokio::sync::oneshot;

// Input validation constants
const MAX_KEYS_LENGTH: usize = 100_000; // 100KB of key input
const MAX_BUFFER_SIZE: usize = 100_000_000; // 100MB max buffer content
const MAX_COMMAND_LENGTH: usize = 10_000; // 10KB max command length

/// Handler for GET /snapshot
pub async fn get_snapshot(State(state): State<ApiState>) -> Response {
    let (tx, rx) = oneshot::channel();

    if state.tx.send(ApiRequest::GetSnapshot(tx)).is_err() {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to get snapshot"),
    }
}

/// Handler for POST /keys
#[derive(Deserialize)]
pub struct SendKeysRequest {
    pub keys: String,
}

pub async fn send_keys(
    State(state): State<ApiState>,
    JsonExtractor(payload): JsonExtractor<SendKeysRequest>,
) -> Response {
    // Validate input length
    if payload.keys.len() > MAX_KEYS_LENGTH {
        return validation_error(&format!(
            "Keys input too large: {} bytes (max: {} bytes)",
            payload.keys.len(),
            MAX_KEYS_LENGTH
        ));
    }

    let (tx, rx) = oneshot::channel();

    if state
        .tx
        .send(ApiRequest::SendKeys(payload.keys, tx))
        .is_err()
    {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to send keys"),
    }
}

/// Handler for GET /buffer
pub async fn get_buffer(State(state): State<ApiState>) -> Response {
    let (tx, rx) = oneshot::channel();

    if state.tx.send(ApiRequest::GetBuffer(tx)).is_err() {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to get buffer"),
    }
}

/// Handler for PUT /buffer
#[derive(Deserialize)]
pub struct SetBufferRequest {
    pub content: String,
}

pub async fn set_buffer(
    State(state): State<ApiState>,
    JsonExtractor(payload): JsonExtractor<SetBufferRequest>,
) -> Response {
    // Validate input length
    if payload.content.len() > MAX_BUFFER_SIZE {
        return validation_error(&format!(
            "Buffer content too large: {} bytes (max: {} bytes)",
            payload.content.len(),
            MAX_BUFFER_SIZE
        ));
    }

    let (tx, rx) = oneshot::channel();

    if state
        .tx
        .send(ApiRequest::SetBuffer(payload.content, tx))
        .is_err()
    {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to set buffer"),
    }
}

/// Handler for GET /cursor
pub async fn get_cursor(State(state): State<ApiState>) -> Response {
    let (tx, rx) = oneshot::channel();

    if state.tx.send(ApiRequest::GetCursor(tx)).is_err() {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to get cursor"),
    }
}

/// Handler for GET /mode
pub async fn get_mode(State(state): State<ApiState>) -> Response {
    let (tx, rx) = oneshot::channel();

    if state.tx.send(ApiRequest::GetMode(tx)).is_err() {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to get mode"),
    }
}

/// Handler for POST /mode
#[derive(Deserialize)]
pub struct SetModeRequest {
    pub mode: String,
}

pub async fn set_mode(
    State(state): State<ApiState>,
    JsonExtractor(payload): JsonExtractor<SetModeRequest>,
) -> Response {
    let (tx, rx) = oneshot::channel();

    if state
        .tx
        .send(ApiRequest::SetMode(payload.mode, tx))
        .is_err()
    {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to set mode"),
    }
}

/// Handler for POST /command
#[derive(Deserialize)]
pub struct ExecuteCommandRequest {
    pub command: String,
}

pub async fn execute_command(
    State(state): State<ApiState>,
    JsonExtractor(payload): JsonExtractor<ExecuteCommandRequest>,
) -> Response {
    // Validate input length
    if payload.command.len() > MAX_COMMAND_LENGTH {
        return validation_error(&format!(
            "Command too large: {} bytes (max: {} bytes)",
            payload.command.len(),
            MAX_COMMAND_LENGTH
        ));
    }

    let (tx, rx) = oneshot::channel();

    if state
        .tx
        .send(ApiRequest::ExecuteCommand(payload.command, tx))
        .is_err()
    {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to execute command"),
    }
}

/// Handler for GET /render
/// Returns pixel-perfect ANSI representation of the editor
pub async fn get_render(State(state): State<ApiState>) -> Response {
    let (tx, rx) = oneshot::channel();

    if state.tx.send(ApiRequest::GetRender(tx)).is_err() {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to render"),
    }
}

/// Handler for GET /lsp/status
/// Returns LSP server status information
pub async fn get_lsp_status(State(state): State<ApiState>) -> Response {
    let (tx, rx) = oneshot::channel();

    if state.tx.send(ApiRequest::GetLspStatus(tx)).is_err() {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to get LSP status"),
    }
}

/// Handler for GET /health
/// Returns health check information including LSP readiness
pub async fn get_health(State(state): State<ApiState>) -> Response {
    let (tx, rx) = oneshot::channel();

    if state.tx.send(ApiRequest::GetHealth(tx)).is_err() {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to get health"),
    }
}

/// Handler for GET /metrics
/// Returns performance metrics information
pub async fn get_metrics(State(state): State<ApiState>) -> Response {
    let (tx, rx) = oneshot::channel();

    if state.tx.send(ApiRequest::GetMetrics(tx)).is_err() {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to get metrics"),
    }
}

/// Helper function to create error responses
fn error_response(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": message
        })),
    )
        .into_response()
}

/// Helper function to create validation error responses
fn validation_error(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": message
        })),
    )
        .into_response()
}
