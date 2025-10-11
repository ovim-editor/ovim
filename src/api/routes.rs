use super::handlers::{
    execute_command, get_buffer, get_cursor, get_health, get_lsp_status, get_mode, get_render, get_snapshot, send_keys, set_buffer,
};
use super::state::ApiState;
use axum::{
    routing::{get, post, put},
    Router,
};

/// Create the API router with all routes
pub fn create_router(state: ApiState) -> Router {
    Router::new()
        .route("/health", get(get_health))
        .route("/snapshot", get(get_snapshot))
        .route("/keys", post(send_keys))
        .route("/buffer", get(get_buffer))
        .route("/buffer", put(set_buffer))
        .route("/cursor", get(get_cursor))
        .route("/mode", get(get_mode))
        .route("/command", post(execute_command))
        .route("/render", get(get_render))
        .route("/lsp/status", get(get_lsp_status))
        .with_state(state)
}
