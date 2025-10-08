//! Daemon communication protocol
//!
//! Defines request/response types for client-daemon communication over Unix socket.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// LSP Position (line, character)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

/// LSP Range (start, end)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

/// Request from client to daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonRequest {
    /// Check if daemon is alive and responsive
    Ping,

    /// Initialize LSP for a document
    Initialize {
        workspace_root: String,
        language_id: String,
    },

    /// Open a document
    DidOpen {
        uri: String,
        language_id: String,
        version: i32,
        text: String,
    },

    /// Document content changed
    DidChange {
        uri: String,
        version: i32,
        text: String,
    },

    /// Document saved
    DidSave {
        uri: String,
        text: Option<String>,
    },

    /// Close document
    DidClose {
        uri: String,
    },

    /// Get hover information
    Hover {
        uri: String,
        position: LspPosition,
    },

    /// Go to definition
    GotoDefinition {
        uri: String,
        position: LspPosition,
    },

    /// Request completion
    Completion {
        uri: String,
        position: LspPosition,
    },

    /// Format document
    FormatDocument {
        uri: String,
    },

    /// Get code actions
    CodeActions {
        uri: String,
        range: LspRange,
    },

    /// Get diagnostics for document
    GetDiagnostics {
        uri: String,
    },

    /// Shutdown daemon gracefully
    Shutdown,
}

/// Response from daemon to client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonResponse {
    /// Pong response to Ping
    Pong,

    /// Request succeeded with no data
    Ok,

    /// Hover information
    Hover {
        content: String,
    },

    /// Definition location
    Definition {
        uri: String,
        line: u32,
        character: u32,
    },

    /// Completion items
    Completion {
        items: Vec<CompletionItem>,
    },

    /// Text edits for formatting
    FormatEdits {
        edits: Vec<TextEdit>,
    },

    /// Code action options
    CodeActions {
        actions: Vec<CodeAction>,
    },

    /// Diagnostics for document
    Diagnostics {
        diagnostics: Vec<Diagnostic>,
    },

    /// Request failed with error
    Error {
        message: String,
    },
}

/// Completion item from LSP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionItem {
    pub label: String,
    pub kind: Option<CompletionItemKind>,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    pub insert_text: Option<String>,
}

/// Completion item kind
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CompletionItemKind {
    Text,
    Method,
    Function,
    Constructor,
    Field,
    Variable,
    Class,
    Interface,
    Module,
    Property,
    Unit,
    Value,
    Enum,
    Keyword,
    Snippet,
    Color,
    File,
    Reference,
    Folder,
    EnumMember,
    Constant,
    Struct,
    Event,
    Operator,
    TypeParameter,
}

/// Text edit from LSP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextEdit {
    pub range: LspRange,
    pub new_text: String,
}

/// Code action from LSP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeAction {
    pub title: String,
    pub kind: Option<String>,
    pub diagnostics: Option<Vec<Diagnostic>>,
    pub edit: Option<WorkspaceEdit>,
}

/// Workspace edit
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceEdit {
    pub changes: HashMap<String, Vec<TextEdit>>,
}

/// Diagnostic from LSP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub range: LspRange,
    pub severity: Option<DiagnosticSeverity>,
    pub message: String,
    pub source: Option<String>,
}

/// Diagnostic severity
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

impl DaemonRequest {
    /// Serialize to JSON bytes for sending over socket
    pub fn to_bytes(&self) -> anyhow::Result<Vec<u8>> {
        let json = serde_json::to_vec(self)?;

        // Prepend length as 4-byte big-endian integer
        let len = json.len() as u32;
        let mut bytes = len.to_be_bytes().to_vec();
        bytes.extend(json);

        Ok(bytes)
    }

    /// Deserialize from JSON bytes received from socket
    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        Ok(serde_json::from_slice(bytes)?)
    }
}

impl DaemonResponse {
    /// Serialize to JSON bytes for sending over socket
    pub fn to_bytes(&self) -> anyhow::Result<Vec<u8>> {
        let json = serde_json::to_vec(self)?;

        // Prepend length as 4-byte big-endian integer
        let len = json.len() as u32;
        let mut bytes = len.to_be_bytes().to_vec();
        bytes.extend(json);

        Ok(bytes)
    }

    /// Deserialize from JSON bytes received from socket
    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        Ok(serde_json::from_slice(bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialization() {
        let request = DaemonRequest::Ping;
        let bytes = request.to_bytes().unwrap();

        // First 4 bytes are length, rest is JSON
        assert!(bytes.len() > 4);

        // Extract length from first 4 bytes
        let len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        assert_eq!(len, bytes.len() - 4);

        // Skip length prefix and deserialize
        let deserialized = DaemonRequest::from_bytes(&bytes[4..]).unwrap();

        match deserialized {
            DaemonRequest::Ping => (),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_response_serialization() {
        let response = DaemonResponse::Pong;
        let bytes = response.to_bytes().unwrap();

        let deserialized = DaemonResponse::from_bytes(&bytes[4..]).unwrap();

        match deserialized {
            DaemonResponse::Pong => (),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_hover_request() {
        let request = DaemonRequest::Hover {
            uri: "file:///test.java".to_string(),
            position: LspPosition { line: 10, character: 5 },
        };

        let bytes = request.to_bytes().unwrap();
        let deserialized = DaemonRequest::from_bytes(&bytes[4..]).unwrap();

        match deserialized {
            DaemonRequest::Hover { uri, position } => {
                assert_eq!(uri, "file:///test.java");
                assert_eq!(position.line, 10);
                assert_eq!(position.character, 5);
            }
            _ => panic!("Wrong variant"),
        }
    }
}
