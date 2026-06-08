//! Wrap-aware viewport commands: `zz` (center) and `zb` (bottom) must position
//! the cursor by *visual* (wrapped) rows, not logical lines.
//!
//! When logical lines above the cursor wrap into multiple visual rows, logical
//! centering pushes the cursor far below the true center — often clean off the
//! bottom of the viewport. These tests pin the visual-row behavior.

mod helpers;
#[path = "helpers/viewport_assertions.rs"]
mod viewport_assertions;

use helpers::EditorTest;
use viewport_assertions::ViewportAssertion;

const WIDTH: usize = 10;
const HEIGHT: usize = 8;

/// 20 lines, each 15 chars → wraps into exactly 2 visual rows at width 10.
fn wrapping_content() -> String {
    (0..20)
        .map(|_| "abcdefghijklmno".to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn setup_wrapped() -> EditorTest {
    let mut test = EditorTest::new(&wrapping_content());
    test.editor.init_window_manager(WIDTH as u16, HEIGHT as u16);
    test.editor.set_viewport_height(HEIGHT);
    test.editor.options.wrap = true;
    test.editor.options.scrolloff = 0;
    test.editor.ensure_wrap_map(WIDTH);
    test
}

#[test]
fn zz_centers_cursor_by_visual_rows() {
    let mut test = setup_wrapped();
    test.set_cursor(10, 0);
    test.keys("zz");

    let vp = ViewportAssertion::new(&test.editor);
    assert!(
        vp.cursor_is_visible(),
        "zz must keep the cursor on screen (visual row {} of {})",
        vp.cursor_visual_row_from_top(),
        HEIGHT
    );
    assert_eq!(
        vp.cursor_visual_row_from_top(),
        HEIGHT / 2,
        "zz centers the cursor in *visual* rows when lines above wrap"
    );
}

#[test]
fn zb_puts_cursor_near_bottom_by_visual_rows() {
    let mut test = setup_wrapped();
    test.set_cursor(10, 0);
    test.keys("zb");

    let vp = ViewportAssertion::new(&test.editor);
    assert!(
        vp.cursor_is_visible(),
        "zb must keep the cursor on screen (visual row {} of {})",
        vp.cursor_visual_row_from_top(),
        HEIGHT
    );
    assert!(
        vp.cursor_visual_row_from_top() >= HEIGHT / 2,
        "zb places the cursor in the bottom half of the viewport, got visual row {}",
        vp.cursor_visual_row_from_top()
    );
}

#[test]
fn zt_puts_cursor_at_top() {
    // zt is already correct for the common case (scroll to the cursor's line),
    // but guard it under wrap so a regression in the shared path is caught.
    let mut test = setup_wrapped();
    test.set_cursor(10, 0);
    test.keys("zt");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(
        vp.cursor_visual_row_from_top(),
        0,
        "zt puts the cursor's line at the top of the viewport"
    );
}
