//! Process management and robust killing
//!
//! Handles edge cases like:
//! - Processes that won't die (SIGKILL doesn't work)
//! - Zombie processes
//! - Processes stuck in uninterruptible sleep (state D)

use anyhow::{Context, Result};
use std::time::Duration;

/// Result of attempting to kill a process
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessKillStatus {
    /// Process terminated cleanly with SIGTERM
    Terminated,

    /// Process killed with SIGKILL
    Killed,

    /// Process is zombie (already dead, waiting for parent)
    Zombie,

    /// Process stuck in uninterruptible sleep (state D)
    /// Cannot be killed!
    Stuck,
}

/// Try to kill a process, escalating through signals
///
/// Strategy:
/// 1. Send SIGTERM, wait up to 5 seconds
/// 2. If still alive, send SIGKILL, wait up to 2 seconds
/// 3. If still alive, check process state
/// 4. Return status based on final state
pub async fn kill_process_forcefully(pid: i32) -> Result<ProcessKillStatus> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    eprintln!("[daemon] Attempting to kill process {}", pid);

    // Check if process exists
    if !super::pid::process_exists(pid) {
        eprintln!("[daemon] Process {} already dead", pid);
        return Ok(ProcessKillStatus::Terminated);
    }

    // Try SIGTERM first (graceful shutdown)
    eprintln!("[daemon] Sending SIGTERM to {}", pid);
    if let Err(e) = kill(Pid::from_raw(pid), Signal::SIGTERM) {
        eprintln!("[daemon] Warning: SIGTERM failed: {}", e);
    }

    // Wait up to 5 seconds for graceful termination
    for i in 0..10 {
        tokio::time::sleep(Duration::from_millis(500)).await;

        if !super::pid::process_exists(pid) {
            eprintln!("[daemon] Process {} terminated gracefully", pid);
            return Ok(ProcessKillStatus::Terminated);
        }

        if i == 5 {
            eprintln!("[daemon] Process {} still alive after 2.5s", pid);
        }
    }

    // Still alive - escalate to SIGKILL
    eprintln!(
        "[daemon] Warning: Process {} did not respond to SIGTERM, sending SIGKILL",
        pid
    );
    if let Err(e) = kill(Pid::from_raw(pid), Signal::SIGKILL) {
        eprintln!("[daemon] Error: SIGKILL failed: {}", e);
    }

    // Wait up to 2 seconds for kill
    for i in 0..4 {
        tokio::time::sleep(Duration::from_millis(500)).await;

        if !super::pid::process_exists(pid) {
            eprintln!("[daemon] Process {} killed", pid);
            return Ok(ProcessKillStatus::Killed);
        }

        if i == 2 {
            eprintln!(
                "[daemon] Warning: Process {} still alive after SIGKILL",
                pid
            );
        }
    }

    // Still alive after SIGKILL - check state
    eprintln!(
        "[daemon] Error: Process {} survived SIGKILL - checking state",
        pid
    );

    match get_process_state(pid)? {
        'Z' => {
            eprintln!("[daemon] Warning: Process {} is zombie", pid);
            Ok(ProcessKillStatus::Zombie)
        }
        'D' => {
            eprintln!(
                "[daemon] Error: Process {} is in uninterruptible sleep (state D)",
                pid
            );
            eprintln!("[daemon] Error: This process cannot be killed! Likely waiting on disk I/O");
            Ok(ProcessKillStatus::Stuck)
        }
        state => {
            eprintln!(
                "[daemon] Error: Process {} in unexpected state: {}",
                pid, state
            );
            anyhow::bail!("Process {} survived SIGKILL and is in state {}", pid, state)
        }
    }
}

/// Get process state character from /proc/{pid}/stat
///
/// States:
/// - R: Running
/// - S: Sleeping (interruptible)
/// - D: Disk sleep (uninterruptible)
/// - Z: Zombie
/// - T: Stopped
/// - X: Dead
#[cfg(target_os = "linux")]
pub fn get_process_state(pid: i32) -> Result<char> {
    use std::fs;

    let stat =
        fs::read_to_string(format!("/proc/{}/stat", pid)).context("Failed to read /proc/stat")?;

    // Format: pid (comm) state ...
    // Find state after the comm field (which may contain spaces/parens)
    let start = stat.rfind(')').context("Invalid stat format")?;
    let after_comm = &stat[start + 1..];
    let state = after_comm
        .trim()
        .chars()
        .next()
        .context("No state character")?;

    Ok(state)
}

#[cfg(target_os = "macos")]
pub fn get_process_state(pid: i32) -> Result<char> {
    use std::process::Command;

    // Use ps to get process state
    let output = Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("state=")
        .output()
        .context("Failed to run ps")?;

    if !output.status.success() {
        anyhow::bail!("ps command failed");
    }

    let state_str = String::from_utf8_lossy(&output.stdout);
    let state = state_str
        .trim()
        .chars()
        .next()
        .context("No state character")?;

    // macOS uses different codes, normalize to Linux-style
    // I/S -> S (sleeping)
    // R -> R (running)
    // U -> D (uninterruptible wait)
    // Z -> Z (zombie)
    // T -> T (stopped)

    let normalized = match state {
        'I' | 'S' => 'S',
        'U' => 'D',
        other => other,
    };

    Ok(normalized)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn get_process_state(_pid: i32) -> Result<char> {
    anyhow::bail!("Process state not supported on this platform")
}

/// Track rogue processes that can't be killed
pub struct RogueProcessTracker {
    rogue_pids: std::sync::Arc<tokio::sync::Mutex<Vec<RogueProcess>>>,
}

#[derive(Debug, Clone)]
pub struct RogueProcess {
    pub pid: i32,
    pub detected_at: std::time::SystemTime,
    pub reason: String,
}

impl RogueProcessTracker {
    pub fn new() -> Self {
        Self {
            rogue_pids: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    /// Mark a process as rogue
    pub async fn mark_rogue(&self, pid: i32, reason: String) {
        eprintln!(
            "[daemon] Error: Marking process {} as rogue: {}",
            pid, reason
        );

        let mut rogues = self.rogue_pids.lock().await;
        rogues.push(RogueProcess {
            pid,
            detected_at: std::time::SystemTime::now(),
            reason,
        });

        // Keep only last 100 rogue processes
        if rogues.len() > 100 {
            rogues.remove(0);
        }
    }

    /// Check if a PID is marked as rogue
    pub async fn is_rogue(&self, pid: i32) -> bool {
        let rogues = self.rogue_pids.lock().await;
        rogues.iter().any(|r| r.pid == pid)
    }

    /// Clean up rogue processes that have died
    pub async fn cleanup_dead_rogues(&self) {
        let mut rogues = self.rogue_pids.lock().await;
        rogues.retain(|r| super::pid::process_exists(r.pid));
    }

    /// Get list of current rogue processes
    pub async fn list_rogues(&self) -> Vec<RogueProcess> {
        self.rogue_pids.lock().await.clone()
    }
}

impl Default for RogueProcessTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_kill_current_process_fails() {
        // Trying to kill ourselves should fail (or we wouldn't be here!)
        let _pid = std::process::id() as i32;

        // Don't actually kill ourselves, just test the logic path
        // We can't actually test successful kill without spawning a process
    }

    #[test]
    fn test_get_process_state() {
        let pid = std::process::id() as i32;
        let state = get_process_state(pid).unwrap();

        // We should be in Running or Sleeping state
        assert!(state == 'R' || state == 'S', "Unexpected state: {}", state);
    }

    #[tokio::test]
    async fn test_rogue_tracker() {
        let tracker = RogueProcessTracker::new();

        // Mark a process as rogue
        tracker.mark_rogue(12345, "test reason".to_string()).await;

        // Should be marked as rogue
        assert!(tracker.is_rogue(12345).await);

        // Should not be rogue
        assert!(!tracker.is_rogue(99999).await);

        // List should contain our rogue
        let rogues = tracker.list_rogues().await;
        assert_eq!(rogues.len(), 1);
        assert_eq!(rogues[0].pid, 12345);
    }
}
