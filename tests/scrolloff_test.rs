/// Tests for scrolloff behavior
mod helpers;

use helpers::{EditorTest, ViewportAssertion};

#[test]
fn test_scrolloff_maintains_margin_when_scrolling() {
    let content = (1..=50)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);
    test.editor.options.scrolloff = 5;

    // Start at line 0, scroll_offset=0, viewport shows lines 0-19
    // Move down to line 16 (within scrolloff margin from bottom)
    test.keys("16j");

    let viewport = ViewportAssertion::new(&test.editor);
    assert_eq!(viewport.cursor_line(), 16);

    // Cursor at line 16 is at viewport position 16 (0-indexed)
    // Viewport bottom is at position 19 (20 lines, positions 0-19)
    // Distance from bottom: 19 - 16 = 3 lines
    // This is LESS than scrolloff=5, so viewport should scroll down
    // New scroll_offset should position cursor at 5 lines from bottom
    // scroll_offset = 16 - (20 - 5 - 1) = 16 - 14 = 2
    assert_eq!(
        viewport.scroll_offset(),
        2,
        "With scrolloff=5, cursor at line 16 should trigger scroll to offset=2. Got {}",
        viewport.scroll_offset()
    );
}

#[test]
fn test_scrolloff_zero_allows_cursor_at_top() {
    let content = (1..=50)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);
    test.editor.options.scrolloff = 0;

    // Move to line 10
    test.keys("10j");

    let viewport = ViewportAssertion::new(&test.editor);

    // Cursor should be on line 10
    assert_eq!(viewport.cursor_line(), 10);

    // With scrolloff=0, scroll_offset stays at 0 (cursor can be at top)
    assert_eq!(
        viewport.scroll_offset(),
        0,
        "With scrolloff=0, scroll_offset should be 0. Got {}",
        viewport.scroll_offset()
    );
}

#[test]
fn test_viewport_command_ignores_scrolloff() {
    let content = (1..=50)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);
    test.editor.options.scrolloff = 5;

    // Move to line 24, then use zt to position at top
    test.keys("24j");
    test.keys("zt");

    let viewport = ViewportAssertion::new(&test.editor);

    // Cursor should be on line 24
    assert_eq!(viewport.cursor_line(), 24);

    // zt should position cursor line at EXACT top (scroll_offset=24)
    // even with scrolloff=5
    assert_eq!(
        viewport.scroll_offset(),
        24,
        "zt should position at exact top (scroll_offset=24) regardless of scrolloff. Got {}",
        viewport.scroll_offset()
    );

    // Now move down one line
    test.keys("j");

    let viewport = ViewportAssertion::new(&test.editor);
    assert_eq!(viewport.cursor_line(), 25);

    // Scroll should stay at 24 (viewport command persistence)
    // not jump due to scrolloff
    assert_eq!(
        viewport.scroll_offset(),
        24,
        "After zt, scroll_offset should persist at 24 when moving down. Got {}",
        viewport.scroll_offset()
    );
}
