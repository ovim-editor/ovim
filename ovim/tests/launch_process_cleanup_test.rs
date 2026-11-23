//! Programs started by the launch flow must not outlive the editor.
//!
//! The registry of live groups is process-global and the cleanup stops all of
//! them, so this lives in a test binary of its own, with a single test.

use ovim_core::launch::process::{CommandSpec, ProcEvent, ProcessHandle};
use ovim_core::launch::{kill_all_launch_groups, process_groups};
use std::collections::BTreeMap;
use std::time::Duration;

fn alive(pid: i32) -> bool {
    // SAFETY: probing for existence only.
    unsafe { libc::kill(pid, 0) == 0 }
}

async fn wait_until_gone(pid: i32) -> bool {
    for _ in 0..200 {
        if !alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

async fn first_line(handle: &mut ProcessHandle) -> String {
    loop {
        if let Some(ProcEvent::Line { text, .. }) = handle.try_recv() {
            return text;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn spec(script: &str) -> CommandSpec {
    CommandSpec {
        argv: vec!["sh".into(), "-c".into(), script.into()],
        cwd: std::env::temp_dir(),
        env: BTreeMap::new(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quitting_stops_running_programs_and_everything_they_started() {
    // A shell that backgrounds a sleeper and waits for it: two processes in
    // one group, like a build tool and its worker.
    let mut running = ProcessHandle::spawn(&spec("sleep 300 & echo $!; wait")).unwrap();
    let shell = running.pid().unwrap() as i32;
    let sleeper: i32 = tokio::time::timeout(Duration::from_secs(5), first_line(&mut running))
        .await
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    // A program that already ended is not the editor's business any more: the
    // group it left behind (a daemon) survives, and nothing is signalled.
    let mut leaver = ProcessHandle::spawn(&spec("sleep 300 & echo $!")).unwrap();
    let daemon: i32 = tokio::time::timeout(Duration::from_secs(5), first_line(&mut leaver))
        .await
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    loop {
        if matches!(leaver.try_recv(), Some(ProcEvent::Exit(_))) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // Anything else can protect its own group the same way.
    let mut adapter = tokio::process::Command::new("sleep");
    adapter.arg("300").process_group(0).kill_on_drop(true);
    let mut adapter = adapter.spawn().unwrap();
    let adapter_pid = adapter.id().unwrap() as i32;
    let _guard = process_groups::register_group(adapter_pid as u32);

    assert!(alive(shell) && alive(sleeper) && alive(daemon) && alive(adapter_pid));

    tokio::task::spawn_blocking(kill_all_launch_groups)
        .await
        .unwrap();

    assert!(
        wait_until_gone(sleeper).await,
        "the worker survived quitting"
    );
    assert!(
        wait_until_gone(shell).await,
        "the program survived quitting"
    );
    // (Waiting also reaps it: a zombie still looks alive to `kill(pid, 0)`.)
    let ended = tokio::time::timeout(Duration::from_secs(5), adapter.wait()).await;
    assert!(ended.is_ok(), "a registered group survived");
    assert!(
        alive(daemon),
        "a finished program's leftovers are left alone"
    );
    // SAFETY: tidying up the sleeper this test left running on purpose.
    unsafe { libc::kill(daemon, libc::SIGKILL) };
    drop(running);
}
