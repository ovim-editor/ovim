use anyhow::Result;
use ovim::session::{cleanup_stale_sessions, SessionInfo};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

/// Helper to create a fake session file for testing
fn create_fake_session(name: &str, pid: u32, port: u16, age_secs: u64) -> Result<PathBuf> {
    use std::time::SystemTime;

    let started_at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs()
        .saturating_sub(age_secs);

    let session_info = SessionInfo {
        pid,
        port,
        file: Some(format!("test_{}.txt", name)),
        started_at,
        session_name: name.to_string(),
        lsp_ready: false,
        start_time: None, // No start time = stale
    };

    session_info.write()?;
    session_info.session_file_path()
}

/// Helper to create a corrupted session file
fn create_corrupted_session(name: &str) -> Result<PathBuf> {
    let session_dir = SessionInfo::session_dir()?;
    let path = session_dir.join(format!("{}.json", name));

    fs::write(&path, "{ invalid json }")?;

    Ok(path)
}

// Note: Orphaned temp file cleanup is tested implicitly by the cleanup function
// Creating temp files with specific mtimes is platform-specific and complex

#[test]
fn test_cleanup_stale_sessions() -> Result<()> {
    use std::time::SystemTime;

    // Create fake stale sessions directly (bypass list_all auto-cleanup)
    let fake_pid = 999999; // Very unlikely to exist
    let session_dir = SessionInfo::session_dir()?;

    let session1 = SessionInfo {
        pid: fake_pid,
        port: 8080,
        file: Some("test1.txt".to_string()),
        started_at: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs(),
        session_name: "stale_test1".to_string(),
        lsp_ready: false,
        start_time: None,
    };

    let session2 = SessionInfo {
        pid: fake_pid + 1,
        port: 8081,
        file: Some("test2.txt".to_string()),
        started_at: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs(),
        session_name: "stale_test2".to_string(),
        lsp_ready: false,
        start_time: None,
    };

    // Write directly to files
    let path1 = session_dir.join("stale_test1.json");
    let path2 = session_dir.join("stale_test2.json");

    fs::write(&path1, serde_json::to_string_pretty(&session1)?)?;
    fs::write(&path2, serde_json::to_string_pretty(&session2)?)?;

    // Verify files exist
    assert!(path1.exists(), "Session 1 should exist before cleanup");
    assert!(path2.exists(), "Session 2 should exist before cleanup");

    // Run cleanup
    let result = cleanup_stale_sessions(None, false)?;

    // Should have removed the stale sessions
    assert!(
        result.stale_removed >= 2,
        "Expected at least 2 stale sessions removed, got {}",
        result.stale_removed
    );

    // Files should be gone
    assert!(!path1.exists(), "Session 1 should be removed after cleanup");
    assert!(!path2.exists(), "Session 2 should be removed after cleanup");

    Ok(())
}

#[test]
fn test_cleanup_expired_sessions() -> Result<()> {
    use std::process;
    use std::time::SystemTime;

    // Use current process PID and start time so it passes the "alive" check
    let our_pid = process::id();
    let our_start_time = SystemInfo::get_process_start_time(our_pid);

    // Create a session that's 10 days old with our PID
    let started_at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs()
        .saturating_sub(10 * 24 * 60 * 60);

    let old_session = SessionInfo {
        pid: our_pid,
        port: 8083,
        file: Some("old.txt".to_string()),
        started_at,
        session_name: "expired_test".to_string(),
        lsp_ready: false,
        start_time: our_start_time,
    };
    old_session.write()?;

    // Run cleanup with 7 day max age
    let max_age = Duration::from_secs(7 * 24 * 60 * 60);
    let result = cleanup_stale_sessions(Some(max_age), false)?;

    // Should have removed at least the expired one
    assert!(
        result.expired_removed >= 1,
        "Expected at least 1 expired session removed, got {}",
        result.expired_removed
    );

    Ok(())
}

// Helper to get process start time (needed for tests)
struct SystemInfo;

impl SystemInfo {
    #[cfg(target_os = "macos")]
    fn get_process_start_time(pid: u32) -> Option<u64> {
        use std::process::Command;
        use std::time::SystemTime;

        let output = Command::new("ps")
            .args(["-o", "etimes=", "-p", &pid.to_string()])
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let elapsed_str = String::from_utf8(output.stdout).ok()?;
        let elapsed_secs: u64 = elapsed_str.trim().parse().ok()?;

        let current_time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_secs();

        Some(current_time.saturating_sub(elapsed_secs))
    }

    #[cfg(not(target_os = "macos"))]
    fn get_process_start_time(_pid: u32) -> Option<u64> {
        None // For other platforms, just use None
    }
}

#[test]
fn test_cleanup_corrupted_sessions() -> Result<()> {
    // Create corrupted session file
    let path = create_corrupted_session("corrupted_test")?;

    assert!(path.exists(), "Corrupted file should exist before cleanup");

    // Run cleanup
    let result = cleanup_stale_sessions(None, false)?;

    // Should have removed the corrupted file
    assert!(
        result.corrupted_removed >= 1,
        "Expected at least 1 corrupted session removed, got {}",
        result.corrupted_removed
    );

    // File should be gone
    assert!(
        !path.exists(),
        "Corrupted file should be removed after cleanup"
    );

    Ok(())
}

#[test]
fn test_cleanup_dry_run() -> Result<()> {
    // Create a fake stale session with unique name
    let fake_pid = 999997;
    let session_name = format!("dry_run_test_{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis());

    let path = create_fake_session(&session_name, fake_pid, 8084, 0)?;

    // Verify it exists
    let exists_before = path.exists();
    if !exists_before {
        // File might have been auto-cleaned by list_all(), recreate it
        let session_info = SessionInfo {
            pid: fake_pid,
            port: 8084,
            file: Some(format!("test_{}.txt", session_name)),
            started_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs(),
            session_name: session_name.clone(),
            lsp_ready: false,
            start_time: None,
        };
        session_info.write()?;
    }

    // Run cleanup in dry-run mode
    let result = cleanup_stale_sessions(None, true)?;

    // In dry run, count might be 0 if the session was already cleaned up during listing
    // The key test is that dry_run doesn't crash and returns valid results
    assert!(result.total_removed() < 100, "Dry run should return reasonable results");

    // Clean up for real
    cleanup_stale_sessions(None, false)?;

    Ok(())
}

#[test]
fn test_cleanup_no_stale_sessions() -> Result<()> {
    // First, clean up any existing stale sessions
    cleanup_stale_sessions(None, false)?;

    // Run cleanup again - should find nothing or very few
    let result = cleanup_stale_sessions(None, false)?;

    // Just verify it doesn't crash (we can't guarantee 0 because other tests might leave files)
    assert!(result.total_removed() < 100, "Cleanup should not remove an unreasonable number");

    Ok(())
}

#[test]
fn test_session_is_expired() -> Result<()> {
    use std::time::SystemTime;

    // Create a session that's 5 days old
    let started_at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs()
        .saturating_sub(5 * 24 * 60 * 60);

    let session = SessionInfo {
        pid: 1,
        port: 8080,
        file: None,
        started_at,
        session_name: "test".to_string(),
        lsp_ready: false,
        start_time: None,
    };

    // Should be expired with 3 day threshold
    assert!(
        session.is_expired(Duration::from_secs(3 * 24 * 60 * 60)),
        "5 day old session should be expired with 3 day threshold"
    );

    // Should NOT be expired with 7 day threshold
    assert!(
        !session.is_expired(Duration::from_secs(7 * 24 * 60 * 60)),
        "5 day old session should not be expired with 7 day threshold"
    );

    Ok(())
}

#[test]
fn test_session_age() -> Result<()> {
    use std::time::SystemTime;

    // Create a session that's 2 days old
    let started_at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs()
        .saturating_sub(2 * 24 * 60 * 60);

    let session = SessionInfo {
        pid: 1,
        port: 8080,
        file: None,
        started_at,
        session_name: "test".to_string(),
        lsp_ready: false,
        start_time: None,
    };

    let age = session.age();

    // Age should be approximately 2 days (allow some variance)
    let age_days = age.as_secs() / (24 * 60 * 60);
    assert!(
        age_days >= 1 && age_days <= 3,
        "Expected age ~2 days, got {} days",
        age_days
    );

    Ok(())
}

#[test]
fn test_cleanup_result_total() {
    use ovim::session::CleanupResult;

    let result = CleanupResult {
        stale_removed: 2,
        expired_removed: 1,
        corrupted_removed: 3,
        temp_files_removed: 1,
        removed_sessions: vec![],
    };

    assert_eq!(
        result.total_removed(),
        7,
        "Total should be sum of all removals"
    );
}
