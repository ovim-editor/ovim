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
