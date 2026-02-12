mod helpers;

use helpers::EditorTest;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use tempfile::NamedTempFile;

// ── Buffer-level reload tests (OV-00099 through OV-00109) ────────────

/// OV-00099/OV-00100: reload_from_disk() clears undo history
#[test]
fn test_reload_clears_undo_history() {
    let mut tmp = NamedTempFile::new().unwrap();
    write!(tmp, "original\n").unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    let mut test = EditorTest::new("original\n");
    test.set_file_path(path);

    // Make an edit that creates undo history
    test.keys("iextra<Esc>");
    assert_ne!(test.buffer_content(), "original\n");

    // Reload from disk
    test.editor.buffer_mut().reload_from_disk().unwrap();

    // Undo should return false — history was cleared
    assert!(
        !test.editor.buffer_mut().undo(),
        "undo should return false after reload"
    );

    // Content should match disk
    assert_eq!(test.buffer_content(), "original\n");
}

/// OV-00103: reload_from_disk() resets folds
#[test]
fn test_reload_resets_folds() {
    let mut tmp = NamedTempFile::new().unwrap();
    write!(tmp, "line1\nline2\nline3\nline4\n").unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    let mut test = EditorTest::new("line1\nline2\nline3\nline4\n");
    test.set_file_path(path);

    // Create a fold
    test.editor.buffer_mut().create_fold(1, 2);
    assert!(
        !test.editor.buffer().fold_manager().folds().is_empty(),
        "fold should exist before reload"
    );

    // Reload
    test.editor.buffer_mut().reload_from_disk().unwrap();

    // Folds should be gone
    assert!(
        test.editor.buffer().fold_manager().folds().is_empty(),
        "folds should be cleared after reload"
    );
}

/// OV-00104: reload_from_disk() bumps version counter
#[test]
fn test_reload_bumps_version() {
    let mut tmp = NamedTempFile::new().unwrap();
    write!(tmp, "content\n").unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    let mut test = EditorTest::new("content\n");
    test.set_file_path(path);

    let version_before = test.editor.buffer().version();

    test.editor.buffer_mut().reload_from_disk().unwrap();

    assert!(
        test.editor.buffer().version() > version_before,
        "version should increase after reload (was {}, now {})",
        version_before,
        test.editor.buffer().version()
    );
}

/// OV-00099/OV-00100: reload clears modified flag (via reset_derived_state replacing ChangeManager)
#[test]
fn test_reload_clears_modified_flag() {
    let mut tmp = NamedTempFile::new().unwrap();
    write!(tmp, "clean\n").unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    let mut test = EditorTest::new("clean\n");
    test.set_file_path(path);

    // Make edits → buffer becomes modified
    test.keys("ichanged<Esc>");
    assert!(
        test.editor.buffer().is_modified(),
        "buffer should be modified after editing"
    );

    // Reload
    test.editor.buffer_mut().reload_from_disk().unwrap();

    assert!(
        !test.editor.buffer().is_modified(),
        "buffer should not be modified after reload"
    );
}

/// OV-00106: reload clamps cursor when file becomes shorter
#[test]
fn test_reload_clamps_cursor_to_shorter_file() {
    let mut tmp = NamedTempFile::new().unwrap();
    write!(tmp, "line1\nline2\nline3\nline4\nline5\n").unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    let mut test = EditorTest::new("line1\nline2\nline3\nline4\nline5\n");
    test.set_file_path(path.clone());

    // Move cursor to last line
    test.keys("G");
    assert_eq!(test.cursor().0, 4, "cursor should be on line 4 (0-indexed)");

    // Overwrite file with shorter content
    std::fs::write(&path, "short\n").unwrap();

    // Reload
    test.editor.buffer_mut().reload_from_disk().unwrap();

    // Cursor should be clamped to new bounds
    let (line, _col) = test.cursor();
    let max_line = test.editor.buffer().line_count().saturating_sub(1);
    assert!(
        line <= max_line,
        "cursor line {} should be <= max line {} after reload of shorter file",
        line,
        max_line,
    );
}

// ── Editor-level command tests (OV-00101, OV-00102) ──────────────────

/// OV-00101: :e on modified buffer returns error
#[test]
fn test_e_command_checks_unsaved() {
    let mut test = EditorTest::new("hello\n");

    // Make an edit so buffer is dirty
    test.keys("ichange<Esc>");
    assert!(test.editor.is_modified());

    // Try :e — should set status error, buffer should remain changed
    test.keys(":e<Enter>");

    // Buffer content should still have the edit (command was rejected)
    assert!(test.buffer_content().contains("change"));
}

/// Save preserves file permissions (overwrite-in-place strategy)
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_save_preserves_file_permissions() {
    let mut tmp = NamedTempFile::new().unwrap();
    write!(tmp, "#!/bin/bash\necho hello\n").unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    // Set executable permissions (0o755)
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut test = EditorTest::new("");
    test.editor.load_file(&path).unwrap();

    // Make an edit and save
    test.keys("Goecho world<Esc>");
    test.editor.buffer_mut().save().unwrap();

    // Permissions should be preserved
    let metadata = std::fs::metadata(&path).unwrap();
    let mode = metadata.permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o755,
        "file permissions should be preserved after save, got {:o}",
        mode
    );
}

/// Save preserves restrictive permissions (0o600)
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_save_preserves_restrictive_permissions() {
    let mut tmp = NamedTempFile::new().unwrap();
    write!(tmp, "secret=hunter2\n").unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    // Set restrictive permissions (0o600 — owner read/write only)
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

    let mut test = EditorTest::new("");
    test.editor.load_file(&path).unwrap();

    // Edit and save
    test.keys("Gosecret2=value<Esc>");
    test.editor.buffer_mut().save().unwrap();

    let metadata = std::fs::metadata(&path).unwrap();
    let mode = metadata.permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "restrictive permissions should be preserved after save, got {:o}",
        mode
    );
}

/// OV-00102: :e! on modified buffer succeeds by reloading from disk
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_e_bang_command_reloads_discarding_changes() {
    let mut tmp = NamedTempFile::new().unwrap();
    write!(tmp, "original\n").unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    let mut test = EditorTest::new("");
    test.editor.load_file(&path).unwrap();

    // Make an edit
    test.keys("ichange<Esc>");
    assert!(test.editor.is_modified());

    // :e! should succeed, discarding changes
    test.keys(":e!<Enter>");

    // Buffer should be back to original content
    assert_eq!(test.buffer_content(), "original\n");
    assert!(!test.editor.is_modified());
}
