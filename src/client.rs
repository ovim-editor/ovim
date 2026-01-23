//! HTTP client for communicating with ovim sessions
//!
//! Client-side code that outputs to stderr for user feedback.
#![allow(clippy::print_stderr)]

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::api::{BufferInfo, ContextWindowInfo, CursorPosition, EditorSnapshot, HealthInfo, LspStatusInfo};
use crate::session::SessionInfo;

/// Client for making requests to an ovim session
pub struct OvimClient {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl OvimClient {
    /// Create a new client for the given session
    pub fn new(session: &SessionInfo) -> Self {
        Self {
            base_url: format!("http://127.0.0.1:{}", session.port),
            client: reqwest::blocking::Client::new(),
        }
    }

    /// Create a client from a port number directly
    pub fn from_port(port: u16) -> Self {
        Self {
            base_url: format!("http://127.0.0.1:{}", port),
            client: reqwest::blocking::Client::new(),
        }
    }

    /// Send keys to the session
    pub fn send_keys(&self, keys: &str) -> Result<()> {
        let response = self
            .client
            .post(format!("{}/keys", self.base_url))
            .json(&json!({ "keys": keys }))
            .send()
            .context("Failed to send request")?;

        if !response.status().is_success() {
            let error: Value = response.json().unwrap_or(json!({"error": "Unknown error"}));
            anyhow::bail!("Failed to send keys: {:?}", error);
        }

        Ok(())
    }

    /// Execute an ex command
    pub fn execute_command(&self, command: &str) -> Result<String> {
        let response = self
            .client
            .post(format!("{}/command", self.base_url))
            .json(&json!({ "command": command }))
            .send()
            .context("Failed to send request")?;

        if !response.status().is_success() {
            let error: Value = response.json().unwrap_or(json!({"error": "Unknown error"}));
            anyhow::bail!("Failed to execute command: {:?}", error);
        }

        let result: Value = response.json()?;

        // Check for error response first
        if let Some(error) = result.get("error").and_then(|v| v.as_str()) {
            anyhow::bail!("{}", error);
        }

        Ok(result
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Success")
            .to_string())
    }

    /// Set the editor mode
    pub fn set_mode(&self, mode: &str) -> Result<()> {
        let response = self
            .client
            .post(format!("{}/mode", self.base_url))
            .json(&json!({ "mode": mode }))
            .send()
            .context("Failed to send request")?;

        if !response.status().is_success() {
            let error: Value = response.json().unwrap_or(json!({"error": "Unknown error"}));
            anyhow::bail!("Failed to set mode: {:?}", error);
        }

        Ok(())
    }

    /// Get the editor snapshot
    pub fn get_snapshot(&self) -> Result<EditorSnapshot> {
        let response = self
            .client
            .get(format!("{}/snapshot", self.base_url))
            .send()
            .context("Failed to send request")?;

        if !response.status().is_success() {
            let error: Value = response.json().unwrap_or(json!({"error": "Unknown error"}));
            anyhow::bail!("Failed to get snapshot: {:?}", error);
        }

        Ok(response.json()?)
    }

    /// Get buffer content
    pub fn get_buffer(&self) -> Result<BufferInfo> {
        let response = self
            .client
            .get(format!("{}/buffer", self.base_url))
            .send()
            .context("Failed to send request")?;

        if !response.status().is_success() {
            let error: Value = response.json().unwrap_or(json!({"error": "Unknown error"}));
            anyhow::bail!("Failed to get buffer: {:?}", error);
        }

        Ok(response.json()?)
    }

    /// Set buffer content
    pub fn set_buffer(&self, content: &str) -> Result<()> {
        let response = self
            .client
            .put(format!("{}/buffer", self.base_url))
            .json(&json!({ "content": content }))
            .send()
            .context("Failed to send request")?;

        if !response.status().is_success() {
            let error: Value = response.json().unwrap_or(json!({"error": "Unknown error"}));
            anyhow::bail!("Failed to set buffer: {:?}", error);
        }

        Ok(())
    }

    /// Get cursor position
    pub fn get_cursor(&self) -> Result<CursorPosition> {
        let response = self
            .client
            .get(format!("{}/cursor", self.base_url))
            .send()
            .context("Failed to send request")?;

        if !response.status().is_success() {
            let error: Value = response.json().unwrap_or(json!({"error": "Unknown error"}));
            anyhow::bail!("Failed to get cursor: {:?}", error);
        }

        Ok(response.json()?)
    }

    /// Get health status
    pub fn get_health(&self) -> Result<HealthInfo> {
        let response = self
            .client
            .get(format!("{}/health", self.base_url))
            .send()
            .context("Failed to send request")?;

        if !response.status().is_success() {
            let error: Value = response.json().unwrap_or(json!({"error": "Unknown error"}));
            anyhow::bail!("Failed to get health: {:?}", error);
        }

        Ok(response.json()?)
    }

    /// Get LSP status
    pub fn get_lsp_status(&self) -> Result<LspStatusInfo> {
        let response = self
            .client
            .get(format!("{}/lsp/status", self.base_url))
            .send()
            .context("Failed to send request")?;

        if !response.status().is_success() {
            let error: Value = response.json().unwrap_or(json!({"error": "Unknown error"}));
            anyhow::bail!("Failed to get LSP status: {:?}", error);
        }

        Ok(response.json()?)
    }

    /// Get context window (21-line view around cursor)
    pub fn get_context_window(&self) -> Result<ContextWindowInfo> {
        let response = self
            .client
            .post(format!("{}/mcp", self.base_url))
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "get_context_window",
                    "arguments": {}
                }
            }))
            .send()
            .context("Failed to send request")?;

        if !response.status().is_success() {
            let error: Value = response.json().unwrap_or(json!({"error": "Unknown error"}));
            anyhow::bail!("Failed to get context window: {:?}", error);
        }

        // Parse the MCP response to extract the tool result
        let mcp_response: Value = response.json()?;
        if let Some(result) = mcp_response.get("result").and_then(|r| r.get("content")) {
            if let Some(text_obj) = result.get(0).and_then(|c| c.get("text")) {
                if let Some(text) = text_obj.as_str() {
                    return Ok(ContextWindowInfo {
                        context: text.to_string(),
                        file: None,
                        mode: String::new(),
                        line: 0,
                        column: 0,
                    });
                }
            }
        }

        anyhow::bail!("Failed to parse context window response")
    }

    /// Send MCP JSON-RPC request
    pub fn send_mcp_request(&self, method: &str, params: Value, id: i64) -> Result<Value> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        let response = self
            .client
            .post(format!("{}/mcp", self.base_url))
            .json(&request)
            .send()
            .context("Failed to send MCP request")?;

        if !response.status().is_success() {
            let error: Value = response.json().unwrap_or(json!({"error": "Unknown error"}));
            anyhow::bail!("Failed to send MCP request: {:?}", error);
        }

        let result: Value = response.json()?;

        // Check for JSON-RPC error
        if let Some(error) = result.get("error") {
            anyhow::bail!("MCP error: {}", serde_json::to_string_pretty(error)?);
        }

        Ok(result)
    }

    /// Kill the session (by sending SIGTERM to the process)
    #[cfg(unix)]
    pub fn kill_session(&self, session: &SessionInfo) -> Result<()> {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;

        kill(Pid::from_raw(session.pid as i32), Signal::SIGTERM)
            .context("Failed to kill process")?;

        // Wait a bit for graceful shutdown
        std::thread::sleep(std::time::Duration::from_millis(500));

        // If still running, send SIGKILL
        if kill(Pid::from_raw(session.pid as i32), Signal::SIGKILL).is_ok() {
            eprintln!("Process did not exit gracefully, sent SIGKILL");
        }

        // Clean up session file
        session.delete()?;

        Ok(())
    }

    #[cfg(not(unix))]
    pub fn kill_session(&self, session: &SessionInfo) -> Result<()> {
        anyhow::bail!("Kill not implemented for this platform")
    }
}
