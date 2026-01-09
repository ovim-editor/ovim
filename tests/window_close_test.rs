mod helpers;

use helpers::EditorTest;

/// Test <C-w>c - close window (should fail on single window)
#[test]
fn test_window_close_single_window() {
    let mut test = EditorTest::new("line1\nline2\n");

    // Initialize window manager
    test.editor.init_window_manager(80, 24);

    // Try to close the only window
    test.keys("<C-w>c");

    // Should still have 1 window (close failed silently)
    assert_eq!(test.editor.window_count(), 1);
}

/// Test <C-w>c - close window after split
#[test]
fn test_window_close_after_split() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    // Initialize window manager
    test.editor.init_window_manager(80, 24);

    // Split horizontally
    test.keys("<C-w>s");
    assert_eq!(test.editor.window_count(), 2);

    // Close one window
    test.keys("<C-w>c");

    // Should have 1 window left
    assert_eq!(test.editor.window_count(), 1);
}

/// Test <C-w>q - quit on single window
#[test]
fn test_window_quit_single_window() {
    let mut test = EditorTest::new("line1\n");

    // Initialize window manager
    test.editor.init_window_manager(80, 24);

    // Quit command on single window should set should_quit
    test.keys("<C-w>q");

    assert!(test.editor.should_quit());
}

/// Test <C-w>q - close window after split
#[test]
fn test_window_quit_after_split() {
    let mut test = EditorTest::new("line1\nline2\n");

    // Initialize window manager
    test.editor.init_window_manager(80, 24);

    // Split vertically
    test.keys("<C-w>v");
    assert_eq!(test.editor.window_count(), 2);

    // Quit should close one window, not exit editor
    test.keys("<C-w>q");

    assert_eq!(test.editor.window_count(), 1);
    assert!(!test.editor.should_quit());
}

/// Test closing multiple windows in sequence
#[test]
fn test_close_multiple_windows_sequence() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    // Initialize window manager
    test.editor.init_window_manager(80, 24);

    // Create 3 windows with two splits
    test.keys("<C-w>s");
    test.keys("<C-w>s");
    assert_eq!(test.editor.window_count(), 3);

    // Close first window
    test.keys("<C-w>c");
    assert_eq!(test.editor.window_count(), 2);

    // Close second window
    test.keys("<C-w>c");
    assert_eq!(test.editor.window_count(), 1);

    // Try to close last window (should fail)
    test.keys("<C-w>c");
    assert_eq!(test.editor.window_count(), 1);
}

/// Test <C-w>o - close other windows (single window)
#[test]
fn test_window_only_single_window() {
    let mut test = EditorTest::new("line1\nline2\n");

    // Initialize window manager
    test.editor.init_window_manager(80, 24);

    // Try to close other windows when only one exists (should be idempotent)
    test.keys("<C-w>o");

    // Should still have 1 window
    assert_eq!(test.editor.window_count(), 1);
}

/// Test <C-w>o - close other windows after split
#[test]
fn test_window_only_after_split() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    // Initialize window manager
    test.editor.init_window_manager(80, 24);

    // Split horizontally
    test.keys("<C-w>s");
    assert_eq!(test.editor.window_count(), 2);

    // Close other windows
    test.keys("<C-w>o");

    // Should have only 1 window
    assert_eq!(test.editor.window_count(), 1);
}

/// Test <C-w>o - close other windows with multiple splits
#[test]
fn test_window_only_multiple_splits() {
    let mut test = EditorTest::new("line1\nline2\nline3\nline4\n");

    // Initialize window manager
    test.editor.init_window_manager(80, 24);

    // Create 4 windows
    test.keys("<C-w>s");
    test.keys("<C-w>s");
    test.keys("<C-w>s");
    assert_eq!(test.editor.window_count(), 4);

    // Move cursor to line 2 in the focused window
    test.keys("2G");
    assert_eq!(test.cursor(), (1, 0));

    // Close all other windows
    test.keys("<C-w>o");

    // Should have only 1 window
    assert_eq!(test.editor.window_count(), 1);

    // Cursor position should be preserved
    assert_eq!(test.cursor(), (1, 0));
}

/// Test :only command (single window)
#[test]
fn test_only_command_single_window() {
    let mut test = EditorTest::new("line1\nline2\n");

    // Initialize window manager
    test.editor.init_window_manager(80, 24);

    // Execute :only command
    test.keys(":only<Enter>");

    // Should still have 1 window
    assert_eq!(test.editor.window_count(), 1);
}

/// Test :only command after split
#[test]
fn test_only_command_after_split() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    // Initialize window manager
    test.editor.init_window_manager(80, 24);

    // Split vertically
    test.keys("<C-w>v");
    assert_eq!(test.editor.window_count(), 2);

    // Execute :only command
    test.keys(":only<Enter>");

    // Should have only 1 window
    assert_eq!(test.editor.window_count(), 1);
}

/// Test :on abbreviation
#[test]
fn test_on_abbreviation() {
    let mut test = EditorTest::new("line1\nline2\n");

    // Initialize window manager
    test.editor.init_window_manager(80, 24);

    // Split horizontally twice
    test.keys("<C-w>s");
    test.keys("<C-w>s");
    assert_eq!(test.editor.window_count(), 3);

    // Execute :on command (abbreviation of :only)
    test.keys(":on<Enter>");

    // Should have only 1 window
    assert_eq!(test.editor.window_count(), 1);
}
