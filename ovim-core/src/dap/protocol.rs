//! DAP protocol message framing and serialization.
//!
//! DAP uses Content-Length framing (same as LSP) with a different JSON envelope:
//! - Requests: `{ seq, type: "request", command, arguments }`
//! - Responses: `{ seq, type: "response", request_seq, success, command, body }`
//! - Events: `{ seq, type: "event", event, body }`

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

/// Outgoing DAP request.
#[derive(Debug, Serialize)]
pub struct DapRequest {
    pub seq: u64,
    #[serde(rename = "type")]
    pub message_type: &'static str,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
}

/// Incoming DAP message (response or event).
#[derive(Debug, Deserialize)]
pub struct DapIncoming {
    #[serde(default)]
    pub seq: u64,
    #[serde(rename = "type")]
    pub message_type: String,
    // Response fields
    #[serde(default)]
    pub request_seq: Option<u64>,
    #[serde(default)]
    pub success: Option<bool>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub body: Option<Value>,
    // Event fields
    #[serde(default)]
    pub event: Option<String>,
}

impl DapIncoming {
    pub fn is_response(&self) -> bool {
        self.message_type == "response"
    }

    pub fn is_event(&self) -> bool {
        self.message_type == "event"
    }
}

/// Write a DAP request with Content-Length framing.
pub async fn write_request<W: AsyncWrite + Unpin>(
    writer: &mut W,
    request: &DapRequest,
) -> Result<()> {
    let json = serde_json::to_string(request)?;
    let header = format!("Content-Length: {}\r\n\r\n", json.len());
    writer.write_all(header.as_bytes()).await?;
    writer.write_all(json.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}

/// Read a single DAP message from a Content-Length framed stream.
/// Returns `None` on EOF.
pub async fn read_message<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<Option<DapIncoming>> {
    let mut content_length: Option<usize> = None;

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Ok(None); // EOF
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }

        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = value.trim().parse().ok();
        }
    }

    let length = content_length.ok_or_else(|| anyhow!("missing Content-Length header"))?;

    const MAX_MESSAGE_SIZE: usize = 50 * 1024 * 1024;
    if length > MAX_MESSAGE_SIZE {
        return Err(anyhow!("message size {length} exceeds maximum"));
    }

    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await?;

    let msg: DapIncoming = serde_json::from_slice(&body)?;
    Ok(Some(msg))
}
