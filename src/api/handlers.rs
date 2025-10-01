use super::state::{ApiRequest, ApiResponse, ApiState};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    Json as JsonExtractor,
};
use serde::Deserialize;
use tokio::sync::oneshot;

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

/// Handler for POST /command
#[derive(Deserialize)]
pub struct ExecuteCommandRequest {
    pub command: String,
}

pub async fn execute_command(
    State(state): State<ApiState>,
    JsonExtractor(payload): JsonExtractor<ExecuteCommandRequest>,
) -> Response {
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
