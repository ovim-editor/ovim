//! Leader keys and ex commands for the project navigation pickers.

mod helpers;

use helpers::EditorTest;
use ovim_core::Mode;
use std::fs;

fn project(files: &[&str]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    fs::create_dir(root.join(".git")).unwrap();
    for file in files {
        fs::write(root.join(file), "text\n").unwrap();
    }
    (dir, root)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn leader_keys_open_the_recent_buffer_and_symbol_pickers() {
    let (_dir, root) = project(&["a.txt", "b.txt"]);
    let mut test = EditorTest::new("");
    test.load_file(&root.join("a.txt").to_string_lossy());
    test.editor.track_recent_file();
    test.load_file(&root.join("b.txt").to_string_lossy());
    test.editor.track_recent_file();

    test.keys(" sh");
    test.assert_mode(Mode::Picker);
    assert_eq!(test.editor.picker().unwrap().title(), Some("Recent files"));
    assert_eq!(
        test.editor
            .picker()
            .unwrap()
            .filtered_result(0)
            .unwrap()
            .display,
        "a.txt",
        "the current file is not offered, the previous one is first"
    );
    test.press_esc();

    test.keys(" sb");
    test.assert_mode(Mode::Picker);
    assert_eq!(test.editor.picker().unwrap().title(), Some("Open buffers"));
    test.press_esc();

    test.keys(" sS");
    test.assert_mode(Mode::Picker);
    assert!(test.editor.picker().unwrap().is_symbol_search());
    test.press_esc();

    test.keys(" S");
    test.assert_mode(Mode::Picker);
    assert!(test.editor.picker().unwrap().is_symbol_search());
    test.press_esc();

    test.command("Recent");
    test.assert_mode(Mode::Picker);
    test.press_esc();
    test.command("Buffers");
    test.assert_mode(Mode::Picker);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn recent_files_says_so_when_there_is_nothing_to_offer() {
    let (_dir, root) = project(&["only.txt"]);
    let mut test = EditorTest::new("");
    test.load_file(&root.join("only.txt").to_string_lossy());
    test.command("Recent");
    test.assert_mode(Mode::Normal);
    assert!(test.editor.status_message().contains("No recent files"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn problems_command_and_key_report_when_there_is_nothing_to_show() {
    let (_dir, root) = project(&["a.txt"]);
    let mut test = EditorTest::new("");
    test.load_file(&root.join("a.txt").to_string_lossy());
    test.keys(" sd");
    test.assert_mode(Mode::Normal);
    assert!(
        test.editor.status_message().contains("no language server"),
        "{}",
        test.editor.status_message()
    );
    test.command("Problems bogus");
    assert!(test
        .editor
        .status_message()
        .contains("use all, warnings or errors"));
}
