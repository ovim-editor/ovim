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

    /// Process start time (system boot time + ticks) for PID verification
    /// This prevents PID reuse race conditions
    pub start_time: Option<u64>,
}

impl SessionInfo {
    /// Create a new session info
    pub fn new(port: u16, file: Option<String>, session_name: String) -> Self {
        let started_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let pid = process::id();
        let start_time = get_process_start_time(pid);

        Self {
            pid,
            port,
            file,
            started_at,
            session_name,
            lsp_ready: false,
            start_time,
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

        // Check if process is still running and matches start time
        if !is_process_alive_with_start_time(info.pid, info.start_time) {
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
                    // Only include if process is still alive and matches start time
                    if is_process_alive_with_start_time(info.pid, info.start_time) {
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

/// Get the process start time for a given PID
/// Returns the start time in a platform-specific format that can be compared for equality
#[cfg(target_os = "linux")]
fn get_process_start_time(pid: u32) -> Option<u64> {
    use std::fs;

    // Read /proc/[pid]/stat
    let stat_path = format!("/proc/{}/stat", pid);
    let stat_content = fs::read_to_string(stat_path).ok()?;

    // The stat file format is: pid (comm) state ppid ... starttime ...
    // We need to handle the comm field which can contain spaces and parentheses
    // Find the last ')' to skip the comm field
    let start_pos = stat_content.rfind(')')?;
    let fields: Vec<&str> = stat_content[start_pos + 1..].split_whitespace().collect();

    // starttime is the 22nd field overall, but we've skipped pid and comm
    // After ')' we have: state ppid pgrp session tty_nr tpgid flags ...
    // starttime is at index 19 (0-indexed) after the ')'
    if fields.len() > 19 {
        fields[19].parse::<u64>().ok()
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn get_process_start_time(pid: u32) -> Option<u64> {
    use std::process::Command;

    // Use ps to get the process start time in seconds since epoch
    // The 'lstart' format gives us the full start time, but we'll use 'etime' or parse differently
    // Actually, we'll use sysctl kern.proc.pid which gives us more reliable data

    // Alternative: use `ps -o lstart= -p PID` and parse it
    // But better: use the start time in seconds
    let output = Command::new("ps")
        .args(&["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let start_str = String::from_utf8(output.stdout).ok()?;
    let start_str = start_str.trim();

    // Parse the date string (format: "Tue Oct 20 10:30:45 2025")
    // We'll convert this to epoch seconds
    // For simplicity and robustness, we'll use a different approach:
    // Get the elapsed time and subtract from current time

    let output = Command::new("ps")
        .args(&["-o", "etimes=", "-p", &pid.to_string()])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let elapsed_str = String::from_utf8(output.stdout).ok()?;
    let elapsed_secs: u64 = elapsed_str.trim().parse().ok()?;

    // Calculate start time = current time - elapsed time
    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();

    Some(current_time.saturating_sub(elapsed_secs))
}

#[cfg(target_os = "windows")]
fn get_process_start_time(pid: u32) -> Option<u64> {
    use sysinfo::{Pid, System};

    let mut system = System::new();
    system.refresh_processes();

    let process = system.process(Pid::from_u32(pid))?;
    Some(process.start_time())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn get_process_start_time(_pid: u32) -> Option<u64> {
    // On unsupported platforms, return None
    // This will fall back to PID-only checking (less safe but better than nothing)
    None
}

/// Check if a process is still alive and matches the expected start time
/// This prevents PID reuse race conditions
fn is_process_alive_with_start_time(pid: u32, expected_start_time: Option<u64>) -> bool {
    // First check if process exists
    if !is_process_exists(pid) {
        return false;
    }

    // If we have an expected start time, verify it matches
    if let Some(expected) = expected_start_time {
        if let Some(actual) = get_process_start_time(pid) {
            // Allow small variance (1-2 seconds) due to timing differences
            // between when we captured the start time and when the process actually started
            let diff = if actual > expected {
                actual - expected
            } else {
                expected - actual
            };

            // If difference is more than 2 seconds, it's likely a different process
            if diff > 2 {
                return false;
            }
        } else {
            // If we can't get the start time but expected one, be conservative
            // and assume it might be a different process
            return false;
        }
    }

    true
}

/// Check if a process exists (without start time verification)
#[cfg(unix)]
fn is_process_exists(pid: u32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    // Signal 0 doesn't send a signal but checks if process exists
    kill(Pid::from_raw(pid as i32), None).is_ok()
}

#[cfg(windows)]
fn is_process_exists(pid: u32) -> bool {
    use sysinfo::{Pid, System};

    let mut system = System::new();
    system.refresh_processes();
    system.process(Pid::from_u32(pid)).is_some()
}

#[cfg(not(any(unix, windows)))]
fn is_process_exists(_pid: u32) -> bool {
    // On other systems, conservatively assume process is alive
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
                if !is_process_alive_with_start_time(info.pid, info.start_time) {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }

    Ok(())
}
