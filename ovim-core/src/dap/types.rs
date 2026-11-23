//! Client-side DAP type definitions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DapSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DapSourceBreakpoint {
    pub line: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hit_condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DapBreakpoint {
    #[serde(default)]
    pub id: Option<u64>,
    pub verified: bool,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub line: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DapThread {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DapStackFrame {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub source: Option<DapSourceResponse>,
    #[serde(default)]
    pub line: u64,
    #[serde(default)]
    pub column: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DapSourceResponse {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DapScope {
    pub name: String,
    pub variables_reference: u64,
    #[serde(default)]
    pub expensive: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DapVariable {
    pub name: String,
    pub value: String,
    #[serde(default, rename = "type")]
    pub type_: Option<String>,
    #[serde(default)]
    pub variables_reference: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DapCapabilities {
    #[serde(default)]
    pub supports_configuration_done_request: bool,
    #[serde(default)]
    pub supports_set_variable: bool,
    #[serde(default)]
    pub supports_conditional_breakpoints: bool,
    #[serde(default)]
    pub supports_terminate_request: bool,
    #[serde(default)]
    pub supports_log_points: bool,
    #[serde(default)]
    pub supports_hit_conditional_breakpoints: bool,
    /// Checkboxes the client may show for "break on exceptions".
    #[serde(default)]
    pub exception_breakpoint_filters: Vec<DapExceptionFilter>,
}

/// One entry of `exceptionBreakpointFilters` (e.g. "all", "uncaught").
#[derive(Debug, Clone, Deserialize)]
pub struct DapExceptionFilter {
    pub filter: String,
    pub label: String,
    #[serde(default)]
    pub default: Option<bool>,
}

/// Answer to `exceptionInfo`: what the debuggee threw.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DapExceptionInfo {
    #[serde(default)]
    pub exception_id: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub details: Option<DapExceptionDetails>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DapExceptionDetails {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub type_name: Option<String>,
    #[serde(default)]
    pub full_type_name: Option<String>,
}

impl DapExceptionInfo {
    /// `Type: message`, the way it is shown to the user.
    pub fn summary(&self) -> String {
        if let Some(description) = self.description.as_deref().filter(|d| !d.is_empty()) {
            return description.to_string();
        }
        let details = self.details.as_ref();
        let type_name = details
            .and_then(|d| d.full_type_name.as_deref().or(d.type_name.as_deref()))
            .filter(|t| !t.is_empty())
            .unwrap_or(&self.exception_id);
        match details.and_then(|d| d.message.as_deref()) {
            Some(message) if !message.is_empty() => format!("{type_name}: {message}"),
            _ => type_name.to_string(),
        }
    }
}
