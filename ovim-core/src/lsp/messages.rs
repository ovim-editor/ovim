//! Server-to-user messages: `window/showMessage` and
//! `window/showMessageRequest`.
//!
//! The manager only queues them; the editor shows them (toast/status line, or
//! an action picker for requests) and answers requests through
//! [`LspManager::reply_message_request`]. `window/logMessage` goes to the LSP
//! log file (see `notifications.rs`).

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageSeverity {
    Error,
    Warning,
    Info,
    Log,
}

impl MessageSeverity {
    pub(super) fn from_lsp(typ: lsp_types::MessageType) -> Self {
        match typ {
            lsp_types::MessageType::ERROR => Self::Error,
            lsp_types::MessageType::WARNING => Self::Warning,
            lsp_types::MessageType::INFO => Self::Info,
            _ => Self::Log,
        }
    }
}

/// A `window/showMessageRequest` waiting for the user's choice.
#[derive(Clone, Debug)]
pub struct MessageRequest {
    pub server_id: String,
    pub id: RequestId,
    pub severity: MessageSeverity,
    pub message: String,
    pub actions: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum ServerMessage {
    Notice {
        server_id: String,
        severity: MessageSeverity,
        message: String,
    },
    Request(MessageRequest),
}

/// Bound on queued messages so a chatty server cannot grow memory.
const MAX_QUEUED_MESSAGES: usize = 200;

impl LspManager {
    pub(super) fn queue_server_message(&self, message: ServerMessage) {
        if let Ok(mut queue) = self.server_messages.lock() {
            if queue.len() >= MAX_QUEUED_MESSAGES {
                queue.remove(0);
            }
            queue.push(message);
        }
    }

    /// Drains messages the servers want the user to see.
    pub fn take_server_messages(&self) -> Vec<ServerMessage> {
        self.server_messages
            .lock()
            .map(|mut queue| std::mem::take(&mut *queue))
            .unwrap_or_default()
    }

    /// Answers a `window/showMessageRequest` with the chosen action title, or
    /// `None` when the user dismissed it.
    pub async fn reply_message_request(&self, request: &MessageRequest, chosen: Option<&str>) {
        let Some(server) = self
            .servers
            .get(&request.server_id)
            .map(|entry| entry.value().clone())
        else {
            return;
        };
        let result = match chosen {
            Some(title) => serde_json::json!({ "title": title }),
            None => serde_json::Value::Null,
        };
        let response = JsonRpcMessage::response(request.id.clone(), result);
        if let Err(error) = server.send_response(response).await {
            lsp_error!(
                "LSP-SERVER-REQUEST",
                "Failed to send showMessageRequest response: {}",
                error
            );
        }
    }
}
