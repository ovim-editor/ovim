mod handlers;
mod routes;
mod state;

pub use routes::create_router;
pub use state::{ApiRequest, ApiResponse, ApiState, BufferInfo, CursorPosition, EditorSnapshot, ErrorResponse, ModeInfo, PickerInfo, PickerResultInfo, SuccessResponse, VisualSelection, parse_key_string};

use anyhow::Result;
use tokio::sync::mpsc;

/// Start the API server on the given address
pub async fn start_server(
    addr: &str,
    tx: mpsc::UnboundedSender<ApiRequest>,
) -> Result<()> {
    let state = ApiState::new(tx);
    let app = create_router(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let actual_addr = listener.local_addr()?;

    eprintln!("REST API server listening on http://{}", actual_addr);
    eprintln!("API URL: http://{}", actual_addr);

    axum::serve(listener, app).await?;

    Ok(())
}
