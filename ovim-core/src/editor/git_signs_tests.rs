//! Gutter signs and the status bar counts come from the file's diff against
//! HEAD. Every way a buffer takes its text from disk again must leave them
//! matching that diff.

use super::Editor;
use crate::git::LineStatus;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const COMMITTED: &str = "one\ntwo\nthree\nfour\nfive\n";
/// Line 2 changed, two lines added after line 5.
const WORKING: &str = "one\nTWO\nthree\nfour\nfive\nsix\nseven\n";

struct Repo {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

/// A repository with `main.txt` committed as [`COMMITTED`] and changed on
/// disk to [`WORKING`].
fn repo_with_uncommitted_change() -> Repo {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let repo = git2::Repository::init(&root).unwrap();
    std::fs::write(root.join("main.txt"), COMMITTED).unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("main.txt")).unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let signature = git2::Signature::now("Test", "t@example.com").unwrap();
    repo.commit(Some("HEAD"), &signature, &signature, "init", &tree, &[])
        .unwrap();
    std::fs::write(root.join("main.txt"), WORKING).unwrap();
    Repo { _dir: dir, root }
}

/// Drains background git refreshes until none is in flight.
fn settle_git_refresh(editor: &mut Editor) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while editor.git_refresh_pending() {
        editor.poll_git_refresh();
        assert!(Instant::now() < deadline, "the git refresh did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn assert_signs_match_working_diff(editor: &Editor, buffer_index: usize, after: &str) {
    let status = editor.buffer_at(buffer_index).unwrap().git_status();
    assert_eq!(
        status.change_counts(),
        (2, 1, 0),
        "after {after}: (added, modified, removed)"
    );
    assert_eq!(status.get_line_status(1), Some(LineStatus::Modified));
    assert_eq!(status.get_line_status(5), Some(LineStatus::Added));
    assert_eq!(status.get_line_status(6), Some(LineStatus::Added));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn forced_edit_reload_keeps_the_git_signs() {
    let repo = repo_with_uncommitted_change();
    let mut editor = Editor::default();
    editor.load_file(repo.root.join("main.txt")).unwrap();
    settle_git_refresh(&mut editor);
    let index = editor.current_buffer_index();
    assert_signs_match_working_diff(&editor, index, "opening the file");

    crate::commands::execute_command(&mut editor, "e!");
    settle_git_refresh(&mut editor);

    assert_signs_match_working_diff(&editor, index, ":e!");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn plain_edit_reload_keeps_the_git_signs() {
    let repo = repo_with_uncommitted_change();
    let mut editor = Editor::default();
    editor.load_file(repo.root.join("main.txt")).unwrap();
    settle_git_refresh(&mut editor);
    let index = editor.current_buffer_index();

    crate::commands::execute_command(&mut editor, "e");
    settle_git_refresh(&mut editor);

    assert_signs_match_working_diff(&editor, index, ":e");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn checktime_reload_recomputes_the_git_signs() {
    let repo = repo_with_uncommitted_change();
    let path = repo.root.join("main.txt");
    let mut editor = Editor::default();
    editor.load_file(&path).unwrap();
    settle_git_refresh(&mut editor);
    let index = editor.current_buffer_index();

    // Another process reverts the file and then makes the same change, so
    // the mtime moves and the diff against HEAD stays the one asserted.
    std::fs::write(&path, COMMITTED).unwrap();
    std::fs::write(&path, WORKING).unwrap();
    bump_mtime(&path);
    let result = crate::commands::execute_command(&mut editor, "checktime");
    let crate::command_result::CommandResult::Success(success) = result else {
        panic!(":checktime failed: {result:?}");
    };
    assert_eq!(
        success.message.as_deref(),
        Some("File reloaded from disk (external changes detected)")
    );
    settle_git_refresh(&mut editor);

    assert_signs_match_working_diff(&editor, index, ":checktime");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn background_buffer_reload_recomputes_the_git_signs() {
    let repo = repo_with_uncommitted_change();
    let path = repo.root.join("main.txt");
    std::fs::write(repo.root.join("other.txt"), "other\n").unwrap();
    let mut editor = Editor::default();
    editor.load_file(&path).unwrap();
    let index = editor.current_buffer_index();
    editor.load_file(repo.root.join("other.txt")).unwrap();
    settle_git_refresh(&mut editor);

    bump_mtime(&path);
    assert_eq!(
        editor.reload_background_buffers_changed_on_disk(),
        vec![path.to_string_lossy().to_string()]
    );
    settle_git_refresh(&mut editor);

    assert_signs_match_working_diff(&editor, index, "a background reload");
}

/// Moves the file's mtime forward so the external-change check sees a
/// change even on filesystems with coarse timestamps.
fn bump_mtime(path: &Path) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    let later = std::time::SystemTime::now() + Duration::from_secs(5);
    file.set_modified(later).unwrap();
}
