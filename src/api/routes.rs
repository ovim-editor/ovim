use super::handlers::{
    execute_command, get_buffer, get_cursor, get_mode, get_snapshot, send_keys, set_buffer,
};
use super::state::ApiState;
use axum::{
    routing::{get, post, put},
    Router,
};

/// Create the API router with all routes
pub fn create_router(state: ApiState) -> Router {
    Router::new()
        .route("/snapshot", get(get_snapshot))
        .route("/keys", post(send_keys))
        .route("/buffer", get(get_buffer))
        .route("/buffer", put(set_buffer))
        .route("/cursor", get(get_cursor))
        .route("/mode", get(get_mode))
        .route("/command", post(execute_command))
        .with_state(state)
}
