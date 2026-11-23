//! Process groups that must not outlive the editor.
//!
//! A program started by the launch flow runs in a process group of its own,
//! so Stop reaches everything it forked. The flip side is that neither the
//! terminal's hangup nor the editor calling `exit` reaches it: a build tool,
//! or a JVM suspended on its debug port, would live on. Each such group is
//! registered here while its leader runs, and the editor calls
//! [`kill_all_launch_groups`] on every way out (quitting, `:cq`, fatal
//! signals, panics). Anything else that starts a long-lived child of its own
//! (a debug adapter) can register its group the same way.

use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard, PoisonError};
#[cfg(unix)]
use std::time::{Duration, Instant};

static LIVE: Mutex<BTreeSet<u32>> = Mutex::new(BTreeSet::new());

/// How long a group gets to exit on SIGTERM before it is killed outright.
#[cfg(unix)]
const TERM_GRACE: Duration = Duration::from_millis(300);

fn live() -> MutexGuard<'static, BTreeSet<u32>> {
    LIVE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Keeps a process group registered; dropping it (when the group's leader has
/// been reaped, or its whole group was stopped on purpose) unregisters it.
#[must_use = "the group is only protected while the guard is alive"]
pub struct GroupGuard {
    pgid: u32,
}

/// Registers the process group `pgid` (the pid of a child started with
/// `process_group(0)`) to be killed by [`kill_all_launch_groups`].
pub fn register_group(pgid: u32) -> GroupGuard {
    live().insert(pgid);
    GroupGuard { pgid }
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        live().remove(&self.pgid);
    }
}

/// Stops every registered group: SIGTERM, a brief wait for them to go, then
/// SIGKILL for what is left. Blocks, so that it can run right before the
/// process exits; calling it again finds nothing left to do.
pub fn kill_all_launch_groups() {
    let groups: Vec<u32> = std::mem::take(&mut *live()).into_iter().collect();
    if groups.is_empty() {
        return;
    }
    #[cfg(unix)]
    {
        for &group in &groups {
            terminate_group(Some(group), false);
        }
        let deadline = Instant::now() + TERM_GRACE;
        while Instant::now() < deadline && groups.iter().any(|&g| group_exists(g)) {
            std::thread::sleep(Duration::from_millis(10));
        }
        for &group in &groups {
            terminate_group(Some(group), true);
        }
    }
}

#[cfg(unix)]
pub(super) fn terminate_group(pgid: Option<u32>, force: bool) {
    let Some(pgid) = pgid else { return };
    let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
    // SAFETY: plain signal delivery to a process group we created.
    unsafe {
        libc::kill(-(pgid as i32), signal);
    }
}

#[cfg(not(unix))]
pub(super) fn terminate_group(_pgid: Option<u32>, _force: bool) {}

/// Whether the group still has a member (an unreaped zombie counts).
#[cfg(unix)]
fn group_exists(pgid: u32) -> bool {
    // SAFETY: probing for existence only.
    let found = unsafe { libc::kill(-(pgid as i32), 0) } == 0;
    found || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_is_registered_exactly_while_its_guard_lives() {
        // A pid no real group has.
        let pgid = u32::MAX / 2;
        let guard = register_group(pgid);
        assert!(live().contains(&pgid));
        drop(guard);
        assert!(!live().contains(&pgid));
    }
}
