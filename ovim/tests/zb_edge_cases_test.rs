/// Tests for zb edge cases and off-by-one bugs
///
/// These tests reproduce the reported issue: "When I `j` to the bottom of the
/// buffer and type `zb` it 'scrolls up' by one"
mod helpers;
#[path = "helpers/viewport_assertions.rs"]
mod viewport_assertions;

use helpers::EditorTest;
use viewport_assertions::ViewportAssertion;

#[test]
fn test_zb_at_end_of_file() {
    // 50 lines (0-49), viewport 20 lines
    // Go to line 49 (last line), press zb
    // Expected: line 49 should be at bottom of viewport (lines 30-49 visible)
    let content = (1..=50)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);

    // Move to last line (49)
    test.keys("G");

    let before = ViewportAssertion::new(&test.editor);
    println!("BEFORE zb:");
    println!("{}", before.debug_display());

    // Position at bottom
    test.keys("zb");

    let after = ViewportAssertion::new(&test.editor);
    println!("\nAFTER zb:");
    println!("{}", after.debug_display());

    // Line 49 should be at bottom of viewport
    // Viewport height 20, so lines 30-49 should be visible
    // scroll_offset = 49 - 19 = 30
    assert_eq!(after.cursor_line(), 49, "Cursor should be on line 49");
    assert_eq!(
        after.scroll_offset(),
        30,
        "Scroll offset should be 30 (49 - 19)"
    );

    let bottom_pos = test.editor.viewport_height().saturating_sub(1);
    assert_eq!(
        after.line_at_viewport_position(bottom_pos),
        49,
        "Line 49 should be at bottom of viewport"
    );
}

#[test]
fn test_zb_then_j_at_end() {
    // This reproduces the reported bug:
    // "When I `j` to the bottom of the buffer and type `zb` it 'scrolls up' by one"
    let content = (1..=50)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);

    // Move to line 48 (display line 48)
    // Since vim uses 1-based display, 48G should go to the 48th line (0-indexed = 47)
    test.keys("48G");

    let cursor_after_48g = test.editor.buffer().cursor();
    println!("Buffer line count: {}", test.editor.buffer().line_count());
    println!(
        "After 48G: cursor at 0-indexed line {}",
        cursor_after_48g.line()
    );
    println!(
        "Line content: {:?}",
        test.editor.buffer().line(cursor_after_48g.line())
    );
    println!("At line {}, before zb:", cursor_after_48g.line());
    println!("{}", ViewportAssertion::new(&test.editor).debug_display());

    // Position at bottom
    test.keys("zb");

    println!("\nAfter zb:");
    let viewport_after_zb = ViewportAssertion::new(&test.editor);
    println!("{}", viewport_after_zb.debug_display());

    // Check window scroll vs editor scroll
    if let Some(wm) = test.editor.window_manager() {
        if let Some(window) = wm.focused_window() {
            println!("Window scroll_offset: {}", window.scroll_offset());
            println!("Window cursor: {:?}", window.cursor());
        }
    }
    println!(
        "Editor scroll_offset (fallback): {}",
        test.editor.scroll_offset()
    );

    // 48G moves to 0-indexed line 47 (display line 48)
    // zb should position line 47 at bottom of viewport
    // scroll_offset = 47 - 19 = 28
    assert_eq!(
        viewport_after_zb.cursor_line(),
        47,
        "Cursor should be at line 47 after 48G"
    );
    assert_eq!(
        viewport_after_zb.scroll_offset(),
        28,
        "Scroll should be 47 - 19 = 28"
    );

    // Now move down to line 48
    test.keys("j");

    println!("\nAfter j:");
    println!("{}", ViewportAssertion::new(&test.editor).debug_display());

    let viewport = ViewportAssertion::new(&test.editor);

    // After j, cursor moves to line 48.
    // Scrolloff (default 10, clamped to 9 for viewport=20) re-engages:
    // ideal offset = 48 + 9 + 1 - 20 = 38, but max_scroll = 50 - 20 = 30, so clamped to 30.
    assert_eq!(viewport.cursor_line(), 48);
    assert_eq!(
        viewport.scroll_offset(),
        30,
        "Should scroll with scrolloff re-engaged"
    );
}

#[test]
fn test_zb_when_line_in_middle_of_viewport() {
    // Test: cursor at line 30, but line 30 is in middle of viewport
    // zb should move it to bottom
    let content = (1..=100)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);

    // Jump to line 30 (0-indexed = 29), starting from scroll=0
    test.keys("30G");

    let before = ViewportAssertion::new(&test.editor);
    println!("BEFORE zb (at line 29 via 30G):");
    println!("{}", before.debug_display());
    if before.scroll_offset() <= 29 {
        println!(
            "Line 29 visible position in viewport: {}",
            29 - before.scroll_offset()
        );
    }

    // Press zb - should position line 29 at bottom
    test.keys("zb");

    let after = ViewportAssertion::new(&test.editor);
    println!("\nAFTER zb:");
    println!("{}", after.debug_display());
    println!(
        "Line 29 visible position in viewport: {}",
        29 - after.scroll_offset()
    );

    // 30G goes to 0-indexed line 29
    // zb should position line 29 at bottom of viewport (position 19 in viewport)
    // scroll_offset = 29 - 19 = 10
    assert_eq!(after.cursor_line(), 29, "Cursor should be at line 29");
    assert_eq!(
        after.scroll_offset(),
        10,
        "Scroll should be 10 to position line 29 at bottom"
    );

    // Verify line 29 is at position 19 (bottom) in viewport
    assert_eq!(
        29 - after.scroll_offset(),
        19,
        "Line 29 should be at viewport position 19 (bottom)"
    );
}

#[test]
fn test_zb_user_bug_report_reproduction() {
    // User report: "When I `j` to the bottom of the buffer and type `zb` it 'scrolls up' by one"
    //
    // Hypothesis: After j-ing to bottom, cursor is at line 49, and scroll is at 30 (correct).
    // Then zb should keep it there (idempotent). But maybe it's changing to 29?
    let content = (1..=50)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);

    // Use j to move to bottom (simulating user workflow)
    // Start at line 0, go to line 49
    for _ in 0..49 {
        test.keys("j");
    }

    let scroll_before = {
        let v = ViewportAssertion::new(&test.editor);
        println!("BEFORE zb (at last line 49, after j-ing to bottom):");
        println!("{}", v.debug_display());
        println!(
            "Visible lines: {}-{}",
            v.scroll_offset(),
            v.scroll_offset() + 19
        );
        v.scroll_offset()
    };

    // Press zb - should position line 49 at bottom
    test.keys("zb");

    let scroll_after = {
        let v = ViewportAssertion::new(&test.editor);
        println!("\nAFTER zb:");
        println!("{}", v.debug_display());
        println!(
            "Visible lines: {}-{}",
            v.scroll_offset(),
            v.scroll_offset() + 19
        );
        v.scroll_offset()
    };

    // Check if scroll changed (user said "scrolls up by one")
    if scroll_before != scroll_after {
        println!(
            "\nBUG REPRODUCED! Scroll changed from {} to {}",
            scroll_before, scroll_after
        );
        println!(
            "This is 'scrolling up by {}' (showing earlier lines)",
            scroll_before - scroll_after
        );
    }

    let final_viewport = ViewportAssertion::new(&test.editor);
    assert_eq!(
        final_viewport.cursor_line(),
        49,
        "Cursor should still be on line 49"
    );

    // Expected: line 49 at bottom = scroll 30 (shows lines 30-49)
    // Should be idempotent!
    assert_eq!(
        scroll_after, scroll_before,
        "zb should be idempotent when line is already at bottom"
    );
    assert_eq!(
        scroll_after, 30,
        "Scroll should be 30 to position line 49 at bottom"
    );
}

#[test]
fn test_zb_calculation_edge_case() {
    // Test with a file exactly 20 lines (same as viewport)
    let content = (1..=20)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);

    // Move to last line (19)
    test.keys("G");

    println!("File has 20 lines, viewport is 20 lines");
    println!("At line 19, before zb:");
    println!("{}", ViewportAssertion::new(&test.editor).debug_display());

    test.keys("zb");

    let viewport = ViewportAssertion::new(&test.editor);
    println!("\nAfter zb:");
    println!("{}", viewport.debug_display());

    // Line 19 at bottom: scroll = 19 - 19 = 0
    // Viewport shows lines 0-19 (all lines)
    assert_eq!(viewport.cursor_line(), 19);
    assert_eq!(viewport.scroll_offset(), 0);
}

#[test]
fn test_zb_with_file_smaller_than_viewport() {
    // File with 10 lines, viewport 20 lines
    let content = (1..=10)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);

    // Move to last line (9)
    test.keys("G");

    println!("File has 10 lines, viewport is 20 lines");
    println!("At line 9, before zb:");
    println!("{}", ViewportAssertion::new(&test.editor).debug_display());

    test.keys("zb");

    let viewport = ViewportAssertion::new(&test.editor);
    println!("\nAfter zb:");
    println!("{}", viewport.debug_display());

    // Line 9 at bottom: scroll = 9 - 19 = saturates to 0
    // Can't scroll negative, so viewport shows lines 0-9 (all lines)
    assert_eq!(viewport.cursor_line(), 9);
    assert_eq!(
        viewport.scroll_offset(),
        0,
        "Can't scroll negative for small files"
    );
}

#[test]
fn test_g_command_line_numbering() {
    let content = (1..=50)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);

    println!("Buffer has {} lines", test.editor.buffer().line_count());

    // Move to line 48 (display line 48, 0-indexed = 47)
    test.keys("48G");

    let cursor = test.editor.buffer().cursor();
    println!("After 48G: cursor at line {} (0-indexed)", cursor.line());
    println!(
        "Buffer line content: {:?}",
        test.editor.buffer().line(cursor.line())
    );

    // Move to last line with G
    test.keys("G");

    let cursor = test.editor.buffer().cursor();
    println!("After G: cursor at line {} (0-indexed)", cursor.line());
    println!(
        "Buffer line content: {:?}",
        test.editor.buffer().line(cursor.line())
    );
    println!("Expected: line 49 (0-indexed)");

    assert_eq!(
        cursor.line(),
        49,
        "G should move to last line (49 in 0-indexed)"
    );
}
