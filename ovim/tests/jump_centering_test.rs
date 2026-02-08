/// Tests for cursor centering after jumps (LSP, marks, jump list)
///
/// These tests verify that jumps (as opposed to incremental movements)
/// center the cursor in the viewport, matching Vim/Neovim behavior.
mod helpers;

use helpers::{EditorTest, ViewportAssertion};

#[test]
fn test_mark_jump_centers_cursor() {
    let content = (1..=50)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);
    test.editor.options.scrolloff = 0; // Disable scrolloff for precise centering

    // Start at line 1, set mark 'a'
    test.keys("ma");

    // Jump to line 40
    test.keys("40G");
    assert_eq!(test.editor.buffer().cursor().line(), 39); // 0-indexed

    // Jump back to mark 'a' (line 1)
    test.keys("`a");

    let viewport = ViewportAssertion::new(&test.editor);

    // Cursor should be on line 1
    assert_eq!(viewport.cursor_line(), 0);

    // Line 1 should be centered in viewport (20 lines tall, so position 10)
    // scroll_offset = cursor_line - (viewport_height / 2) = 0 - 10 = 0 (saturates to 0)
    // Since we're at the beginning of the file, we can't actually center
    assert_eq!(
        viewport.scroll_offset(),
        0,
        "Jump to line 1: can't center (at top of file), scroll_offset should be 0. Got {}",
        viewport.scroll_offset()
    );
}

#[test]
fn test_mark_jump_centers_cursor_middle_of_file() {
    let content = (1..=50)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);
    test.editor.options.scrolloff = 0;

    // Start at line 1, move to line 25, set mark 'a'
    test.keys("25G");
    test.keys("ma");

    // Move to line 1
    test.keys("gg");

    // Jump back to mark 'a' (line 25)
    test.keys("`a");

    let viewport = ViewportAssertion::new(&test.editor);

    // Cursor should be on line 25 (0-indexed: 24)
    assert_eq!(viewport.cursor_line(), 24);

    // Line 25 should be centered in viewport
    // scroll_offset = cursor_line - (viewport_height / 2) = 24 - 10 = 14
    // This shows lines 14-33, with cursor at line 24 (position 10 in viewport)
    assert_eq!(
        viewport.scroll_offset(),
        14,
        "Jump to line 25 should center cursor (scroll_offset=14). Got {}",
        viewport.scroll_offset()
    );
}

#[test]
fn test_jump_list_back_centers_cursor() {
    let content = (1..=50)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);
    test.editor.options.scrolloff = 0;

    // Add jump: move to line 30 (creates jump entry)
    test.keys("30G");

    // Move to line 1 (creates another jump entry)
    test.keys("gg");

    // Jump back with Ctrl-O (should go to line 30 and center)
    test.press_with(ovim_core::KeyCode::Char('o'), ovim_core::Modifiers::CONTROL);

    let viewport = ViewportAssertion::new(&test.editor);

    // Cursor should be on line 30 (0-indexed: 29)
    assert_eq!(viewport.cursor_line(), 29);

    // Line 30 should be centered
    // scroll_offset = 29 - 10 = 19
    assert_eq!(
        viewport.scroll_offset(),
        19,
        "Ctrl-O jump to line 30 should center cursor (scroll_offset=19). Got {}",
        viewport.scroll_offset()
    );
}

#[test]
fn test_jump_list_forward_centers_cursor() {
    let content = (1..=50)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);
    test.editor.options.scrolloff = 0;

    // Create jump entries
    test.keys("30G"); // Jump to line 30
    test.keys("gg"); // Jump to line 1

    // Jump back
    test.press_with(ovim_core::KeyCode::Char('o'), ovim_core::Modifiers::CONTROL);

    // Now jump forward with Tab (should go to line 1 and center)
    test.keys("<Tab>");

    let viewport = ViewportAssertion::new(&test.editor);

    // Cursor should be on line 1 (0-indexed: 0)
    assert_eq!(viewport.cursor_line(), 0);

    // Can't center line 1 (at top of file)
    assert_eq!(
        viewport.scroll_offset(),
        0,
        "Tab jump to line 1 should try to center but saturate to 0. Got {}",
        viewport.scroll_offset()
    );
}

#[test]
fn test_jump_near_end_of_file() {
    let content = (1..=50)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);
    test.editor.options.scrolloff = 0;

    // Start at line 1, set mark 'a' at line 48
    test.keys("48G");
    test.keys("ma");
    test.keys("gg");

    // Jump to mark (line 48)
    test.keys("`a");

    let viewport = ViewportAssertion::new(&test.editor);

    // Cursor should be on line 48 (0-indexed: 47)
    assert_eq!(viewport.cursor_line(), 47);

    // Centering: scroll_offset = 47 - 10 = 37
    // This will show lines 37-49 (13 lines visible, 7 blank lines at bottom)
    // Note: We don't clamp to avoid blank lines - centering is more important
    assert_eq!(
        viewport.scroll_offset(),
        37,
        "Jump to line 48 near EOF: centers cursor (scroll_offset=37). Got {}",
        viewport.scroll_offset()
    );
}

#[test]
fn test_jump_with_scrolloff_respects_scrolloff() {
    let content = (1..=50)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);
    test.editor.options.scrolloff = 5;

    // Set mark at line 25
    test.keys("25G");
    test.keys("ma");
    test.keys("gg");

    // Jump to mark (line 25)
    test.keys("`a");

    let viewport = ViewportAssertion::new(&test.editor);

    // Cursor should be on line 25 (0-indexed: 24)
    assert_eq!(viewport.cursor_line(), 24);

    // Centering: scroll_offset = 24 - 10 = 14
    // Even with scrolloff=5, jumps should center (not apply scrolloff positioning)
    assert_eq!(
        viewport.scroll_offset(),
        14,
        "Jump should center cursor even with scrolloff=5. Got {}",
        viewport.scroll_offset()
    );
}

#[test]
fn test_apostrophe_mark_jump_centers_cursor() {
    let content = (1..=50)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);
    test.editor.options.scrolloff = 0;

    // Set mark at line 30
    test.keys("30G");
    test.keys("ma");
    test.keys("gg");

    // Jump with apostrophe (goes to first non-blank on mark line)
    test.keys("'a");

    let viewport = ViewportAssertion::new(&test.editor);

    // Cursor should be on line 30 (0-indexed: 29)
    assert_eq!(viewport.cursor_line(), 29);

    // Should be centered: scroll_offset = 29 - 10 = 19
    assert_eq!(
        viewport.scroll_offset(),
        19,
        "Apostrophe jump should center cursor. Got {}",
        viewport.scroll_offset()
    );
}

#[test]
fn test_multiple_jumps_each_center() {
    let content = (1..=50)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);
    test.editor.options.scrolloff = 0;

    // Set marks at different locations
    test.keys("15G");
    test.keys("ma");
    test.keys("35G");
    test.keys("mb");
    test.keys("gg");

    // Jump to mark 'a' (line 15)
    test.keys("`a");
    let viewport = ViewportAssertion::new(&test.editor);
    assert_eq!(viewport.cursor_line(), 14);
    assert_eq!(
        viewport.scroll_offset(),
        4, // 14 - 10 = 4
        "First jump should center at scroll_offset=4. Got {}",
        viewport.scroll_offset()
    );

    // Jump to mark 'b' (line 35)
    test.keys("`b");
    let viewport = ViewportAssertion::new(&test.editor);
    assert_eq!(viewport.cursor_line(), 34);
    assert_eq!(
        viewport.scroll_offset(),
        24, // 34 - 10 = 24
        "Second jump should center at scroll_offset=24. Got {}",
        viewport.scroll_offset()
    );

    // Jump back to mark 'a' (line 15)
    test.keys("`a");
    let viewport = ViewportAssertion::new(&test.editor);
    assert_eq!(viewport.cursor_line(), 14);
    assert_eq!(
        viewport.scroll_offset(),
        4,
        "Third jump should center at scroll_offset=4. Got {}",
        viewport.scroll_offset()
    );
}

#[test]
fn test_incremental_movement_does_not_center() {
    let content = (1..=50)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);
    test.editor.options.scrolloff = 0;

    // Start at line 1, move down incrementally
    test.keys("j"); // Line 2

    let viewport = ViewportAssertion::new(&test.editor);
    assert_eq!(viewport.cursor_line(), 1);

    // Should NOT center - scroll_offset stays at 0
    assert_eq!(
        viewport.scroll_offset(),
        0,
        "Incremental movement should NOT center. Got {}",
        viewport.scroll_offset()
    );

    // Continue moving down
    test.keys("10j"); // Line 12

    let viewport = ViewportAssertion::new(&test.editor);
    assert_eq!(viewport.cursor_line(), 11);

    // Still should NOT center - scroll_offset stays at 0
    // (cursor is still within viewport 0-19)
    assert_eq!(
        viewport.scroll_offset(),
        0,
        "Incremental movement should NOT center. Got {}",
        viewport.scroll_offset()
    );
}
