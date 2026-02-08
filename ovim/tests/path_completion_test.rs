mod helpers;
use helpers::EditorTest;
use ovim::mode::Mode;
use ovim_core::KeyCode;
use std::fs;
use tempfile::TempDir;

/// Create a test directory structure for completion tests.
fn setup_test_dir() -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    // Files
    fs::write(tmp.path().join("main.rs"), "fn main() {}").unwrap();
    fs::write(tmp.path().join("lib.rs"), "pub mod foo;").unwrap();
    fs::write(tmp.path().join("README.md"), "# Hello").unwrap();
    // Directories
    fs::create_dir(tmp.path().join("src")).unwrap();
    fs::write(tmp.path().join("src").join("app.rs"), "").unwrap();
    fs::write(tmp.path().join("src").join("buffer.rs"), "").unwrap();
    fs::create_dir(tmp.path().join("tests")).unwrap();
    fs::write(tmp.path().join("tests").join("test1.rs"), "").unwrap();
    // Hidden files
    fs::write(tmp.path().join(".gitignore"), "target/").unwrap();
    fs::create_dir(tmp.path().join(".git")).unwrap();
    tmp
}

/// Helper: enter command mode, type `:e <abs_path>` with the temp dir as prefix.
fn enter_edit_with_abs_path(t: &mut EditorTest, tmp: &TempDir, suffix: &str) {
    t.press(':');
    let cmd = format!("e {}/{}", tmp.path().display(), suffix);
    t.type_text(&cmd);
}

/// Helper: enter command mode, type a command with absolute path.
fn enter_cmd_with_abs_path(t: &mut EditorTest, cmd_prefix: &str, tmp: &TempDir, suffix: &str) {
    t.press(':');
    let cmd = format!("{}{}/{}", cmd_prefix, tmp.path().display(), suffix);
    t.type_text(&cmd);
}

// ============================================================================
// Basic completion triggering
// ============================================================================

#[test]
fn test_tab_triggers_completion_popup() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    enter_edit_with_abs_path(&mut t, &tmp, "");

    t.press_key(KeyCode::Tab);

    assert!(
        t.editor.path_completion().is_visible(),
        "Tab after `:e <dir>/` should show completion popup"
    );
    assert!(
        !t.editor.path_completion().entries().is_empty(),
        "Popup should have entries for test dir"
    );
}

#[test]
fn test_tab_fills_first_entry() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    enter_edit_with_abs_path(&mut t, &tmp, "");

    t.press_key(KeyCode::Tab);

    let cmd = t.editor.command_line().to_string();
    assert!(
        cmd.starts_with("e "),
        "Command should still start with 'e '"
    );
    let path_portion = &cmd[2..];
    assert!(
        !path_portion.is_empty(),
        "Tab should fill in a completion entry"
    );
}

#[test]
fn test_tab_does_nothing_for_non_file_command() {
    let mut t = EditorTest::new("hello");
    t.press(':');
    t.type_text("set number");
    t.press_key(KeyCode::Tab);

    assert!(
        !t.editor.path_completion().is_visible(),
        "Tab on `:set number` should not show file completion"
    );
}

#[test]
fn test_tab_does_nothing_for_bare_command() {
    let mut t = EditorTest::new("hello");
    t.press(':');
    t.type_text("e");
    // No space after 'e'
    t.press_key(KeyCode::Tab);

    assert!(
        !t.editor.path_completion().is_visible(),
        "Tab on `:e` (no space) should not trigger completion"
    );
}

// ============================================================================
// Tab cycling
// ============================================================================

#[test]
fn test_tab_cycles_through_entries() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    enter_edit_with_abs_path(&mut t, &tmp, "");

    // First tab
    t.press_key(KeyCode::Tab);
    let first_cmd = t.editor.command_line().to_string();

    // Second tab — should change to next entry
    t.press_key(KeyCode::Tab);
    let second_cmd = t.editor.command_line().to_string();

    // They might differ if first was a dir (triggers refresh) or same-level next
    // Either way the command line should have changed
    assert_ne!(
        first_cmd, second_cmd,
        "Second Tab should cycle to a different entry"
    );
}

#[test]
fn test_backtab_cycles_backward() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    // Use src/ dir which has exactly 2 files (app.rs, buffer.rs)
    enter_edit_with_abs_path(&mut t, &tmp, "src/");

    // Tab to get entry 0 (app.rs)
    t.press_key(KeyCode::Tab);
    let entry_0 = t.editor.command_line().to_string();

    // Tab to get entry 1 (buffer.rs)
    t.press_key(KeyCode::Tab);
    let entry_1 = t.editor.command_line().to_string();

    assert_ne!(entry_0, entry_1, "Tab should cycle entries");

    // BackTab to go back to entry 0
    t.press_key(KeyCode::BackTab);
    let after_backtab = t.editor.command_line().to_string();

    assert_eq!(
        entry_0, after_backtab,
        "BackTab should go back to previous entry"
    );
}

// ============================================================================
// Directory completion
// ============================================================================

#[test]
fn test_tab_into_directory_appends_slash_and_refreshes() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    enter_edit_with_abs_path(&mut t, &tmp, "");

    t.press_key(KeyCode::Tab);
    let cmd = t.editor.command_line().to_string();
    let path = &cmd[2..]; // strip "e "

    // First entry should be a dir (dirs sort first) with trailing slash
    assert!(
        path.ends_with('/'),
        "Directory completion should append '/': got {:?}",
        path
    );

    // Popup should still be visible with the directory's contents
    assert!(
        t.editor.path_completion().is_visible(),
        "After completing a directory, popup should refresh with its contents"
    );
}

// ============================================================================
// Prefix filtering
// ============================================================================

#[test]
fn test_typing_prefix_filters_entries() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    enter_edit_with_abs_path(&mut t, &tmp, "m");

    assert!(t.editor.path_completion().is_visible());
    let entries = t.editor.path_completion().entries();
    for entry in entries {
        assert!(
            entry.name.to_lowercase().starts_with("m"),
            "Entry {:?} should start with 'm'",
            entry.name
        );
    }
}

#[test]
fn test_typing_narrows_completion() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    // Start with all entries visible
    enter_edit_with_abs_path(&mut t, &tmp, "");

    let all_count = t.editor.path_completion().entries().len();

    // Type 'R' to narrow to README.md
    t.press('R');
    let filtered_count = t.editor.path_completion().entries().len();

    assert!(
        filtered_count < all_count,
        "Typing a prefix should narrow results: all={}, filtered={}",
        all_count,
        filtered_count
    );
}

// ============================================================================
// Enter behavior
// ============================================================================

// Note: Enter with `:e`/`:w` commands requires a Tokio runtime (file I/O is async).
// Popup hiding on Enter is implicitly tested by the command mode flow — Enter always
// calls `path_completion_mut().hide()` before executing. The Esc test covers the
// popup-specific hiding behavior without needing async infrastructure.

// ============================================================================
// Escape dismisses popup
// ============================================================================

#[test]
fn test_esc_hides_popup_and_returns_to_normal() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    enter_edit_with_abs_path(&mut t, &tmp, "");
    t.press_key(KeyCode::Tab);

    assert!(t.editor.path_completion().is_visible());

    t.press_esc();

    assert!(!t.editor.path_completion().is_visible());
    t.assert_mode(Mode::Normal);
}

// ============================================================================
// Arrow key behavior
// ============================================================================

#[test]
fn test_up_down_navigate_popup_when_visible() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    // Use src/ which has exactly 2 known files
    enter_edit_with_abs_path(&mut t, &tmp, "src/");

    // Trigger popup (don't use Tab which accepts and may change context)
    t.press_key(KeyCode::Tab);

    let visible = t.editor.path_completion().is_visible();
    let count = t.editor.path_completion().entries().len();
    assert!(visible, "Popup should be visible");
    assert!(count >= 2, "Need >= 2 entries, got {}", count);

    let initial = t.editor.path_completion().selected_index();

    // Down arrow should change selection
    t.press_key(KeyCode::Down);
    let after_down = t.editor.path_completion().selected_index();

    assert_ne!(
        initial, after_down,
        "Down arrow should change selection (initial={}, after={})",
        initial, after_down,
    );

    // Up arrow should go back
    t.press_key(KeyCode::Up);
    let after_up = t.editor.path_completion().selected_index();

    assert_eq!(
        initial, after_up,
        "Up after Down should return to original selection"
    );
}

#[test]
fn test_arrows_do_not_crash_when_popup_hidden() {
    let mut t = EditorTest::new("hello");
    t.press(':');
    t.type_text("e nonexistent_xyz_");

    // Popup should not be visible (no matches)
    t.press_key(KeyCode::Up);
    t.press_key(KeyCode::Down);
    // Just verify no crash
}

// ============================================================================
// Bug: Up/Down don't update command line text
// ============================================================================

#[test]
fn test_arrow_selection_does_not_update_command_line() {
    // Documents current behavior: Up/Down change visual selection
    // but do NOT update the command line text. This means Enter after
    // arrow navigation executes the Tab-selected entry, not the
    // arrow-selected one.
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    // Use a dir with multiple files
    enter_edit_with_abs_path(&mut t, &tmp, "src/");
    t.press_key(KeyCode::Tab);

    let cmd_after_tab = t.editor.command_line().to_string();

    // Down arrow changes visual selection but NOT command line
    t.press_key(KeyCode::Down);
    let cmd_after_down = t.editor.command_line().to_string();

    assert_eq!(
        cmd_after_tab, cmd_after_down,
        "Current behavior: Down arrow does not update command line text"
    );
}

// ============================================================================
// Backspace interaction
// ============================================================================

#[test]
fn test_backspace_updates_completion() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    enter_edit_with_abs_path(&mut t, &tmp, "ma");

    let narrow_count = t.editor.path_completion().entries().len();

    // Backspace removes 'a', widening to 'm' prefix
    t.press_backspace();
    let wider_count = t.editor.path_completion().entries().len();

    assert!(
        wider_count >= narrow_count,
        "Backspace should widen completion: narrow={}, wider={}",
        narrow_count,
        wider_count
    );
}

#[test]
fn test_backspace_past_space_hides_completion() {
    let _tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    t.press(':');
    t.type_text("e ");

    assert!(t.editor.path_completion().is_visible());

    // Backspace removes the space -> "e" -> no match
    t.press_backspace();

    assert!(
        !t.editor.path_completion().is_visible(),
        "Backspace removing the space after command should hide popup"
    );
}

// ============================================================================
// Subdirectory navigation
// ============================================================================

#[test]
fn test_typing_subpath() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    enter_edit_with_abs_path(&mut t, &tmp, "src/");

    assert!(t.editor.path_completion().is_visible());
    let entries = t.editor.path_completion().entries();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();

    assert!(
        names.contains(&"app.rs"),
        "Typing 'src/' should show src contents: {:?}",
        names
    );
    assert!(
        names.contains(&"buffer.rs"),
        "Typing 'src/' should show src contents: {:?}",
        names
    );
}

// ============================================================================
// Supported commands
// ============================================================================

#[test]
fn test_completion_works_for_write_command() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    enter_cmd_with_abs_path(&mut t, "w ", &tmp, "");

    assert!(
        t.editor.path_completion().is_visible(),
        ":w should trigger path completion (entries={})",
        t.editor.path_completion().entries().len(),
    );
}

#[test]
fn test_completion_works_for_split_commands() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    enter_cmd_with_abs_path(&mut t, "sp ", &tmp, "");

    assert!(
        t.editor.path_completion().is_visible(),
        ":sp should trigger path completion"
    );
}

#[test]
fn test_completion_works_for_tabe_command() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    enter_cmd_with_abs_path(&mut t, "tabe ", &tmp, "");

    assert!(
        t.editor.path_completion().is_visible(),
        ":tabe should trigger path completion"
    );
}

// ============================================================================
// Case insensitive matching
// ============================================================================

#[test]
fn test_case_insensitive_prefix_matching() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    // 'r' lowercase should match 'README.md'
    enter_edit_with_abs_path(&mut t, &tmp, "r");

    let entries = t.editor.path_completion().entries();
    let has_readme = entries.iter().any(|e| e.name == "README.md");
    assert!(
        has_readme,
        "Lowercase 'r' should match 'README.md': entries={:?}",
        entries.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_empty_directory_no_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let empty_dir = tmp.path().join("empty");
    fs::create_dir(&empty_dir).unwrap();

    let mut t = EditorTest::new("hello");
    t.press(':');
    t.type_text(&format!("e {}/", empty_dir.display()));

    assert!(
        !t.editor.path_completion().is_visible() || t.editor.path_completion().entries().is_empty(),
        "Empty directory should show no completion entries"
    );
}

#[test]
fn test_nonexistent_subdir_no_entries() {
    let mut t = EditorTest::new("hello");
    t.press(':');
    t.type_text("e /nonexistent_path_xyz_12345/");

    assert!(
        t.editor.path_completion().entries().is_empty(),
        "Nonexistent directory should produce no entries"
    );
}

#[test]
fn test_hidden_files_sort_after_visible_within_group() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    enter_edit_with_abs_path(&mut t, &tmp, "");

    let entries = t.editor.path_completion().entries();

    // Sort order: dirs first, then files. Within each group, non-hidden before hidden.
    // So: [non-hidden dirs, hidden dirs, non-hidden files, hidden files]
    let names: Vec<String> = entries
        .iter()
        .map(|e| format!("{}(dir={},hidden={})", e.name, e.is_dir, e.is_hidden))
        .collect();

    // Among directories: non-hidden dirs come before hidden dirs.
    let dirs: Vec<_> = entries.iter().filter(|e| e.is_dir).collect();
    let first_hidden_dir = dirs.iter().position(|e| e.is_hidden);
    let last_non_hidden_dir = dirs.iter().rposition(|e| !e.is_hidden);
    if let (Some(fh), Some(lnh)) = (first_hidden_dir, last_non_hidden_dir) {
        assert!(
            fh > lnh,
            "Hidden dirs should sort after non-hidden dirs: {:?}",
            names
        );
    }

    // Among files: non-hidden files come before hidden files.
    let files: Vec<_> = entries.iter().filter(|e| !e.is_dir).collect();
    let first_hidden_file = files.iter().position(|e| e.is_hidden);
    let last_non_hidden_file = files.iter().rposition(|e| !e.is_hidden);
    if let (Some(fh), Some(lnh)) = (first_hidden_file, last_non_hidden_file) {
        assert!(
            fh > lnh,
            "Hidden files should sort after non-hidden files: {:?}",
            names
        );
    }
}

#[test]
fn test_dirs_sort_before_files() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    enter_edit_with_abs_path(&mut t, &tmp, "");

    let entries = t.editor.path_completion().entries();
    let non_hidden: Vec<_> = entries.iter().filter(|e| !e.is_hidden).collect();

    let first_file_idx = non_hidden.iter().position(|e| !e.is_dir);
    let last_dir_idx = non_hidden.iter().rposition(|e| e.is_dir);

    if let (Some(ff), Some(ld)) = (first_file_idx, last_dir_idx) {
        assert!(
            ld < ff,
            "Directories should sort before files (among non-hidden)"
        );
    }
}

// ============================================================================
// Tab on src/ subdir (more controlled test)
// ============================================================================

#[test]
fn test_tab_in_known_subdir() {
    let tmp = setup_test_dir();
    let mut t = EditorTest::new("hello");
    enter_edit_with_abs_path(&mut t, &tmp, "src/");

    // Tab in src/ should fill first entry (app.rs, alphabetically first)
    t.press_key(KeyCode::Tab);

    let cmd = t.editor.command_line().to_string();
    let path = &cmd[2..]; // strip "e "
    assert!(
        path.ends_with("src/app.rs"),
        "Tab in src/ should complete to app.rs: got {:?}",
        path
    );

    // Second tab should give buffer.rs
    t.press_key(KeyCode::Tab);
    let cmd2 = t.editor.command_line().to_string();
    let path2 = &cmd2[2..];
    assert!(
        path2.ends_with("src/buffer.rs"),
        "Second Tab should complete to buffer.rs: got {:?}",
        path2
    );
}
