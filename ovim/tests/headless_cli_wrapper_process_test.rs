//! Process-level regression coverage for blocking session CLI wrappers.
//!
//! These commands use reqwest's blocking client while the ovim binary also
//! owns a Tokio runtime for editor/server mode. The wrappers must complete
//! without dropping reqwest's private runtime from a non-blocking async task.

use serde_json::Value;
use std::fs;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

struct HeadlessSession {
    child: Child,
}

impl Drop for HeadlessSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn ovim() -> &'static str {
    env!("CARGO_BIN_EXE_ovim")
}

fn run_cli(session_dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(ovim())
        .args(args)
        .env("OVIM_SESSION_DIR", session_dir)
        .output()
        .expect("launch ovim CLI wrapper")
}

fn output_detail(output: &Output) -> String {
    format!(
        "status={}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn send_and_snapshot_wrappers_work_against_a_real_headless_process() {
    let directory = tempfile::tempdir().unwrap();
    let session_dir = directory.path().join("sessions");
    fs::create_dir(&session_dir).unwrap();
    let file = directory.path().join("document.txt");
    fs::write(&file, "first\nsecond\n").unwrap();
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let session_name = format!("cli-wrapper-{}-{unique}", std::process::id());

    let child = Command::new(ovim())
        .arg(&file)
        .arg("--headless")
        .arg("--session")
        .arg(&session_name)
        .arg("--dimension")
        .arg("64x18")
        .env("OVIM_SESSION_DIR", &session_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start real headless ovim process");
    let mut session = HeadlessSession { child };

    let session_file = session_dir.join(format!("{session_name}.json"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !session_file.exists() {
        assert!(
            session.child.try_wait().unwrap().is_none(),
            "headless ovim exited before registering its session"
        );
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            session_file.display()
        );
        thread::sleep(Duration::from_millis(25));
    }

    // Establish that the documented snapshot wrapper can connect. Retrying
    // only absorbs the small registration/listener scheduling window.
    let initial_snapshot = loop {
        let output = run_cli(&session_dir, &["snapshot", "-s", &session_name]);
        if output.status.success() {
            break output;
        }
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("Cannot drop a runtime"),
            "snapshot wrapper reproduced the Tokio/reqwest panic:\n{}",
            output_detail(&output)
        );
        assert!(
            Instant::now() < deadline,
            "snapshot wrapper never connected:\n{}",
            output_detail(&output)
        );
        thread::sleep(Duration::from_millis(25));
    };
    let initial: Value = serde_json::from_slice(&initial_snapshot.stdout)
        .expect("snapshot wrapper should print JSON");
    assert_eq!(initial["schema_version"], 3);
    assert_eq!(initial["buffer"]["content"], "first\nsecond\n");
    assert_eq!(initial["view"]["viewport_width"], 64);
    assert_eq!(initial["view"]["viewport_height"], 18);
    let session_json: Value =
        serde_json::from_slice(&fs::read(&session_file).expect("read registered session metadata"))
            .expect("session metadata is JSON");
    assert_eq!(session_json["viewport_width"], 64);
    assert_eq!(session_json["viewport_height"], 18);

    let file_arg = file.to_string_lossy().into_owned();
    let live_edit = run_cli(
        &session_dir,
        &[
            "edit",
            &file_arg,
            "--line",
            "1",
            "--old",
            "first",
            "--new",
            "FIRST",
            "--session",
            &session_name,
        ],
    );
    assert!(live_edit.status.success(), "{}", output_detail(&live_edit));
    let edited_snapshot = run_cli(&session_dir, &["snapshot", "-s", &session_name]);
    let edited: Value = serde_json::from_slice(&edited_snapshot.stdout).unwrap();
    assert_eq!(edited["buffer"]["content"], "FIRST\nsecond\n");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "first\nsecond\n",
        "session-aware edits should not bypass the live buffer"
    );

    let save = run_cli(&session_dir, &["exec", "-s", &session_name, "w"]);
    assert!(save.status.success(), "{}", output_detail(&save));
    assert_eq!(fs::read_to_string(&file).unwrap(), "FIRST\nsecond\n");

    // A clean headless buffer should detect and reload an external write just
    // like a focused TUI does, without requiring terminal focus events.
    thread::sleep(Duration::from_millis(20));
    fs::write(&file, "first\nsecond\n").unwrap();
    let reload_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let output = run_cli(&session_dir, &["snapshot", "-s", &session_name]);
        if output.status.success() {
            let snapshot: Value = serde_json::from_slice(&output.stdout).unwrap();
            if snapshot["buffer"]["content"] == "first\nsecond\n" {
                break;
            }
        }
        assert!(
            Instant::now() < reload_deadline,
            "headless session did not reload the external file change"
        );
        thread::sleep(Duration::from_millis(50));
    }

    let send = run_cli(&session_dir, &["send", "-s", &session_name, "j"]);
    assert!(
        send.status.success(),
        "send wrapper failed or panicked:\n{}",
        output_detail(&send)
    );
    let rendered = String::from_utf8_lossy(&send.stdout);
    assert!(
        rendered.contains("first") && rendered.contains("second"),
        "send wrapper should print the plain editor render:\n{}",
        output_detail(&send)
    );
    assert_eq!(
        rendered.lines().count(),
        18,
        "send should use session height"
    );
    assert!(
        rendered.lines().all(|line| line.chars().count() == 64),
        "send should use session width"
    );

    let enter_insert = run_cli(&session_dir, &["send", "-s", &session_name, "i"]);
    assert!(
        enter_insert.status.success(),
        "{}",
        output_detail(&enter_insert)
    );
    let paste = run_cli(
        &session_dir,
        &["paste", "-s", &session_name, "héllo\\nworld"],
    );
    assert!(paste.status.success(), "{}", output_detail(&paste));
    let leave_insert = run_cli(&session_dir, &["send", "-s", &session_name, "<Esc>"]);
    assert!(
        leave_insert.status.success(),
        "{}",
        output_detail(&leave_insert)
    );

    let snapshot = run_cli(&session_dir, &["snapshot", "-s", &session_name]);
    assert!(
        snapshot.status.success(),
        "snapshot wrapper failed or panicked after send:\n{}",
        output_detail(&snapshot)
    );
    let state: Value =
        serde_json::from_slice(&snapshot.stdout).expect("snapshot wrapper should print JSON");
    assert_eq!(state["buffer"]["content"], "first\nhéllo\nworldsecond\n");
    assert_eq!(state["cursor"]["line"], 2);
    assert_eq!(state["mode"], "NORMAL");

    let terminate = Command::new("kill")
        .args(["-TERM", &session.child.id().to_string()])
        .status()
        .expect("send SIGTERM to headless session");
    assert!(terminate.success());
    let shutdown_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if session.child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(
            Instant::now() < shutdown_deadline,
            "headless session did not shut down gracefully after SIGTERM"
        );
        thread::sleep(Duration::from_millis(25));
    }
    assert!(
        !session_file.exists(),
        "session metadata should be removed during graceful shutdown"
    );
}
