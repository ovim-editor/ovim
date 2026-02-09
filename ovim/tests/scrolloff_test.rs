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
fn test_viewport_command_scrolloff_reengages_on_movement() {
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

    // zt explicitly positions the cursor line at the top of the viewport (ignores scrolloff)
    // scroll_offset = cursor_line = 24
    assert_eq!(
        viewport.scroll_offset(),
        24,
        "zt should position at scroll_offset=24. Got {}",
        viewport.scroll_offset()
    );

    // Now move down one line — scrolloff re-engages (matches Vim behavior).
    // Cursor at line 25, scrolloff=5, so scroll adjusts to keep 5 lines above cursor.
    test.keys("j");

    let viewport = ViewportAssertion::new(&test.editor);
    assert_eq!(viewport.cursor_line(), 25);

    // scrolloff=5: cursor needs 5 lines above it → scroll_offset = 25 - 5 = 20
    assert_eq!(
        viewport.scroll_offset(),
        20,
        "After zt+j, scrolloff re-engages. Got {}",
        viewport.scroll_offset()
    );
}

// === Regression tests for Bug 1: w at EOF viewport issue ===

#[test]
fn test_bug1_w_at_eof_short_file() {
    // Bug: When file is shorter than scrolloff, pressing 'w' at EOF
    // caused viewport to jump to top instead of staying at bottom
    let content = "line1 word1 word2\nline2 word3 word4\nline3 word5 last";

    let mut test = EditorTest::new(content);
    test.editor.init_window_manager(80, 24);
    test.editor.set_viewport_height(24);
    test.editor.options.scrolloff = 10; // scrolloff larger than file!

    // Move to last word
    test.keys("G$");
    let viewport = ViewportAssertion::new(&test.editor);
    let line_before_w = viewport.cursor_line();

    // Press 'w' at end of file - cursor may move to empty final line
    test.keys("w");

    let viewport = ViewportAssertion::new(&test.editor);
    let line_after_w = viewport.cursor_line();

    // Cursor may advance one line if there's a trailing newline, but should stay near EOF
    assert!(
        line_after_w >= line_before_w && line_after_w <= line_before_w + 1,
        "Cursor should stay near EOF after w. Was {}, now {}",
        line_before_w,
        line_after_w
    );
    // CRITICAL: scroll_offset should be 0 (at top) not wrap-around
    assert_eq!(
        viewport.scroll_offset(),
        0,
        "Bug 1 fix: scroll_offset should be 0 for short file, not jump to top. Got {}",
        viewport.scroll_offset()
    );
}

#[test]
fn test_bug1_scrolloff_larger_than_viewport() {
    // Edge case: scrolloff larger than half viewport
    let content = "line1\nline2\nline3\nline4\nline5";

    let mut test = EditorTest::new(content);
    test.editor.init_window_manager(80, 8); // Small viewport
    test.editor.set_viewport_height(8);
    test.editor.options.scrolloff = 10; // scrolloff > viewport/2

    // Move to last line
    test.keys("G");
    let viewport = ViewportAssertion::new(&test.editor);

    // Should center cursor when scrolloff > viewport/2
    assert_eq!(
        viewport.cursor_line(),
        4,
        "Should be on last line (index 4)"
    );
    // scroll_offset should center the file, not be negative
    assert!(
        viewport.scroll_offset() == 0,
        "Bug 1 fix: When scrolloff > viewport, should center at 0 for short files. Got {}",
        viewport.scroll_offset()
    );
}

#[test]
fn test_bug1_e_motion_at_eof() {
    // Adjacent bug check: does 'e' have same issue?
    let content = "line1 word1\nline2 last";

    let mut test = EditorTest::new(content);
    test.editor.init_window_manager(80, 24);
    test.editor.set_viewport_height(24);
    test.editor.options.scrolloff = 10;

    // Move to last word, press 'e'
    test.keys("G$e");

    let viewport = ViewportAssertion::new(&test.editor);
    let final_line = viewport.cursor_line();

    // Cursor should be near the last line (exact position depends on trailing newline)
    assert!(
        final_line >= test.editor.buffer().line_count().saturating_sub(2),
        "Should be near last line, got {}",
        final_line
    );

    // CRITICAL: Bug 1 fix - scroll_offset should NOT wrap to large value
    assert_eq!(
        viewport.scroll_offset(),
        0,
        "'e' motion should also handle short files correctly. Got {}",
        viewport.scroll_offset()
    );
}
