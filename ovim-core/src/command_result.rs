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

// ---- Convenience constructors ----

/// Shorthand for a successful command result with an optional message.
pub fn ok(message: impl Into<String>) -> CommandResult {
    CommandResult::Success(SuccessResponse {
        success: true,
        message: Some(message.into()),
        line_count: None,
    })
}

/// Shorthand for a successful command result with no message.
pub fn ok_silent() -> CommandResult {
    CommandResult::Success(SuccessResponse {
        success: true,
        message: None,
        line_count: None,
    })
}

/// Shorthand for an error command result.
pub fn err(message: impl Into<String>) -> CommandResult {
    CommandResult::Error(ErrorResponse {
        error: message.into(),
    })
}
