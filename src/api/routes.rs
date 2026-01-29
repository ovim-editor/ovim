use super::handlers::{
    execute_command, get_buffer, get_cursor, get_health, get_lsp_status, get_metrics, get_mode,
    get_outline, get_prometheus_metrics, get_render, get_snapshot, get_trace, search_symbol,
    send_keys, set_buffer, set_mode,
};
use super::mcp_handler::handle_mcp;
use super::state::ApiState;
use axum::{
    routing::{get, post, put},
    Router,
};

/// Create the API router with all routes
pub fn create_router(state: ApiState) -> Router {
    // V1 API routes (current stable API)
    let v1_routes = Router::new()
        .route("/health", get(get_health))
        .route("/snapshot", get(get_snapshot))
        .route("/keys", post(send_keys))
        .route("/buffer", get(get_buffer))
        .route("/buffer", put(set_buffer))
        .route("/cursor", get(get_cursor))
        .route("/mode", get(get_mode))
        .route("/mode", post(set_mode))
        .route("/command", post(execute_command))
        .route("/render", get(get_render))
        .route("/lsp/status", get(get_lsp_status))
        .route("/metrics", get(get_metrics))
        .route("/prometheus", get(get_prometheus_metrics))
        .route("/outline", get(get_outline))
        .route("/symbol", get(search_symbol))
        .route("/trace", get(get_trace))
        .route("/mcp", post(handle_mcp));

    // Root router with version namespaces
    Router::new()
        // V1 API under /v1 prefix (recommended)
        .nest("/v1", v1_routes.clone())

        // Legacy routes (no prefix) - for backward compatibility
        // These will be removed in ovim v1.0
        .merge(v1_routes)

        .with_state(state)
}
