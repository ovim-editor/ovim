//! OV-00212: `Change::ResourceOp` undo/redo must surface filesystem errors
//! rather than silently swallowing them.
//!
//! Pre-fix, `restore_file_snapshot` used `let _ = fs::write(...)` and
//! friends, so AI-driven file operation undo would *appear* to succeed even
//! when permission-denied / disk-full prevented the bytes from actually
//! being written. This test exercises the failure path to ensure the error
//! propagates as `UndoOutcome::Failed` and the change stays on the undo
//! stack so the user can retry.

#![cfg(unix)]

mod helpers;

use helpers::EditorTest;
use ovim_core::change::{Change, CursorPos, UndoOutcome};
use ovim_core::editor::{ToastLevel, ToastSource};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn unique_temp_dir(name: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ovim_ov00212_{name}_{id}"))
}

/// Helper that flips a directory between writable and read-only so tests can
/// guarantee cleanup runs even when an assertion fails.
struct Chmod(PathBuf);

impl Drop for Chmod {
    fn drop(&mut self) {
        // Restore writability so tempdir can be removed; ignore errors during
        // cleanup — the test itself has already reported pass/fail.
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Writes-to-newly-created-path variant: snapshot.before is `Some(bytes)`,
/// so undo tries to write them back into a path whose parent directory is
/// locked. Since the file doesn't exist yet, `create_dir_all` would fall
/// through (no-op for an existing dir) and the open-for-write would fail
/// with PermissionDenied because the parent denies entry creation.
#[test]
fn undo_resource_op_surfaces_write_failure() {
    let dir = unique_temp_dir("write");
    std::fs::create_dir_all(&dir).unwrap();
    let _guard = Chmod(dir.clone());

    // Snapshot represents "file existed with 'before' bytes, was deleted
    // ('after' is None)". Undo will recreate it — but the parent is locked
    // so the create fails.
    let path = dir.join("recreated.txt");
    let snap = Change::resource_snapshot(path.clone(), Some(b"before".to_vec()), None);
    let change = Change::resource_op(vec![snap], CursorPos::ZERO, CursorPos::ZERO);

    // Lock the directory so creating new entries fails.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    let mut buffer = ovim_core::buffer::Buffer::new();
    let result = change.undo(&mut buffer);

    assert!(
        result.is_err(),
        "undo trying to recreate a file in a read-only dir should propagate the write error, got {result:?}"
    );
    assert!(
        !path.exists(),
        "no file should have appeared when the directory denies writes"
    );
}

/// Removes-locked-file variant: snapshot.before is `None`, so undo tries to
/// remove the file; the parent is locked so the unlink fails.
#[test]
fn undo_resource_op_surfaces_remove_failure() {
    let dir = unique_temp_dir("remove");
    std::fs::create_dir_all(&dir).unwrap();
    let _guard = Chmod(dir.clone());

    let path = dir.join("locked.txt");
    std::fs::write(&path, b"alive").unwrap();

    let snap = Change::resource_snapshot(path.clone(), None, Some(b"alive".to_vec()));
    let change = Change::resource_op(vec![snap], CursorPos::ZERO, CursorPos::ZERO);

    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    let mut buffer = ovim_core::buffer::Buffer::new();
    let result = change.undo(&mut buffer);

    assert!(
        result.is_err(),
        "undo on a locked unlink should propagate the remove error, got {result:?}"
    );
}

/// End-to-end check via the high-level Editor wrapper: a failing undo must
/// (a) leave the change on the undo stack so retry is possible, and
/// (b) surface a sticky error toast tagged System.
#[test]
fn editor_undo_failure_keeps_change_on_stack_and_toasts() {
    let dir = unique_temp_dir("editor");
    std::fs::create_dir_all(&dir).unwrap();
    let _guard = Chmod(dir.clone());

    // Same shape as `undo_resource_op_surfaces_write_failure`: snapshot says
    // file existed before with these bytes and was deleted; undo recreates
    // it, parent dir is locked, write fails.
    let path = dir.join("recreated.txt");
    let snap = Change::resource_snapshot(path.clone(), Some(b"before".to_vec()), None);
    let change = Change::resource_op(vec![snap], CursorPos::ZERO, CursorPos::ZERO);

    let mut test = EditorTest::new("dummy\n");
    test.editor
        .buffer_mut()
        .change_manager_mut()
        .push_undo_change_preserving_repeat(change);

    assert_eq!(
        test.editor.buffer().change_manager().undo_stack.len(),
        1,
        "test setup: ResourceOp seeded onto undo stack"
    );

    // Lock the directory now that the change is staged.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    test.editor.undo();

    // Change is restored to the undo stack on failure — user can retry once
    // they fix permissions.
    assert_eq!(
        test.editor.buffer().change_manager().undo_stack.len(),
        1,
        "failed undo must leave the change on the undo stack so a retry is possible"
    );
    assert_eq!(
        test.editor.buffer().change_manager().redo_stack.len(),
        0,
        "failed undo must NOT rotate the change onto the redo stack"
    );

    // Toast was pushed; user sees the failure rather than silent corruption.
    let toasts = test.editor.visible_toasts_newest_first(8);
    assert!(
        toasts
            .iter()
            .any(|t| matches!(t.level, ToastLevel::Error)
                && matches!(t.source, ToastSource::System)
                && t.message.starts_with("Undo failed:")),
        "expected a System Error toast starting with 'Undo failed:', got {:?}",
        toasts
            .iter()
            .map(|t| (t.source, t.level, &t.message))
            .collect::<Vec<_>>()
    );
}

/// Sanity: control test confirming that the same undo flow succeeds and is
/// silent (no toast, change moves to redo stack) when the filesystem is
/// healthy. Catches false positives where the "failure" assertion would also
/// hold for the success path.
#[test]
fn editor_undo_success_path_stays_silent() {
    let dir = unique_temp_dir("ok");
    std::fs::create_dir_all(&dir).unwrap();
    let _guard = Chmod(dir.clone());

    let path = dir.join("file.txt");
    std::fs::write(&path, b"after").unwrap();

    let snap = Change::resource_snapshot(path.clone(), Some(b"before".to_vec()), Some(b"after".to_vec()));
    let change = Change::resource_op(vec![snap], CursorPos::ZERO, CursorPos::ZERO);

    let mut test = EditorTest::new("dummy\n");
    test.editor
        .buffer_mut()
        .change_manager_mut()
        .push_undo_change_preserving_repeat(change);

    test.editor.undo();

    assert_eq!(
        test.editor.buffer().change_manager().undo_stack.len(),
        0,
        "successful undo moves change off the undo stack"
    );
    assert_eq!(
        test.editor.buffer().change_manager().redo_stack.len(),
        1,
        "successful undo rotates change to the redo stack"
    );

    let on_disk = std::fs::read(&path).unwrap();
    assert_eq!(on_disk, b"before", "successful undo restored snapshot bytes");

    let toasts = test.editor.visible_toasts_newest_first(8);
    assert!(
        toasts.is_empty(),
        "successful undo should not push a toast, got {:?}",
        toasts.iter().map(|t| &t.message).collect::<Vec<_>>()
    );
}

/// Direct check on `UndoOutcome` plumbing — touched_buffer/is_done semantics.
#[test]
fn undo_outcome_helpers_distinguish_states() {
    let nothing = UndoOutcome::Nothing;
    assert!(!nothing.touched_buffer());
    assert!(!nothing.is_done());

    let done = UndoOutcome::Done;
    assert!(done.touched_buffer());
    assert!(done.is_done());

    let failed = UndoOutcome::Failed(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "x"));
    assert!(
        failed.touched_buffer(),
        "Failed must invalidate caches — partial state may exist on disk"
    );
    assert!(!failed.is_done());
}
