#![cfg(unix)]

use nix::{sys::signal::kill, unistd::Pid};
use ovim::language_config::AutoInstallConfig;
use ovim::lsp_init::auto_install::{attempt_auto_install, InstallResult};
use std::{path::Path, time::Duration};

fn installer(dir: &Path, name: &str) -> AutoInstallConfig {
    serde_json::from_value(serde_json::json!({
        "method": {
            "type": "shell",
            "command": format!("python3 {} {} {name}", dir.join("install.py").display(), dir.display())
        }
    }))
    .unwrap()
}

async fn wait_until(mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("installer did not reach the expected state");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn installs_are_serialized_and_cancellation_stops_the_process_before_retry() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("install.py"),
        include_str!("helpers/controlled_install.py"),
    )
    .unwrap();
    let first_config = installer(dir.path(), "first");
    let first =
        tokio::spawn(async move { attempt_auto_install("Test", "sh", &first_config).await });
    let first_pid = dir.path().join("first.pid");
    wait_until(|| first_pid.exists()).await;
    let pid = Pid::from_raw(std::fs::read_to_string(first_pid).unwrap().parse().unwrap());

    // Poll the second request while the first installer holds the barrier.
    // Its process must not enter the shared installation directory yet.
    let second_config = installer(dir.path(), "second");
    let second = attempt_auto_install("Test", "sh", &second_config);
    tokio::pin!(second);
    assert!(futures::poll!(&mut second).is_pending());
    let second_pid = dir.path().join("second.pid");
    assert!(tokio::time::timeout(
        Duration::from_millis(100),
        wait_until(|| second_pid.exists())
    )
    .await
    .is_err());

    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    wait_until(|| kill(pid, None).is_err()).await;

    std::fs::write(dir.path().join("second.release"), "").unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .expect("cancelled install left the installation gate locked");
    assert!(matches!(result, InstallResult::Success(_)), "{result:?}");
    assert!(second_pid.exists());
}
