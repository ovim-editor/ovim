//! `ovim session kill --verified`, the only way the remote GUI's `--fresh`
//! stops a session: it must signal a process only when it is verifiably the
//! session the descriptor describes.

#![cfg(unix)]

use std::fs;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

fn ovim() -> &'static str {
    env!("CARGO_BIN_EXE_ovim")
}

fn kill_verified(session_dir: &Path, name: &str) -> Output {
    Command::new(ovim())
        .args(["session", "kill", "--verified", "-s", name])
        .env("OVIM_SESSION_DIR", session_dir)
        .output()
        .expect("launch ovim session kill")
}

fn is_running(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .is_ok_and(|status| status.success())
}

/// A process that is not Ovim, killed when the test ends.
struct Bystander(Child);

impl Drop for Bystander {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn bystander() -> Bystander {
    Bystander(
        Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .spawn()
            .expect("sleep should start"),
    )
}

/// Write the descriptor a session would, naming `pid`.
fn descriptor(session_dir: &Path, name: &str, pid: u32, start_time: Option<u64>) {
    let start_time = start_time.map_or("null".to_string(), |start| start.to_string());
    fs::write(
        session_dir.join(format!("{name}.json")),
        format!(
            r#"{{
  "pid": {pid},
  "port": 51829,
  "file": null,
  "started_at": 1,
  "session_name": "{name}",
  "capability": "cafe0000cafe0000cafe0000cafe0000cafe0000cafe0000cafe0000cafe0000",
  "lsp_ready": false,
  "start_time": {start_time}
}}"#
        ),
    )
    .unwrap();
}

fn session_dir() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let sessions = directory.path().join("sessions");
    fs::create_dir(&sessions).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700)).unwrap();
    }
    (directory, sessions)
}

#[test]
fn a_descriptor_without_a_start_time_is_refused_and_nothing_is_signalled() {
    let (_directory, sessions) = session_dir();
    let other = bystander();
    descriptor(&sessions, "no-start", other.0.id(), None);

    let output = kill_verified(&sessions, "no-start");

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("Refusing"));
    assert!(is_running(other.0.id()));
}

#[test]
fn a_pid_that_started_at_another_time_is_left_alone_and_the_descriptor_removed() {
    let (_directory, sessions) = session_dir();
    let other = bystander();
    descriptor(&sessions, "reused", other.0.id(), Some(1));

    let output = kill_verified(&sessions, "reused");

    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(is_running(other.0.id()));
    assert!(!sessions.join("reused.json").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn a_live_process_that_is_not_ovim_is_refused_even_with_its_real_start_time() {
    let (_directory, sessions) = session_dir();
    let other = bystander();
    let pid = other.0.id();
    let start_time = (0..100)
        .find_map(|_| {
            thread::sleep(Duration::from_millis(10));
            let exe = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
            if exe.file_name()? != "sleep" {
                return None;
            }
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            stat[stat.rfind(')')? + 1..]
                .split_whitespace()
                .nth(19)?
                .parse::<u64>()
                .ok()
        })
        .expect("the start time of sleep");
    descriptor(&sessions, "not-ovim", pid, Some(start_time));

    let output = kill_verified(&sessions, "not-ovim");

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("not Ovim"));
    assert!(is_running(pid));
}

#[test]
fn a_real_session_is_verified_and_stopped() {
    let (directory, sessions) = session_dir();
    let file = directory.path().join("document.txt");
    fs::write(&file, "first\n").unwrap();
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("kill-verified-{}-{unique}", std::process::id());
    let mut child = Command::new(ovim())
        .arg(&file)
        .args(["--headless", "--session", &name])
        .env("OVIM_SESSION_DIR", &sessions)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start a real headless ovim");
    let descriptor_path = sessions.join(format!("{name}.json"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !descriptor_path.exists() {
        assert!(Instant::now() < deadline, "the session never registered");
        thread::sleep(Duration::from_millis(25));
    }

    let output = kill_verified(&sessions, &name);

    assert!(output.status.success(), "{output:?}");
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the verified session was not stopped");
        }
        thread::sleep(Duration::from_millis(25));
    }
}
