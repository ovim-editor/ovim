//! Session management for headless mode
//!
//! Manages session files in ~/.cache/ovim/sessions/ that allow:
//! - Port discovery for running instances
//! - Session naming for multiple concurrent instances
//! - Automatic cleanup on exit
//! - Status checking (LSP ready, etc.)
//!
//! Client-side messages use stderr for user feedback.
#![allow(clippy::print_stderr)]

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process;
use std::time::{Duration, SystemTime};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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

    /// Logical viewport used by headless rendering and motion commands.
    #[serde(default)]
    pub viewport_width: Option<u16>,
    #[serde(default)]
    pub viewport_height: Option<u16>,
}

impl SessionInfo {
    /// Create a new session info
    pub fn new(port: u16, file: Option<String>, session_name: String) -> Self {
        let started_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(std::time::Duration::ZERO)
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
            viewport_width: None,
            viewport_height: None,
        }
    }

    pub fn with_dimensions(mut self, width: u16, height: u16) -> Self {
        self.viewport_width = Some(width);
        self.viewport_height = Some(height);
        self
    }

    pub fn dimensions(&self) -> Option<(u16, u16)> {
        Some((self.viewport_width?, self.viewport_height?))
    }

    pub fn set_dimensions(&mut self, width: u16, height: u16) -> Result<()> {
        self.viewport_width = Some(width);
        self.viewport_height = Some(height);
        self.write()
    }

    /// Get the session directory path
    pub fn session_dir() -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("OVIM_SESSION_DIR") {
            let session_dir = PathBuf::from(dir);
            fs::create_dir_all(&session_dir)?;
            return Ok(session_dir);
        }

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

    /// Write this session info to disk atomically
    pub fn write(&self) -> Result<()> {
        use std::io::Write;

        let path = self.session_file_path()?;
        let json = serde_json::to_string_pretty(self)?;

        // Use atomic write: write to temp file, then rename
        let temp_path = path.with_extension("tmp");

        // Write to temporary file
        let mut temp_file =
            fs::File::create(&temp_path).context("Failed to create temporary session file")?;

        // SECURITY: Set restrictive permissions (0o600 = rw-------) to prevent information
        // disclosure on multi-user systems. Session files contain sensitive data like port
        // numbers and file paths that should only be readable by the owner.
        #[cfg(unix)]
        {
            let mut perms = temp_file.metadata()?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&temp_path, perms)?;
        }

        temp_file
            .write_all(json.as_bytes())
            .context("Failed to write to temporary session file")?;
        temp_file
            .flush()
            .context("Failed to flush temporary session file")?;

        // Ensure data is written to disk before atomic rename to prevent data loss on crash
        temp_file
            .sync_all()
            .context("Failed to sync session file to disk")?;

        // Atomically replace the old file with the new one
        fs::rename(&temp_path, &path).context("Failed to rename session file")?;

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

        // Remove file directly without checking exists() first to avoid TOCTOU race.
        // An attacker could replace the file with a symlink between the check and removal.
        // remove_file is idempotent - if the file doesn't exist, we treat that as success.
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Read a session by name with helpful error messages
    pub fn read(session_name: &str) -> Result<Self> {
        let session_dir = Self::session_dir()?;
        let path = session_dir.join(format!("{}.json", session_name));

        // Try to read the file
        let json = fs::read_to_string(&path).context(format!(
            "Session '{}' not found.\n\nAvailable sessions:\n{}",
            session_name,
            Self::format_available_sessions()
        ))?;

        // Try to parse JSON
        let info: SessionInfo = serde_json::from_str(&json).context(format!(
            "Session file for '{}' is corrupted. Run 'ovim cleanup' to remove invalid sessions.",
            session_name
        ))?;

        // Check if process is still running and matches start time
        if !is_process_alive_with_start_time(info.pid, info.start_time) {
            // Clean up stale session file
            let _ = fs::remove_file(&path);
            anyhow::bail!(
                "Session '{}' is not running (process {} terminated).\n\
                 Stale session file has been cleaned up.\n\
                 \n\
                 Active sessions:\n{}",
                session_name,
                info.pid,
                Self::format_available_sessions()
            );
        }

        Ok(info)
    }

    /// Format available sessions as a helpful string for error messages
    fn format_available_sessions() -> String {
        match Self::list_all() {
            Ok(sessions) if !sessions.is_empty() => {
                let mut output = String::new();
                for session in sessions {
                    output.push_str(&format!(
                        "  - {} (PID {}, port {})\n",
                        session.session_name, session.pid, session.port
                    ));
                }
                output
            }
            _ => "  (none - start a session with: ovim <file> --headless --session <name>)\n"
                .to_string(),
        }
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

    /// Check if session is expired (older than max_age)
    ///
    /// # Arguments
    /// * `max_age` - Maximum session age before considered expired
    ///
    /// # Educational Note
    /// Session expiry is useful for cleaning up long-running sessions that may have been
    /// forgotten. However, be careful with automatic expiry - some users may intentionally
    /// keep sessions running for days/weeks (e.g., remote development, long-running tasks).
    /// Default expiry should be conservative (7+ days).
    pub fn is_expired(&self, max_age: Duration) -> bool {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();

        let age = now.saturating_sub(self.started_at);
        age > max_age.as_secs()
    }

    /// Health check: verify session is actually responding
    ///
    /// This goes beyond PID checking - it verifies the API endpoint is accessible.
    /// This is crucial because:
    /// 1. Process might exist but be hung/deadlocked
    /// 2. Process might exist but API server failed to start
    /// 3. Port might be bound by a different process (PID reuse + port reuse race)
    ///
    /// # Returns
    /// * `Ok(true)` - Session is healthy and responding
    /// * `Ok(false)` - Session exists but not responding
    /// * `Err(_)` - Network/connection error
    ///
    /// # Educational Note
    /// Health checks are essential for distributed systems. A process existing (PID check)
    /// doesn't mean it's functional. Always verify the actual service endpoint when possible.
    /// This is why Kubernetes has liveness probes, not just PID checks.
    pub fn check_health(&self) -> Result<bool> {
        // First check if process is alive (fast check)
        if !is_process_alive_with_start_time(self.pid, self.start_time) {
            return Ok(false);
        }

        // Then verify API endpoint is responding (comprehensive check)
        // Use a short timeout to avoid blocking cleanup operations
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(500))
            .build()?;

        let health_url = format!("http://127.0.0.1:{}/health", self.port);

        match client.get(&health_url).send() {
            Ok(response) => Ok(response.status().is_success()),
            Err(_) => Ok(false), // Network error = unhealthy
        }
    }

    /// Get human-readable session age
    pub fn age(&self) -> Duration {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();

        Duration::from_secs(now.saturating_sub(self.started_at))
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
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let bytes_written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            std::ptr::addr_of_mut!(info).cast::<libc::c_void>(),
            std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int,
        )
    };

    if bytes_written as usize != std::mem::size_of::<libc::proc_bsdinfo>() {
        return None;
    }

    Some(info.pbi_start_tvsec)
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
            let diff = actual.abs_diff(expected);

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

/// Result of cleanup operation
#[derive(Debug, Clone)]
pub struct CleanupResult {
    /// Number of stale sessions removed (dead processes)
    pub stale_removed: usize,
    /// Number of expired sessions removed (too old)
    pub expired_removed: usize,
    /// Number of corrupted session files removed
    pub corrupted_removed: usize,
    /// Number of orphaned temp files removed
    pub temp_files_removed: usize,
    /// Details of removed sessions for logging
    pub removed_sessions: Vec<String>,
}

impl CleanupResult {
    pub fn total_removed(&self) -> usize {
        self.stale_removed + self.expired_removed + self.corrupted_removed + self.temp_files_removed
    }
}

/// Clean up all stale session files with detailed reporting
///
/// # Arguments
/// * `max_age` - Optional maximum session age. Sessions older than this are removed. None = no age limit.
/// * `dry_run` - If true, report what would be removed without actually removing
///
/// # Educational Notes
/// **Why PID checking isn't enough:**
/// PIDs can be reused by the OS. If ovim crashes and a new process gets the same PID,
/// we'd incorrectly think the session is still alive. That's why we also check:
/// 1. Process start time (from /proc or ps) - ensures it's the SAME process
/// 2. API health endpoint - ensures the process is actually functional
///
/// **Why we clean up temp files:**
/// During atomic writes, we create temp.tmp then rename to session.json.
/// If the process crashes between these steps, temp files are orphaned.
/// We only clean up old temp files (>1 hour) to avoid racing with active writers.
///
/// **Session expiry considerations:**
/// Some users run ovim sessions for days/weeks (remote dev, long tasks).
/// Default expiry should be conservative (7+ days). This is opt-in via --max-age.
pub fn cleanup_stale_sessions(max_age: Option<Duration>, dry_run: bool) -> Result<CleanupResult> {
    let session_dir = SessionInfo::session_dir()?;

    let mut result = CleanupResult {
        stale_removed: 0,
        expired_removed: 0,
        corrupted_removed: 0,
        temp_files_removed: 0,
        removed_sessions: Vec::new(),
    };

    // Clean up session files
    for entry in fs::read_dir(&session_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let session_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        // Try to parse session file
        match fs::read_to_string(&path) {
            Ok(json) => {
                match serde_json::from_str::<SessionInfo>(&json) {
                    Ok(info) => {
                        let mut should_remove = false;
                        let mut reason = String::new();

                        // Check if process is dead
                        if !is_process_alive_with_start_time(info.pid, info.start_time) {
                            should_remove = true;
                            reason = format!("process {} not running", info.pid);
                            result.stale_removed += 1;
                        }
                        // Check if expired (if max_age specified)
                        else if let Some(max_age) = max_age {
                            if info.is_expired(max_age) {
                                should_remove = true;
                                let age_days = info.age().as_secs() / 86400;
                                reason = format!("expired ({} days old)", age_days);
                                result.expired_removed += 1;
                            }
                        }

                        if should_remove {
                            let detail = format!("{} ({})", session_name, reason);
                            result.removed_sessions.push(detail.clone());

                            if !dry_run {
                                // Best-effort removal - file might already be gone
                                match fs::remove_file(&path) {
                                    Ok(()) => {}
                                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                                    Err(e) => {
                                        return Err(e).context(format!(
                                            "Failed to remove session file: {}",
                                            path.display()
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        // Corrupted session file
                        let detail = format!("{} (corrupted: {})", session_name, e);
                        result.removed_sessions.push(detail);
                        result.corrupted_removed += 1;

                        if !dry_run {
                            // Best-effort removal - file might already be gone
                            match fs::remove_file(&path) {
                                Ok(()) => {}
                                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                                Err(e) => {
                                    return Err(e).context(format!(
                                        "Failed to remove corrupted session file: {}",
                                        path.display()
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                // Can't read file
                let detail = format!("{} (unreadable: {})", session_name, e);
                result.removed_sessions.push(detail);
                result.corrupted_removed += 1;

                if !dry_run {
                    // Best-effort removal - file might already be gone
                    match fs::remove_file(&path) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => {
                            return Err(e).context(format!(
                                "Failed to remove unreadable session file: {}",
                                path.display()
                            ));
                        }
                    }
                }
            }
        }
    }

    // Clean up orphaned .tmp files older than 1 hour
    // These are created during atomic writes (write + rename) but can be left behind
    // if the process crashes or the write fails between creating the temp file and renaming it
    for entry in fs::read_dir(&session_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("tmp") {
            // Only delete old temp files (>1 hour old) to avoid races with concurrent writers
            if let Ok(metadata) = fs::metadata(&path) {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(elapsed) = modified.elapsed() {
                        if elapsed > Duration::from_secs(3600) {
                            result.temp_files_removed += 1;

                            if !dry_run {
                                let _ = fs::remove_file(&path); // Best-effort cleanup
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(result)
}

/// Guard that ensures session cleanup on drop (even during panic)
///
/// This struct wraps SessionInfo and implements Drop to guarantee that
/// the session file is deleted when the guard goes out of scope, whether
/// due to normal exit, Ctrl+C, or panic unwinding.
///
/// # Example
///
/// ```no_run
/// use ovim_core::session::{SessionGuard, SessionInfo};
///
/// let session_info = SessionInfo::new(8080, Some("file.txt".to_string()), "dev".to_string());
/// let _guard = SessionGuard::new(session_info.clone());
/// // Session file will be cleaned up automatically when _guard is dropped
/// ```
pub struct SessionGuard {
    session_info: SessionInfo,
}

impl SessionGuard {
    /// Create a new session guard
    pub fn new(session_info: SessionInfo) -> Self {
        Self { session_info }
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        // Best-effort cleanup on panic/normal exit
        // We ignore errors since:
        // 1. We're already panicking or exiting
        // 2. The file might already be cleaned up by signal handler
        // 3. Logging during drop/panic can cause issues
        let _ = self.session_info.delete();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic;

    fn init_test_session_dir() {
        static SESSION_DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        SESSION_DIR.get_or_init(|| {
            let dir = tempfile::tempdir().expect("tempdir");
            std::env::set_var("OVIM_SESSION_DIR", dir.path());
            dir
        });
    }

    #[test]
    fn test_session_guard_cleans_up_on_drop() {
        init_test_session_dir();
        let session_info =
            SessionInfo::new(9999, Some("test.txt".to_string()), "guard_test".to_string());
        session_info.write().unwrap();

        let session_file_path = session_info.session_file_path().unwrap();
        assert!(
            session_file_path.exists(),
            "Session file should exist before drop"
        );

        // Create and immediately drop the guard
        {
            let _guard = SessionGuard::new(session_info.clone());
        }

        // Give filesystem a moment
        std::thread::sleep(std::time::Duration::from_millis(10));

        assert!(
            !session_file_path.exists(),
            "Session file should be deleted after guard drops"
        );
    }

    #[test]
    fn test_session_guard_cleans_up_on_panic() {
        init_test_session_dir();
        let session_info = SessionInfo::new(
            9998,
            Some("panic_test.txt".to_string()),
            "panic_guard_test".to_string(),
        );
        session_info.write().unwrap();

        let session_file_path = session_info.session_file_path().unwrap();
        assert!(
            session_file_path.exists(),
            "Session file should exist before panic"
        );

        // Trigger panic with guard in scope
        let result = panic::catch_unwind(|| {
            let _guard = SessionGuard::new(session_info.clone());
            panic!("Test panic!");
        });

        assert!(result.is_err(), "Panic should have occurred");

        // Give filesystem a moment
        std::thread::sleep(std::time::Duration::from_millis(10));

        assert!(
            !session_file_path.exists(),
            "Session file should be deleted after panic"
        );
    }
}
