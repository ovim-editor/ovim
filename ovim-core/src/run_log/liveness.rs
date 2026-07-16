//! Process-liveness checks for durable run ownership.
//!
//! Mirrors the stale-PID detection in `crate::session`: a PID is only
//! considered the same process when its start time matches the recorded
//! marker, so PID reuse never masquerades as a live run owner.
//!
//! Death must be proven, never inferred from an observation failure: callers
//! append terminal recovery events to runs whose owner is "dead", so a false
//! death verdict corrupts a live writer's run. Only two observations count as
//! proof: the kernel reporting that no such process exists (ESRCH), or a
//! readable start time that does not match the recorded marker (PID reuse).
//! Everything else — EPERM, an unreadable /proc entry, a failed
//! proc_pidinfo — means "possibly alive" and recovery is declined.

/// The calling process's PID and start-time marker for owner registration.
pub(crate) fn current_process_liveness() -> (u32, Option<u64>) {
    let pid = std::process::id();
    (pid, get_process_start_time(pid))
}

/// Whether a process with this PID and recorded start time may still be
/// alive. Returns `false` only on positive proof of death (nonexistence or a
/// start-time mismatch that proves PID reuse); any failure to observe the
/// process returns `true` so callers never treat a live writer as dead.
pub(crate) fn process_is_alive(pid: u32, expected_start_time: Option<u64>) -> bool {
    liveness_decision(observe_process(pid), expected_start_time)
}

/// A single point-in-time observation of a PID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProcessObservation {
    /// The kernel proved that no process has this PID.
    NotFound,
    /// The process exists and its start time was read.
    ExistsWithStartTime(u64),
    /// The process exists — or its existence could not be ruled out (for
    /// example EPERM, or a transient /proc or proc_pidinfo failure) — but its
    /// start time could not be read.
    ExistsStartTimeUnknown,
}

/// Pure decision core over an observation. Conservative by construction:
/// every arm that lacks positive proof of death returns "alive".
fn liveness_decision(observation: ProcessObservation, expected_start_time: Option<u64>) -> bool {
    match observation {
        ProcessObservation::NotFound => false,
        // The PID is (or may be) occupied but the start time is unknowable, so
        // PID reuse cannot be proven; the owner must be assumed alive.
        ProcessObservation::ExistsStartTimeUnknown => true,
        ProcessObservation::ExistsWithStartTime(actual) => match expected_start_time {
            // Allow small variance (1-2 seconds) between capturing the marker
            // and the process actually starting. A larger difference proves
            // the PID was reused by a different process.
            Some(expected) => actual.abs_diff(expected) <= 2,
            None => true,
        },
    }
}

#[cfg(unix)]
fn observe_process(pid: u32) -> ProcessObservation {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    match kill(Pid::from_raw(pid as i32), None) {
        // ESRCH is the only kernel answer that proves nonexistence.
        Err(Errno::ESRCH) => ProcessObservation::NotFound,
        // Ok: the process exists and is signalable. EPERM: the process exists
        // but belongs to someone else. Any other error: existence cannot be
        // ruled out, so treat the PID as occupied.
        Ok(()) | Err(_) => match get_process_start_time(pid) {
            Some(start_time) => ProcessObservation::ExistsWithStartTime(start_time),
            None => ProcessObservation::ExistsStartTimeUnknown,
        },
    }
}

#[cfg(windows)]
fn observe_process(pid: u32) -> ProcessObservation {
    use sysinfo::{Pid, System};

    let mut system = System::new();
    system.refresh_processes();
    match system.process(Pid::from_u32(pid)) {
        // A full enumeration without the PID is the strongest available proof
        // of nonexistence on Windows.
        None => ProcessObservation::NotFound,
        Some(process) => ProcessObservation::ExistsWithStartTime(process.start_time()),
    }
}

#[cfg(not(any(unix, windows)))]
fn observe_process(_pid: u32) -> ProcessObservation {
    // No way to observe processes here; nonexistence can never be proven.
    ProcessObservation::ExistsStartTimeUnknown
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
        let (pid, start_time) = current_process_liveness();
        // The marker can only prove PID reuse where start times are readable;
        // elsewhere the conservative answer is "alive".
        if start_time.is_some() {
            assert!(!process_is_alive(pid, Some(1)));
        } else {
            assert!(process_is_alive(pid, Some(1)));
        }
    }

    #[test]
    fn proven_nonexistence_is_dead() {
        assert!(!liveness_decision(ProcessObservation::NotFound, None));
        assert!(!liveness_decision(ProcessObservation::NotFound, Some(1)));
    }

    #[test]
    fn an_unreadable_start_time_for_an_existing_pid_is_possibly_alive() {
        // EPERM-style observations land here: the PID is occupied but not
        // ours to inspect. Even with a recorded marker, reuse is unprovable.
        assert!(liveness_decision(
            ProcessObservation::ExistsStartTimeUnknown,
            Some(1)
        ));
        assert!(liveness_decision(
            ProcessObservation::ExistsStartTimeUnknown,
            None
        ));
    }

    #[test]
    fn a_matching_start_time_is_alive() {
        assert!(liveness_decision(
            ProcessObservation::ExistsWithStartTime(100),
            Some(100)
        ));
    }

    #[test]
    fn start_time_variance_within_two_seconds_is_alive() {
        assert!(liveness_decision(
            ProcessObservation::ExistsWithStartTime(102),
            Some(100)
        ));
        assert!(liveness_decision(
            ProcessObservation::ExistsWithStartTime(98),
            Some(100)
        ));
    }

    #[test]
    fn start_time_variance_beyond_two_seconds_proves_pid_reuse() {
        assert!(!liveness_decision(
            ProcessObservation::ExistsWithStartTime(103),
            Some(100)
        ));
    }

    #[test]
    fn an_existing_pid_without_a_marker_is_alive() {
        assert!(liveness_decision(
            ProcessObservation::ExistsWithStartTime(100),
            None
        ));
    }

    #[cfg(unix)]
    #[test]
    fn pid_one_is_alive_even_when_it_cannot_be_signaled() {
        // kill(1, 0) from an unprivileged test yields EPERM on most systems:
        // the process exists but is not ours. That must never read as dead.
        assert!(process_is_alive(1, None));
    }
}
