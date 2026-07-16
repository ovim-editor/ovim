//! Process-liveness checks for durable run ownership.
//!
//! Mirrors the stale-PID detection in `crate::session`: a PID is only
//! considered the same process when its start time matches the recorded
//! marker, so PID reuse never masquerades as a live run owner.

/// The calling process's PID and start-time marker for owner registration.
pub(crate) fn current_process_liveness() -> (u32, Option<u64>) {
    let pid = std::process::id();
    (pid, get_process_start_time(pid))
}

/// Whether a process with this PID and recorded start time is still alive.
/// Conservative in the same way as session cleanup: an unreadable start time
/// for an existing PID is treated as a different (dead) process only when a
/// marker was recorded.
pub(crate) fn process_is_alive(pid: u32, expected_start_time: Option<u64>) -> bool {
    if !is_process_exists(pid) {
        return false;
    }
    if let Some(expected) = expected_start_time {
        if let Some(actual) = get_process_start_time(pid) {
            // Allow small variance (1-2 seconds) between capturing the marker
            // and the process actually starting.
            if actual.abs_diff(expected) > 2 {
                return false;
            }
        } else {
            return false;
        }
    }
    true
}

#[cfg(target_os = "linux")]
fn get_process_start_time(pid: u32) -> Option<u64> {
    use std::fs;

    let stat_content = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Skip past the comm field, which can contain spaces and parentheses.
    let start_pos = stat_content.rfind(')')?;
    let fields: Vec<&str> = stat_content[start_pos + 1..].split_whitespace().collect();
    // starttime is the 22nd stat field; index 19 after the ')'.
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
    None
}

#[cfg(unix)]
fn is_process_exists(pid: u32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

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
    // Conservatively assume the process is alive on unsupported platforms.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_process_is_alive_under_its_own_marker() {
        let (pid, start_time) = current_process_liveness();
        assert!(process_is_alive(pid, start_time));
    }

    #[test]
    fn a_mismatched_start_time_proves_the_owner_dead() {
        let (pid, _) = current_process_liveness();
        assert!(!process_is_alive(pid, Some(1)));
    }
}
