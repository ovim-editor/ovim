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
    use sysinfo::{Pid, ProcessesToUpdate, System};

    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let table_is_populated = !system.processes().is_empty();
    let owner_entry = system
        .process(Pid::from_u32(pid))
        .map(|process| process.start_time());
    observation_from_full_process_table(table_is_populated, owner_entry)
}

/// Maps a full process-table enumeration (sysinfo on Windows) to an
/// observation. Platform-independent so the mapping is unit-testable on the
/// unix hosts that build this crate, even though only the Windows branch
/// feeds it.
///
/// Conservative on two sysinfo failure modes:
/// - When `NtQuerySystemInformation` fails, sysinfo's refresh returns 0 and
///   leaves the process table empty, so an empty table is indistinguishable
///   from a failed enumeration and never proves nonexistence. Only a
///   populated table that lacks the PID counts as proof of death.
/// - When sysinfo cannot open a process handle (e.g. access denied), it
///   records a start time of 0 rather than failing, so a zero start time
///   means "present but unreadable" — the EPERM analogue — and must not be
///   compared against the recorded marker.
#[cfg_attr(not(windows), allow(dead_code))]
fn observation_from_full_process_table(
    table_is_populated: bool,
    owner_entry: Option<u64>,
) -> ProcessObservation {
    match owner_entry {
        Some(0) => ProcessObservation::ExistsStartTimeUnknown,
        Some(start_time) => ProcessObservation::ExistsWithStartTime(start_time),
        None if table_is_populated => ProcessObservation::NotFound,
        None => ProcessObservation::ExistsStartTimeUnknown,
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
    use sysinfo::{Pid, ProcessesToUpdate, System};

    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    let start_time = system.process(Pid::from_u32(pid))?.start_time();
    // sysinfo records 0 when it cannot open the process handle; an unreadable
    // start time must stay `None` so it is never mistaken for a real marker.
    (start_time != 0).then_some(start_time)
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

    #[test]
    fn an_empty_process_table_never_proves_death() {
        // On Windows, a failed NtQuerySystemInformation leaves sysinfo's
        // table empty; that is an observation failure, not proof the PID is
        // gone, so recovery must be declined even with a recorded marker.
        let observation = observation_from_full_process_table(false, None);
        assert_eq!(observation, ProcessObservation::ExistsStartTimeUnknown);
        assert!(liveness_decision(observation, Some(100)));
        assert!(liveness_decision(observation, None));
    }

    #[test]
    fn a_populated_process_table_without_the_pid_proves_death() {
        let observation = observation_from_full_process_table(true, None);
        assert_eq!(observation, ProcessObservation::NotFound);
        assert!(!liveness_decision(observation, Some(100)));
        assert!(!liveness_decision(observation, None));
    }

    #[test]
    fn a_table_entry_with_a_readable_start_time_feeds_the_reuse_check() {
        let observation = observation_from_full_process_table(true, Some(100));
        assert_eq!(observation, ProcessObservation::ExistsWithStartTime(100));
        assert!(liveness_decision(observation, Some(100)));
        assert!(!liveness_decision(observation, Some(500)));
    }

    #[test]
    fn a_zero_start_time_entry_is_unreadable_not_a_mismatch() {
        // sysinfo records start_time 0 when the process handle could not be
        // opened (access denied). A live but unreadable owner must not be
        // declared dead via a bogus 0-vs-marker mismatch.
        let observation = observation_from_full_process_table(true, Some(0));
        assert_eq!(observation, ProcessObservation::ExistsStartTimeUnknown);
        assert!(liveness_decision(observation, Some(100)));
    }

    #[cfg(unix)]
    #[test]
    fn pid_one_is_alive_even_when_it_cannot_be_signaled() {
        // kill(1, 0) from an unprivileged test yields EPERM on most systems:
        // the process exists but is not ours. That must never read as dead.
        assert!(process_is_alive(1, None));
    }
}
