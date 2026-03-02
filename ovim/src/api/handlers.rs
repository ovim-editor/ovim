use super::state::{ApiRequest, ApiResponse, ApiState};
use crate::metrics;
use axum::{
    extract::State,
    http::{header, HeaderValue, StatusCode},
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
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

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
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

    // Validate input length
    if payload.keys.len() > MAX_KEYS_LENGTH {
        return plain_text_error(
            StatusCode::BAD_REQUEST,
            &format!(
                "Keys input too large: {} bytes (max: {} bytes)",
                payload.keys.len(),
                MAX_KEYS_LENGTH
            ),
        );
    }

    let (tx, rx) = oneshot::channel();

    if state
        .tx
        .send(ApiRequest::SendKeys(payload.keys, tx))
        .is_err()
    {
        return plain_text_error(StatusCode::INTERNAL_SERVER_ERROR, "Editor not available");
    }

    match rx.await {
        Ok(response) => send_keys_to_plain_text(response),
        Err(_) => plain_text_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to send keys"),
    }
}

/// Convert a SendKeys API response into a plain-text HTTP response with metadata headers.
fn send_keys_to_plain_text(response: ApiResponse) -> Response {
    match response {
        ApiResponse::SendKeysResult(result) => {
            let ctx = &result.context;
            let mut builder = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .header("X-Ovim-Success", "true")
                .header("X-Ovim-Mode", &ctx.mode)
                .header("X-Ovim-Line", (ctx.line + 1).to_string())
                .header("X-Ovim-Column", (ctx.column + 1).to_string());

            if let Some(ref file) = ctx.file {
                if let Ok(val) = HeaderValue::from_str(file) {
                    builder = builder.header("X-Ovim-File", val);
                }
            }

            builder.body(ctx.context.clone().into()).unwrap()
        }
        ApiResponse::Error(err) => {
            plain_text_error(StatusCode::INTERNAL_SERVER_ERROR, &err.error)
        }
        _ => plain_text_error(StatusCode::INTERNAL_SERVER_ERROR, "Unexpected response type"),
    }
}

/// Handler for GET /buffer
pub async fn get_buffer(State(state): State<ApiState>) -> Response {
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

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
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

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
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

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
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

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
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

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
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

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
/// Returns pixel-perfect ANSI representation of the editor.
///
/// Query params:
///   `width`  — terminal columns (default 120, max 500)
///   `height` — terminal rows   (default 40, max 200)
///   `plain`  — if `true`, strip ANSI escapes and return the raw character grid
#[derive(Deserialize)]
pub struct RenderQuery {
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub plain: Option<bool>,
}

pub async fn get_render(
    State(state): State<ApiState>,
    axum::extract::Query(params): axum::extract::Query<RenderQuery>,
) -> Response {
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

    let width = params.width.unwrap_or(120).min(500).max(10);
    let height = params.height.unwrap_or(40).min(200).max(3);
    let plain = params.plain.unwrap_or(false);

    let (tx, rx) = oneshot::channel();

    if state
        .tx
        .send(ApiRequest::GetRender {
            width,
            height,
            plain,
            tx,
        })
        .is_err()
    {
        return plain_text_error(StatusCode::INTERNAL_SERVER_ERROR, "Editor not available");
    }

    match rx.await {
        Ok(response) => render_to_plain_text(response),
        Err(_) => plain_text_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to render"),
    }
}

/// Convert a Render API response into a plain-text HTTP response with metadata headers.
fn render_to_plain_text(response: ApiResponse) -> Response {
    match response {
        ApiResponse::Render(info) => {
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .header("X-Ovim-Width", info.width.to_string())
                .header("X-Ovim-Height", info.height.to_string())
                .body(info.ansi.into())
                .unwrap()
        }
        ApiResponse::Error(err) => {
            plain_text_error(StatusCode::INTERNAL_SERVER_ERROR, &err.error)
        }
        _ => plain_text_error(StatusCode::INTERNAL_SERVER_ERROR, "Unexpected response type"),
    }
}

/// Handler for GET /lsp/status
/// Returns LSP server status information
pub async fn get_lsp_status(State(state): State<ApiState>) -> Response {
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

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
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

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
/// Returns performance metrics information (JSON format)
pub async fn get_metrics(State(state): State<ApiState>) -> Response {
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

    let (tx, rx) = oneshot::channel();

    if state.tx.send(ApiRequest::GetMetrics(tx)).is_err() {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to get metrics"),
    }
}

/// Handler for GET /v1/prometheus or /prometheus
/// Returns metrics in Prometheus text format for scraping
///
/// This endpoint returns metrics in the Prometheus exposition format, which can be
/// scraped by Prometheus servers or compatible monitoring tools.
///
/// # Example Output
///
/// ```text
/// # HELP ovim_http_requests_total Total HTTP API requests received
/// # TYPE ovim_http_requests_total counter
/// ovim_http_requests_total 42
/// # HELP ovim_buffer_edits_total Total buffer edit operations
/// # TYPE ovim_buffer_edits_total counter
/// ovim_buffer_edits_total 15
/// ...
/// ```
pub async fn get_prometheus_metrics() -> Response {
    // Note: We deliberately don't instrument this endpoint to avoid metric explosion
    let metrics_text = metrics::export_metrics();

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        metrics_text,
    )
        .into_response()
}

/// Handler for GET /outline
pub async fn get_outline(State(state): State<ApiState>) -> Response {
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

    let (tx, rx) = oneshot::channel();

    if state.tx.send(ApiRequest::GetOutline(tx)).is_err() {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to get outline"),
    }
}

/// Handler for GET /symbol?q=query
#[derive(Deserialize)]
pub struct SymbolSearchQuery {
    pub q: String,
}

pub async fn search_symbol(
    State(state): State<ApiState>,
    axum::extract::Query(params): axum::extract::Query<SymbolSearchQuery>,
) -> Response {
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

    let (tx, rx) = oneshot::channel();

    if state
        .tx
        .send(ApiRequest::SearchSymbol(params.q, tx))
        .is_err()
    {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to search symbols"),
    }
}

/// Handler for GET /trace
pub async fn get_trace(State(state): State<ApiState>) -> Response {
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

    let (tx, rx) = oneshot::channel();

    if state.tx.send(ApiRequest::GetTrace(tx)).is_err() {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to get trace"),
    }
}

/// Handler for GET /diagnostics
pub async fn get_diagnostics(State(state): State<ApiState>) -> Response {
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

    let (tx, rx) = oneshot::channel();

    if state.tx.send(ApiRequest::GetDiagnostics(tx)).is_err() {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to get diagnostics"),
    }
}

/// Handler for POST /edit
#[derive(Deserialize)]
pub struct EditRequest {
    pub line: Option<usize>,
    pub old: String,
    pub new: String,
}

pub async fn edit_line(
    State(state): State<ApiState>,
    JsonExtractor(payload): JsonExtractor<EditRequest>,
) -> Response {
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

    let (tx, rx) = oneshot::channel();

    // Convert 1-indexed line to 0-indexed
    let line = payload.line.map(|l| l.saturating_sub(1));

    if state
        .tx
        .send(ApiRequest::EditLine {
            line,
            old: payload.old,
            new: payload.new,
            tx,
        })
        .is_err()
    {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to edit line"),
    }
}

/// Handler for POST /insert
#[derive(Deserialize)]
pub struct InsertRequest {
    pub after: Option<usize>,
    pub before: Option<usize>,
    pub text: String,
}

pub async fn insert_lines(
    State(state): State<ApiState>,
    JsonExtractor(payload): JsonExtractor<InsertRequest>,
) -> Response {
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

    // Determine insert position (0-indexed) and direction
    let (line, is_before) = match (payload.after, payload.before) {
        (Some(after), None) => (after, false), // after line N means insert at line N+1 (but after=0 means before line 1)
        (None, Some(before)) => (before.saturating_sub(1), true), // before line N (1-indexed) -> 0-indexed
        (None, None) => {
            return validation_error("Either 'after' or 'before' must be specified");
        }
        (Some(_), Some(_)) => {
            return validation_error("Cannot specify both 'after' and 'before'");
        }
    };

    let (tx, rx) = oneshot::channel();

    if state
        .tx
        .send(ApiRequest::InsertLines {
            line,
            before: is_before,
            text: payload.text,
            tx,
        })
        .is_err()
    {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to insert lines"),
    }
}

/// Handler for POST /delete-lines
#[derive(Deserialize)]
pub struct DeleteLinesRequest {
    pub from: usize,
    pub to: usize,
}

pub async fn delete_lines(
    State(state): State<ApiState>,
    JsonExtractor(payload): JsonExtractor<DeleteLinesRequest>,
) -> Response {
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

    // Convert 1-indexed to 0-indexed
    let from = payload.from.saturating_sub(1);
    let to = payload.to.saturating_sub(1);

    let (tx, rx) = oneshot::channel();

    if state
        .tx
        .send(ApiRequest::DeleteLines { from, to, tx })
        .is_err()
    {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to delete lines"),
    }
}

/// Handler for GET /lines?from=N&to=M
#[derive(Deserialize)]
pub struct ReadLinesQuery {
    pub from: usize,
    pub to: usize,
}

pub async fn read_lines(
    State(state): State<ApiState>,
    axum::extract::Query(params): axum::extract::Query<ReadLinesQuery>,
) -> Response {
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

    // Convert 1-indexed to 0-indexed
    let from = params.from.saturating_sub(1);
    let to = params.to.saturating_sub(1);

    let (tx, rx) = oneshot::channel();

    if state
        .tx
        .send(ApiRequest::ReadLines { from, to, tx })
        .is_err()
    {
        return error_response("Editor not available");
    }

    match rx.await {
        Ok(response) => Json(response).into_response(),
        Err(_) => error_response("Failed to read lines"),
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

/// Plain-text error response (for endpoints that return text/plain)
fn plain_text_error(status: StatusCode, message: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header("X-Ovim-Success", "false")
        .body(message.to_string().into())
        .unwrap()
}
