//! Command execution result types.
//!
//! Used by the editor's command execution layer (ex commands like `:w`, `:q`, etc.)
//! to return success/error information.

use serde::Serialize;

/// Result of executing an ex command.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum CommandResult {
    Success(SuccessResponse),
    Error(ErrorResponse),
}

/// Success response from a command.
#[derive(Debug, Clone, Serialize)]
pub struct SuccessResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_count: Option<usize>,
}

/// Error response from a command.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}
