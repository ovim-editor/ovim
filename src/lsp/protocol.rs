//! JSON-RPC protocol implementation for LSP
//!
//! Handles message framing, serialization, and deserialization for the
//! Language Server Protocol over stdio.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

/// JSON-RPC request/response/notification identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(u64),
    String(String),
}

/// JSON-RPC error object
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// JSON-RPC message (request, response, or notification)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcMessage {
    pub jsonrpc: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<RequestId>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

impl JsonRpcMessage {
    /// Creates a JSON-RPC request
    pub fn request(id: RequestId, method: String, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            method: Some(method),
            params: Some(params),
            result: None,
            error: None,
        }
    }

    /// Creates a JSON-RPC notification (no id, no response expected)
    pub fn notification(method: String, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: Some(method),
            params: Some(params),
            result: None,
            error: None,
        }
    }

    /// Creates a JSON-RPC success response
    pub fn response(id: RequestId, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            method: None,
            params: None,
            result: Some(result),
            error: None,
        }
    }

    /// Creates a JSON-RPC error response
    pub fn error_response(id: RequestId, error: ResponseError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            method: None,
            params: None,
            result: None,
            error: Some(error),
        }
    }

    /// Checks if this is a request (has id and method)
    pub fn is_request(&self) -> bool {
        self.id.is_some() && self.method.is_some()
    }

    /// Checks if this is a notification (has method but no id)
    pub fn is_notification(&self) -> bool {
        self.id.is_none() && self.method.is_some()
    }

    /// Checks if this is a response (has id but no method)
    pub fn is_response(&self) -> bool {
        self.id.is_some() && self.method.is_none()
    }
}

/// Writes a JSON-RPC message with Content-Length header framing
pub async fn write_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: &JsonRpcMessage,
) -> Result<()> {
    let json = serde_json::to_string(message)?;
    let content_length = json.len();

    // Debug log outgoing messages
    if message.is_notification() {
        if let Some(method) = &message.method {
            crate::lsp_debug!("LSP-OUT", "Sending notification: {} | Body: {}", method, json);
        }
    } else if message.is_request() {
        if let Some(method) = &message.method {
            crate::lsp_debug!("LSP-OUT", "Sending request: {} | Body: {}", method, json);
        }
    }

    // Write headers
    writer
        .write_all(format!("Content-Length: {}\r\n\r\n", content_length).as_bytes())
        .await?;

    // Write body
    writer.write_all(json.as_bytes()).await?;
    writer.flush().await?;

    Ok(())
}

/// Reads a JSON-RPC message with Content-Length header framing
pub async fn read_message<R: AsyncRead + Unpin>(reader: &mut R) -> Result<JsonRpcMessage> {
    let mut buf_reader = BufReader::new(reader);
    let mut headers = Vec::new();

    // Read headers until we find an empty line
    loop {
        let mut line = String::new();
        buf_reader.read_line(&mut line).await?;

        if line.trim().is_empty() {
            break; // End of headers
        }

        headers.push(line);
    }

    // Parse Content-Length header
    let content_length = headers
        .iter()
        .find_map(|line| {
            if line.starts_with("Content-Length:") {
                line.split(':')
                    .nth(1)
                    .and_then(|s| s.trim().parse::<usize>().ok())
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow!("Missing Content-Length header"))?;

    // Validate content_length to prevent buffer overflow
    const MAX_MESSAGE_SIZE: usize = 50 * 1024 * 1024; // 50MB
    if content_length > MAX_MESSAGE_SIZE {
        return Err(anyhow!(
            "Message size {} exceeds maximum allowed size {}",
            content_length,
            MAX_MESSAGE_SIZE
        ));
    }

    // Read exact content_length bytes
    let mut content = vec![0u8; content_length];
    buf_reader.read_exact(&mut content).await?;

    // Parse JSON
    let message: JsonRpcMessage = serde_json::from_slice(&content)?;

    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_id_serialization() {
        let id_num = RequestId::Number(42);
        let json = serde_json::to_string(&id_num).unwrap();
        assert_eq!(json, "42");

        let id_str = RequestId::String("test-id".to_string());
        let json = serde_json::to_string(&id_str).unwrap();
        assert_eq!(json, r#""test-id""#);
    }

    #[test]
    fn test_json_rpc_request() {
        let msg = JsonRpcMessage::request(
            RequestId::Number(1),
            "initialize".to_string(),
            serde_json::json!({"processId": 1234}),
        );

        assert!(msg.is_request());
        assert!(!msg.is_notification());
        assert!(!msg.is_response());
        assert_eq!(msg.method, Some("initialize".to_string()));
        assert_eq!(msg.id, Some(RequestId::Number(1)));
    }

    #[test]
    fn test_json_rpc_notification() {
        let msg = JsonRpcMessage::notification(
            "textDocument/didOpen".to_string(),
            serde_json::json!({"uri": "file:///test.rs"}),
        );

        assert!(!msg.is_request());
        assert!(msg.is_notification());
        assert!(!msg.is_response());
        assert_eq!(msg.method, Some("textDocument/didOpen".to_string()));
        assert_eq!(msg.id, None);
    }

    #[test]
    fn test_json_rpc_response() {
        let msg = JsonRpcMessage::response(
            RequestId::Number(1),
            serde_json::json!({"capabilities": {}}),
        );

        assert!(!msg.is_request());
        assert!(!msg.is_notification());
        assert!(msg.is_response());
        assert_eq!(msg.id, Some(RequestId::Number(1)));
        assert!(msg.result.is_some());
    }

    #[test]
    fn test_json_rpc_error_response() {
        let error = ResponseError {
            code: -32600,
            message: "Invalid request".to_string(),
            data: None,
        };

        let msg = JsonRpcMessage::error_response(RequestId::Number(1), error);

        assert!(msg.is_response());
        assert!(msg.error.is_some());
        assert_eq!(msg.result, None);
    }

    #[tokio::test]
    async fn test_message_write_read_roundtrip() {
        let original = JsonRpcMessage::request(
            RequestId::Number(42),
            "test".to_string(),
            serde_json::json!({"key": "value"}),
        );

        let mut buffer = Vec::new();
        write_message(&mut buffer, &original).await.unwrap();

        // Verify format
        let output = String::from_utf8(buffer.clone()).unwrap();
        assert!(output.starts_with("Content-Length: "));
        assert!(output.contains("\r\n\r\n"));
        assert!(output.contains(r#""jsonrpc":"2.0""#));

        // Read back
        let mut cursor = std::io::Cursor::new(buffer);
        let parsed = read_message(&mut cursor).await.unwrap();

        assert_eq!(parsed.id, original.id);
        assert_eq!(parsed.method, original.method);
        assert_eq!(parsed.params, original.params);
    }
}
