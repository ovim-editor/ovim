mod helpers;
use helpers::EditorTest;

#[test]
fn test_zt_is_idempotent() {
    // Create a buffer with 50 lines
    let content = (1..=50).map(|i| format!("Line {}", i)).collect::<Vec<_>>().join("\n");
    let mut test = EditorTest::new(&content);

    // Initialize window manager with a known viewport size
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);

    // Move to line 25 (middle of buffer) - use 24j to go down 24 lines
    test.keys("24j");

    // Verify we're on line 24 (0-indexed)
    assert_eq!(test.editor.buffer().cursor().line(), 24);
    eprintln!("Starting position: line {}", test.editor.buffer().cursor().line());
    eprintln!("Starting Editor.scroll_offset: {}", test.editor.scroll_offset());
    if let Some(wm) = test.editor.window_manager() {
        if let Some(window) = wm.focused_window() {
            eprintln!("Starting Window.scroll_offset: {}", window.scroll_offset());
            eprintln!("Starting Window.cursor: {:?}", window.cursor());
        }
    }

    // First zt - should position line 24 at top
    eprintln!("\n=== FIRST ZT ===");
    test.press('z');
    eprintln!("After 'z': Editor.scroll_offset = {}", test.editor.scroll_offset());
    if let Some(wm) = test.editor.window_manager() {
        if let Some(window) = wm.focused_window() {
            eprintln!("After 'z': Window.scroll_offset = {}", window.scroll_offset());
        }
    }

    test.press('t');
    let first_scroll = test.editor.scroll_offset();
    eprintln!("After 't': Editor.scroll_offset = {}", first_scroll);
    assert_eq!(first_scroll, 24, "First zt should set scroll to 24");

    // Get window scroll offset
    if let Some(wm) = test.editor.window_manager() {
        if let Some(window) = wm.focused_window() {
            eprintln!("After 't': Window.scroll_offset = {}", window.scroll_offset());
            eprintln!("After 't': Window.cursor = {:?}", window.cursor());
        }
    }

    // Second zt - should be no-op (idempotent)
    eprintln!("\n=== SECOND ZT ===");
    test.press('z');
    eprintln!("After 'z': Editor.scroll_offset = {}", test.editor.scroll_offset());
    if let Some(wm) = test.editor.window_manager() {
        if let Some(window) = wm.focused_window() {
            eprintln!("After 'z': Window.scroll_offset = {}", window.scroll_offset());
        }
    }

    test.press('t');
    let second_scroll = test.editor.scroll_offset();
    eprintln!("After 't': Editor.scroll_offset = {}", second_scroll);

    // Get window scroll offset again
    if let Some(wm) = test.editor.window_manager() {
        if let Some(window) = wm.focused_window() {
            eprintln!("After 't': Window.scroll_offset = {}", window.scroll_offset());
            eprintln!("After 't': Window.cursor = {:?}", window.cursor());
        }
    }

    assert_eq!(
        first_scroll, second_scroll,
        "zt should be idempotent - scroll offset should not change on second press"
    );
}

#[test]
fn test_zz_is_idempotent() {
    // Create a buffer with 50 lines
    let content = (1..=50).map(|i| format!("Line {}", i)).collect::<Vec<_>>().join("\n");
    let mut test = EditorTest::new(&content);

    // Initialize window manager with a known viewport size
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);

    // Move to line 25 (middle of buffer)
    test.keys("24j");

    // Verify we're on line 24 (0-indexed)
    assert_eq!(test.editor.buffer().cursor().line(), 24);

    // First zz - should center cursor
    test.keys("zz");
    let first_scroll = test.editor.scroll_offset();
    eprintln!("After first zz: scroll_offset = {}", first_scroll);

    // Second zz - should be no-op (idempotent)
    test.keys("zz");
    let second_scroll = test.editor.scroll_offset();
    eprintln!("After second zz: scroll_offset = {}", second_scroll);

    assert_eq!(
        first_scroll, second_scroll,
        "zz should be idempotent - scroll offset should not change on second press"
    );
}

#[test]
fn test_zb_is_idempotent() {
    // Create a buffer with 50 lines
    let content = (1..=50).map(|i| format!("Line {}", i)).collect::<Vec<_>>().join("\n");
    let mut test = EditorTest::new(&content);

    // Initialize window manager with a known viewport size
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);

    // Move to line 25 (middle of buffer)
    test.keys("24j");

    // Verify we're on line 24 (0-indexed)
    assert_eq!(test.editor.buffer().cursor().line(), 24);

    // First zb - should position cursor at bottom
    test.keys("zb");
    let first_scroll = test.editor.scroll_offset();
    eprintln!("After first zb: scroll_offset = {}", first_scroll);

    // Second zb - should be no-op (idempotent)
    test.keys("zb");
    let second_scroll = test.editor.scroll_offset();
    eprintln!("After second zb: scroll_offset = {}", second_scroll);

    assert_eq!(
        first_scroll, second_scroll,
        "zb should be idempotent - scroll offset should not change on second press"
    );
}
