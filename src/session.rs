//! Session management for headless mode
//!
//! Manages session files in ~/.cache/ovim/sessions/ that allow:
//! - Port discovery for running instances
//! - Session naming for multiple concurrent instances
//! - Automatic cleanup on exit
//! - Status checking (LSP ready, etc.)

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process;
use std::time::SystemTime;

/// Information about a running headless session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Process ID of the ovim instance
    pub pid: u32,

    /// Port the API server is listening on
    pub port: u16,

    /// File being edited
    pub file: Option<String>,

    /// When the session was started (Unix timestamp)
    pub started_at: u64,

    /// Session name (default: "default")
    pub session_name: String,

    /// Whether LSP is ready
    pub lsp_ready: bool,
}

impl SessionInfo {
    /// Create a new session info
    pub fn new(port: u16, file: Option<String>, session_name: String) -> Self {
        let started_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            pid: process::id(),
            port,
            file,
            started_at,
            session_name,
            lsp_ready: false,
        }
    }

    /// Get the session directory path
    pub fn session_dir() -> Result<PathBuf> {
        let cache_dir = dirs::cache_dir().context("Failed to get cache directory")?;

        let session_dir = cache_dir.join("ovim").join("sessions");
        fs::create_dir_all(&session_dir)?;

        Ok(session_dir)
    }

    /// Get the path for this session's file
    pub fn session_file_path(&self) -> Result<PathBuf> {
        let session_dir = Self::session_dir()?;
        Ok(session_dir.join(format!("{}.json", self.session_name)))
    }

    /// Write this session info to disk
    pub fn write(&self) -> Result<()> {
        let path = self.session_file_path()?;
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&path, json)?;
        Ok(())
    }

    /// Update LSP ready status
    pub fn set_lsp_ready(&mut self, ready: bool) -> Result<()> {
        self.lsp_ready = ready;
        self.write()
    }

    /// Delete this session file
    pub fn delete(&self) -> Result<()> {
        let path = self.session_file_path()?;
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Read a session by name
    pub fn read(session_name: &str) -> Result<Self> {
        let session_dir = Self::session_dir()?;
        let path = session_dir.join(format!("{}.json", session_name));

        let json =
            fs::read_to_string(&path).context(format!("Session '{}' not found", session_name))?;

        let info: SessionInfo = serde_json::from_str(&json)?;

        // Check if process is still running
        if !is_process_alive(info.pid) {
            // Clean up stale session file
            let _ = fs::remove_file(&path);
            anyhow::bail!(
                "Session '{}' is not running (stale file cleaned up)",
                session_name
            );
        }

        Ok(info)
    }

    /// List all active sessions
    pub fn list_all() -> Result<Vec<SessionInfo>> {
        let session_dir = Self::session_dir()?;
        let mut sessions = Vec::new();

        for entry in fs::read_dir(session_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }

            if let Ok(json) = fs::read_to_string(&path) {
                if let Ok(info) = serde_json::from_str::<SessionInfo>(&json) {
                    // Only include if process is still alive
                    if is_process_alive(info.pid) {
                        sessions.push(info);
                    } else {
                        // Clean up stale file
                        let _ = fs::remove_file(&path);
                    }
                }
            }
        }

        sessions.sort_by_key(|s| s.started_at);
        Ok(sessions)
    }

    /// Get the default session (most recently started)
    pub fn get_default() -> Result<Self> {
        let sessions = Self::list_all()?;
        sessions
            .into_iter()
            .max_by_key(|s| s.started_at)
            .context("No active sessions found")
    }
}

/// Check if a process is still alive
#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    // Signal 0 doesn't send a signal but checks if process exists
    kill(Pid::from_raw(pid as i32), Some(Signal::SIGUSR1)).is_ok()
        || kill(Pid::from_raw(pid as i32), None).is_ok()
}

#[cfg(windows)]
fn is_process_alive(pid: u32) -> bool {
    use sysinfo::{Pid, System};

    let mut system = System::new();
    system.refresh_processes();
    system.process(Pid::from_u32(pid)).is_some()
}

// Fallback for non-Windows, non-Unix systems (if any)
#[cfg(not(any(unix, windows)))]
fn is_process_alive(_pid: u32) -> bool {
    // On other systems, conservatively assume process is alive
    // to avoid accidentally deleting active session files
    true
}

/// Clean up all stale session files
pub fn cleanup_stale_sessions() -> Result<()> {
    let session_dir = SessionInfo::session_dir()?;

    for entry in fs::read_dir(session_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        if let Ok(json) = fs::read_to_string(&path) {
            if let Ok(info) = serde_json::from_str::<SessionInfo>(&json) {
                if !is_process_alive(info.pid) {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }

    Ok(())
}
