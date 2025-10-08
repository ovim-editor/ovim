//! PID verification and management
//!
//! CRITICAL SECURITY: Prevents PID reuse attack
//!
//! Problem: If daemon crashes, the OS could reuse the PID for a different process.
//! Without verification, we could kill the wrong process!
//!
//! Solution: Store and verify:
//! 1. PID
//! 2. Process start time (unique per boot)
//! 3. Command line hash (identifies our process)

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::SystemTime;

/// PID information with verification data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonPidInfo {
    /// Process ID
    pub pid: i32,

    /// Process start time (prevents PID reuse)
    pub start_time: SystemTime,

    /// Hash of command line (identifies our process)
    pub cmd_hash: u64,
}

impl DaemonPidInfo {
    /// Create PID info for a process
    pub fn new(pid: i32) -> Result<Self> {
        let start_time = get_process_start_time(pid)
            .with_context(|| format!("Failed to get start time for PID {}", pid))?;

        let cmdline = get_process_cmdline(pid)
            .with_context(|| format!("Failed to get cmdline for PID {}", pid))?;

        let cmd_hash = hash_string(&cmdline);

        Ok(Self {
            pid,
            start_time,
            cmd_hash,
        })
    }

    /// Verify this PID still matches the original process
    ///
    /// Returns Ok(true) if process matches, Ok(false) if mismatch, Err if check failed
    pub fn verify(&self) -> Result<bool> {
        // Check 1: Process exists?
        if !process_exists(self.pid) {
            return Ok(false);
        }

        // Check 2: Start time matches?
        let current_start_time = get_process_start_time(self.pid)?;
        if current_start_time != self.start_time {
            eprintln!(
                "[daemon] Warning: PID {} start time mismatch (PID reused by different process)",
                self.pid
            );
            return Ok(false);
        }

        // Check 3: Command line matches?
        let current_cmdline = get_process_cmdline(self.pid)?;
        let current_hash = hash_string(&current_cmdline);
        if current_hash != self.cmd_hash {
            eprintln!(
                "[daemon] Warning: PID {} cmdline mismatch (process changed or PID reused)",
                self.pid
            );
            return Ok(false);
        }

        // All checks passed - this is our process
        Ok(true)
    }

    /// Save to PID file
    pub async fn save(&self, pid_file: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        tokio::fs::write(pid_file, json).await?;
        Ok(())
    }

    /// Load from PID file
    pub async fn load(pid_file: &Path) -> Result<Self> {
        let json = tokio::fs::read_to_string(pid_file).await?;
        let info: Self = serde_json::from_str(&json)?;
        Ok(info)
    }

    /// Load and verify in one step
    pub async fn load_and_verify(pid_file: &Path) -> Result<Option<Self>> {
        let info = Self::load(pid_file).await?;

        match info.verify()? {
            true => Ok(Some(info)),
            false => Ok(None),
        }
    }
}

/// Check if process exists
pub fn process_exists(pid: i32) -> bool {
    #[cfg(unix)]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;

        // Signal 0 checks existence without sending signal
        match kill(Pid::from_raw(pid), None) {
            Ok(_) => true,
            Err(nix::errno::Errno::ESRCH) => false, // No such process
            Err(_) => true, // Process exists but we can't signal it
        }
    }

    #[cfg(not(unix))]
    {
        // Fallback for non-Unix
        std::path::Path::new(&format!("/proc/{}", pid)).exists()
    }
}

/// Get process start time
///
/// This is CRITICAL for preventing PID reuse attacks.
/// Start time is unique per boot cycle.
#[cfg(target_os = "linux")]
pub fn get_process_start_time(pid: i32) -> Result<SystemTime> {
    use std::fs;

    // Read /proc/{pid}/stat
    let stat = fs::read_to_string(format!("/proc/{}/stat", pid))
        .context("Failed to read /proc/stat")?;

    // Parse stat file
    // Format: pid (comm) state ppid pgrp session tty_nr tpgid flags ... starttime
    // Field 22 (index 21) is starttime in clock ticks since boot

    // Find the last ')' to handle process names with spaces/parens
    let start = stat.rfind(')').context("Invalid stat format")?;
    let fields: Vec<&str> = stat[start + 1..].split_whitespace().collect();

    if fields.len() < 20 {
        anyhow::bail!("stat file has too few fields");
    }

    // Field 22 is at index 19 after the comm field
    let start_ticks: u64 = fields[19]
        .parse()
        .context("Failed to parse start time")?;

    // Get system boot time
    let boot_time = get_boot_time()?;

    // Get clock ticks per second
    let ticks_per_sec = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as u64;

    // Calculate start time
    let start_secs = start_ticks / ticks_per_sec;
    let start_time = boot_time + std::time::Duration::from_secs(start_secs);

    Ok(start_time)
}

#[cfg(target_os = "macos")]
pub fn get_process_start_time(pid: i32) -> Result<SystemTime> {
    use std::process::Command;
    use std::time::UNIX_EPOCH;

    // Use ps to get process start time in epoch seconds
    // The 'etime' format shows elapsed time, but we need absolute start time
    // Use 'lstart' which gives us the full start time
    let output = Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("lstart=")
        .output()
        .context("Failed to run ps")?;

    if !output.status.success() {
        anyhow::bail!("ps command failed for PID {}", pid);
    }

    let lstart_str = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // lstart format: "Tue Jan  7 14:23:45 2025"
    // We'll use a simpler approach: get elapsed time and subtract from now
    let output = Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("etime=")
        .output()
        .context("Failed to run ps for etime")?;

    if !output.status.success() {
        anyhow::bail!("ps etime command failed for PID {}", pid);
    }

    let etime_str = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Parse elapsed time (formats: "MM:SS", "HH:MM:SS", "DD-HH:MM:SS")
    let elapsed_secs = parse_elapsed_time(&etime_str)
        .with_context(|| format!("Failed to parse etime: {}", etime_str))?;

    // Start time = now - elapsed
    let now = SystemTime::now();
    let start_time = now
        .checked_sub(std::time::Duration::from_secs(elapsed_secs))
        .context("Failed to calculate start time")?;

    Ok(start_time)
}

/// Parse ps etime format into seconds
/// Formats: "MM:SS", "HH:MM:SS", "DD-HH:MM:SS", "DD-HH:MM:SS"
#[cfg(target_os = "macos")]
fn parse_elapsed_time(etime: &str) -> Result<u64> {
    let etime = etime.trim();

    // Check for days format: "DD-HH:MM:SS"
    if let Some((days_str, rest)) = etime.split_once('-') {
        let days: u64 = days_str.parse().context("Invalid days")?;
        let parts: Vec<&str> = rest.split(':').collect();

        if parts.len() != 3 {
            anyhow::bail!("Invalid time format after days: {}", rest);
        }

        let hours: u64 = parts[0].parse().context("Invalid hours")?;
        let minutes: u64 = parts[1].parse().context("Invalid minutes")?;
        let seconds: u64 = parts[2].parse().context("Invalid seconds")?;

        return Ok(days * 86400 + hours * 3600 + minutes * 60 + seconds);
    }

    // No days, parse as HH:MM:SS or MM:SS
    let parts: Vec<&str> = etime.split(':').collect();

    match parts.len() {
        2 => {
            // MM:SS
            let minutes: u64 = parts[0].trim().parse().context("Invalid minutes")?;
            let seconds: u64 = parts[1].trim().parse().context("Invalid seconds")?;
            Ok(minutes * 60 + seconds)
        }
        3 => {
            // HH:MM:SS
            let hours: u64 = parts[0].trim().parse().context("Invalid hours")?;
            let minutes: u64 = parts[1].trim().parse().context("Invalid minutes")?;
            let seconds: u64 = parts[2].trim().parse().context("Invalid seconds")?;
            Ok(hours * 3600 + minutes * 60 + seconds)
        }
        _ => anyhow::bail!("Invalid etime format: {}", etime),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn get_process_start_time(_pid: i32) -> Result<SystemTime> {
    anyhow::bail!("Process start time not supported on this platform")
}

/// Get boot time (Linux only)
#[cfg(target_os = "linux")]
fn get_boot_time() -> Result<SystemTime> {
    use std::fs;
    use std::time::UNIX_EPOCH;

    let stat = fs::read_to_string("/proc/stat")?;

    for line in stat.lines() {
        if line.starts_with("btime ") {
            let boot_secs: u64 = line[6..]
                .trim()
                .parse()
                .context("Failed to parse btime")?;

            return Ok(UNIX_EPOCH + std::time::Duration::from_secs(boot_secs));
        }
    }

    anyhow::bail!("btime not found in /proc/stat")
}

/// Get process command line
#[cfg(target_os = "linux")]
pub fn get_process_cmdline(pid: i32) -> Result<String> {
    use std::fs;

    let cmdline = fs::read_to_string(format!("/proc/{}/cmdline", pid))
        .context("Failed to read cmdline")?;

    // cmdline uses null bytes as separators
    let cmdline = cmdline.replace('\0', " ").trim().to_string();

    if cmdline.is_empty() {
        anyhow::bail!("Empty cmdline");
    }

    Ok(cmdline)
}

#[cfg(target_os = "macos")]
pub fn get_process_cmdline(pid: i32) -> Result<String> {
    use std::process::Command;

    let output = Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("command=")
        .output()
        .context("Failed to run ps")?;

    if !output.status.success() {
        anyhow::bail!("ps command failed");
    }

    let cmdline = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string();

    if cmdline.is_empty() {
        anyhow::bail!("Empty cmdline");
    }

    Ok(cmdline)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn get_process_cmdline(_pid: i32) -> Result<String> {
    anyhow::bail!("Process cmdline not supported on this platform")
}

/// Hash a string
fn hash_string(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_exists() {
        // Current process should exist
        let pid = std::process::id() as i32;
        assert!(process_exists(pid));

        // PID 99999 probably doesn't exist
        assert!(!process_exists(99999));
    }

    #[test]
    fn test_get_process_start_time() {
        let pid = std::process::id() as i32;
        let start_time = get_process_start_time(pid).unwrap();

        // Start time should be recent (within last hour)
        let now = SystemTime::now();
        let elapsed = now.duration_since(start_time).unwrap();

        assert!(elapsed.as_secs() < 3600, "Process started too long ago");
    }

    #[test]
    fn test_get_process_cmdline() {
        let pid = std::process::id() as i32;
        let cmdline = get_process_cmdline(pid).unwrap();

        // Should get non-empty cmdline for current process
        assert!(!cmdline.is_empty());

        // Cmdline should contain ovim or rust/cargo related terms
        // (but don't be too strict as it depends on test runner)
        assert!(
            cmdline.contains("ovim")
            || cmdline.contains("cargo")
            || cmdline.contains("test")
            || cmdline.contains("rust")
            || cmdline.len() > 5, // At minimum, should have some content
            "Unexpected cmdline: {}", cmdline
        );
    }

    #[test]
    fn test_pid_info_creation() {
        let pid = std::process::id() as i32;
        let info = DaemonPidInfo::new(pid).unwrap();

        assert_eq!(info.pid, pid);
        assert!(info.verify().unwrap());
    }

    #[test]
    fn test_pid_info_verify() {
        let pid = std::process::id() as i32;
        let info = DaemonPidInfo::new(pid).unwrap();

        // Should verify successfully
        assert!(info.verify().unwrap());

        // Create info with wrong PID
        let wrong_info = DaemonPidInfo {
            pid: 99999,
            start_time: SystemTime::now(),
            cmd_hash: 0,
        };

        // Should fail verification (process doesn't exist)
        assert!(!wrong_info.verify().unwrap());
    }

    #[test]
    fn test_hash_string() {
        let s1 = "hello world";
        let s2 = "hello world";
        let s3 = "different";

        assert_eq!(hash_string(s1), hash_string(s2));
        assert_ne!(hash_string(s1), hash_string(s3));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_parse_elapsed_time() {
        // Test MM:SS format
        assert_eq!(parse_elapsed_time("05:30").unwrap(), 5 * 60 + 30);

        // Test HH:MM:SS format
        assert_eq!(parse_elapsed_time("02:15:45").unwrap(), 2 * 3600 + 15 * 60 + 45);

        // Test DD-HH:MM:SS format
        assert_eq!(
            parse_elapsed_time("1-03:20:10").unwrap(),
            1 * 86400 + 3 * 3600 + 20 * 60 + 10
        );

        // Test with spaces (trim)
        assert_eq!(parse_elapsed_time("  10:05  ").unwrap(), 10 * 60 + 5);
    }
}
