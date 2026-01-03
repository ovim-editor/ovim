/// Tests for viewport command persistence issues
///
/// These tests verify that viewport commands (zt, zz, zb) properly maintain
/// scroll position after cursor movements.

mod helpers;

use helpers::{EditorTest, ViewportAssertion};

/// Macro for declarative viewport testing
#[macro_export]
macro_rules! viewport_test {
    (
        name: $name:ident,
        lines: $lines:expr,
        viewport: $viewport:expr,
        $(scrolloff: $scrolloff:expr,)?
        setup: $setup:expr,
        action: $action:expr,
        $(then: $then:expr,)?
        expect: {
            $(cursor_line: $cursor_line:expr,)?
            $(cursor_col: $cursor_col:expr,)?
            $(scroll_offset: $scroll_offset:expr,)?
            $(line_at_top: $line_at_top:expr,)?
            $(line_at_bottom: $line_at_bottom:expr,)?
        }
    ) => {
        #[test]
        fn $name() {
            // Generate buffer content
            let content = (1..=$lines)
                .map(|i| format!("Line {}", i))
                .collect::<Vec<_>>()
                .join("\n");

            let mut test = EditorTest::new(&content);
            test.editor.init_window_manager(80, $viewport);
            test.editor.set_viewport_height($viewport);

            $(
                test.editor.options.scrolloff = $scrolloff;
            )?

            // Setup phase
            if !$setup.is_empty() {
                test.keys($setup);
            }

            // Action phase
            test.keys($action);

            // Optional "then" phase for multi-step tests
            $(
                test.keys($then);
            )?

            // Assertions
            let viewport = ViewportAssertion::new(&test.editor);

            $(
                assert_eq!(
                    viewport.cursor_line(),
                    $cursor_line,
                    "Expected cursor at line {}, got {}. Viewport:\n{}",
                    $cursor_line,
                    viewport.cursor_line(),
                    viewport.debug_display()
                );
            )?

            $(
                assert_eq!(
                    viewport.cursor_col(),
                    $cursor_col,
                    "Expected cursor at col {}, got {}",
                    $cursor_col,
                    viewport.cursor_col()
                );
            )?

            $(
                assert_eq!(
                    viewport.scroll_offset(),
                    $scroll_offset,
                    "Expected scroll_offset {}, got {}. Viewport state:\n{}",
                    $scroll_offset,
                    viewport.scroll_offset(),
                    viewport.debug_display()
                );
            )?

            $(
                assert_eq!(
                    viewport.line_at_viewport_position(0),
                    $line_at_top,
                    "Expected line {} at top, got {}. Viewport:\n{}",
                    $line_at_top,
                    viewport.line_at_viewport_position(0),
                    viewport.debug_display()
                );
            )?

            $(
                let bottom_pos = test.editor.viewport_height().saturating_sub(1);
                assert_eq!(
                    viewport.line_at_viewport_position(bottom_pos),
                    $line_at_bottom,
                    "Expected line {} at bottom, got {}. Viewport:\n{}",
                    $line_at_bottom,
                    viewport.line_at_viewport_position(bottom_pos),
                    viewport.debug_display()
                );
            )?
        }
    };
}

// Problem 1: Viewport position not persisted after cursor movement
viewport_test! {
    name: test_zt_persists_after_j,
    lines: 50,
    viewport: 20,
    setup: "24j",           // Go to line 24 (0-indexed)
    action: "zt",           // Position at top
    then: "j",              // Move down
    expect: {
        cursor_line: 25,
        scroll_offset: 24,  // Should STAY at 24, not recalculate!
        line_at_top: 24,
    }
}

viewport_test! {
    name: test_zz_persists_after_j,
    lines: 50,
    viewport: 20,
    setup: "24j",
    action: "zz",           // Center
    then: "j",
    expect: {
        cursor_line: 25,
        scroll_offset: 14,  // Should STAY at 14 (24 - 10)
    }
}

viewport_test! {
    name: test_zb_persists_after_j,
    lines: 50,
    viewport: 20,
    setup: "24j",
    action: "zb",           // Position at bottom (scroll=5, viewport shows 5-24)
    then: "j",              // Move to line 25 (BELOW viewport)
    expect: {
        cursor_line: 25,
        scroll_offset: 6,   // Scroll adjusts to keep cursor visible (25-19=6)
    }
}

// Problem 2: Clarify zt behavior - it positions the LINE at top, cursor stays on same line
viewport_test! {
    name: test_zt_positions_current_line_at_top,
    lines: 50,
    viewport: 20,
    setup: "24j",
    action: "zt",
    expect: {
        cursor_line: 24,      // Cursor stays on line 24
        cursor_col: 0,
        scroll_offset: 24,    // Scroll adjusts so line 24 is at viewport top
        line_at_top: 24,
    }
}

viewport_test! {
    name: test_zz_centers_current_line,
    lines: 50,
    viewport: 20,
    setup: "24j",
    action: "zz",
    expect: {
        cursor_line: 24,
        scroll_offset: 14,    // 24 - 10 (half of viewport height 20)
    }
}

viewport_test! {
    name: test_zb_positions_current_line_at_bottom,
    lines: 50,
    viewport: 20,
    setup: "24j",
    action: "zb",
    expect: {
        cursor_line: 24,
        scroll_offset: 5,     // 24 - 19 (viewport height - 1)
        line_at_bottom: 24,
    }
}

// Problem 3: zb behavior with small line numbers (not enough lines above)
viewport_test! {
    name: test_zb_near_beginning,
    lines: 50,
    viewport: 20,
    setup: "10j",  // Line 10 - not enough lines above to position at bottom
    action: "zb",
    expect: {
        cursor_line: 10,
        scroll_offset: 0,     // Can't scroll negative, saturates to 0
        line_at_top: 0,       // Line 0 is at top
    }
}

viewport_test! {
    name: test_zb_exactly_viewport_height_minus_one,
    lines: 50,
    viewport: 20,
    setup: "19j",  // Line 19 = viewport_height - 1
    action: "zb",
    expect: {
        cursor_line: 19,
        scroll_offset: 0,     // 19 - 19 = 0
        line_at_top: 0,
        line_at_bottom: 19,
    }
}

// Multi-step persistence tests
viewport_test! {
    name: test_zt_persists_through_multiple_movements,
    lines: 50,
    viewport: 20,
    setup: "24j",
    action: "zt",
    then: "jjj",  // Move down 3 lines
    expect: {
        cursor_line: 27,
        scroll_offset: 24,  // Should still be 24!
    }
}

viewport_test! {
    name: test_zt_then_k_movement,
    lines: 50,
    viewport: 20,
    setup: "24j",
    action: "zt",           // Position at top (scroll=24, viewport shows 24-43)
    then: "k",              // Move to line 23 (ABOVE viewport)
    expect: {
        cursor_line: 23,
        scroll_offset: 23,  // Scroll adjusts to keep cursor visible
    }
}

// Edge case: cursor moves beyond viewport
viewport_test! {
    name: test_zt_then_move_beyond_viewport,
    lines: 50,
    viewport: 20,
    setup: "24j",
    action: "zt",
    then: "25j",  // Move way down, beyond current viewport
    expect: {
        cursor_line: 49,
        // Scroll should now adjust to keep cursor visible
        // This is expected behavior - viewport commands don't "lock" scroll forever
        scroll_offset: 30,  // 49 - 19
    }
}
