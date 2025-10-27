mod handlers;
mod mcp;
mod mcp_handler;
mod routes;
mod state;

pub use mcp::{JsonRpcRequest, JsonRpcResponse};
pub use routes::create_router;
pub use state::{
    parse_key_string, ApiRequest, ApiResponse, ApiState, BufferInfo, CursorPosition,
    EditorSnapshot, ErrorResponse, HealthInfo, LspServerInfoItem, LspStatusInfo, MetricsInfo,
    ModeInfo, PickerInfo, PickerResultInfo, RenderInfo, SuccessResponse, VisualSelection,
};

use anyhow::Result;
use tokio::sync::mpsc;

/// Start the API server on the given address
/// Returns the actual port number the server is listening on
pub async fn start_server(
    addr: &str,
    tx: mpsc::UnboundedSender<ApiRequest>,
    port_tx: tokio::sync::oneshot::Sender<u16>,
) -> Result<()> {
    let state = ApiState::new(tx);
    let app = create_router(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let actual_addr = listener.local_addr()?;

    eprintln!("REST API server listening on http://{}", actual_addr);
    eprintln!("API URL: http://{}", actual_addr);

    // Send the actual port back
    let _ = port_tx.send(actual_addr.port());

    axum::serve(listener, app).await?;

    Ok(())
}
