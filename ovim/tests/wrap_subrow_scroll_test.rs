//! Regression tests for the two scroll bugs that needed a *visual-row
//! sub-offset* in the viewport — the ability to start rendering partway into a
//! wrapped logical line. The viewport top is now `(scroll_offset, scroll_subrow)`
//! (see `Window::scroll_subrow`), so:
//!
//!   #4  The tail of a wrapped sole/final logical line taller than the viewport
//!       is reachable — scrolling begins partway into the line so the cursor at
//!       its end stays on screen.
//!   #5  Ctrl-E / Ctrl-Y scroll exactly one *visual* (wrapped) row, not a whole
//!       logical line.

mod helpers;
#[path = "helpers/viewport_assertions.rs"]
mod viewport_assertions;

use helpers::EditorTest;
use viewport_assertions::ViewportAssertion;

const WIDTH: usize = 10;

fn wrapped_editor(content: &str, height: usize) -> EditorTest {
    let mut test = EditorTest::new(content);
    test.editor.init_window_manager(WIDTH as u16, height as u16);
    test.editor.set_viewport_height(height);
    test.editor.options.wrap = true;
    test.editor.options.scrolloff = 0;
    test.editor.ensure_wrap_map(WIDTH);
    test
}

// =====================================================================
// #4 — tail of a wrapped line must be reachable (cursor stays visible)
// =====================================================================

#[test]
fn cursor_visible_at_end_of_sole_wrapped_line() {
    // One 50-char logical line wraps into 5 visual rows at width 10.
    // Viewport is only 3 rows tall, so the end of the line (visual row 4)
    // can only be shown by scrolling *into* the wrapped line.
    let content = "0123456789".repeat(5); // 50 chars, no trailing newline
    let mut test = wrapped_editor(&content, 3);

    test.keys("$"); // move to the last column

    let vp = ViewportAssertion::new(&test.editor);
    assert!(
        vp.cursor_is_visible(),
        "cursor at the end of a wrapped sole line must be on screen \
         (visual row {} of viewport height {})",
        vp.cursor_visual_row_from_top(),
        vp.viewport_height(),
    );
}

#[test]
fn cursor_visible_at_end_of_final_wrapped_line() {
    // Two short lines, then a 50-char final line (5 visual rows) at width 10.
    // The final line is taller than the 3-row viewport.
    let content = format!("a\nb\n{}", "0123456789".repeat(5));
    let mut test = wrapped_editor(&content, 3);

    test.keys("G$"); // last line, last column

    let vp = ViewportAssertion::new(&test.editor);
    assert!(
        vp.cursor_is_visible(),
        "cursor at the end of a wrapped final line must be on screen \
         (visual row {} of viewport height {})",
        vp.cursor_visual_row_from_top(),
        vp.viewport_height(),
    );
}

// =====================================================================
// #5 — Ctrl-E / Ctrl-Y scroll one *visual* row, not one logical line
// =====================================================================

#[test]
fn ctrl_e_scrolls_one_visual_row() {
    // Line 0 is 30 chars → 3 visual rows at width 10, followed by short lines.
    let content = format!("{}\n{}", "0123456789".repeat(3), "x\n".repeat(10));
    let mut test = wrapped_editor(&content, 6);

    let before = ViewportAssertion::new(&test.editor).viewport_top_visual_row();
    test.keys("<C-e>");
    let after = ViewportAssertion::new(&test.editor).viewport_top_visual_row();

    assert_eq!(
        after - before,
        1,
        "Ctrl-E scrolls the view down exactly one visual row (line 0 wraps, so \
         the second wrapped segment should become the top row)"
    );
}

#[test]
fn ctrl_y_scrolls_one_visual_row() {
    // Short lines, then a 30-char wrapped line (3 rows) at logical index 5,
    // then more short lines. Start with the viewport top just *below* the
    // wrapped line, so scrolling up one visual row must enter that line's
    // last wrapped segment.
    let content = format!(
        "{}{}\n{}",
        "x\n".repeat(5),        // lines 0..4 (short) + line 5 starts here
        "0123456789".repeat(3), // line 5: 30 chars → 3 visual rows
        "y\n".repeat(10)        // lines 6..
    );
    let mut test = wrapped_editor(&content, 6);

    // Park the viewport top at logical line 6 (just past the wrapped line).
    if let Some(wm) = test.editor.window_manager_mut() {
        if let Some(window) = wm.focused_window_mut() {
            window.set_scroll_offset(6);
        }
    }
    // Cursor comfortably inside the viewport so update_scroll_offset won't fight.
    test.set_cursor(8, 0);

    let before = ViewportAssertion::new(&test.editor).viewport_top_visual_row();
    test.keys("<C-y>");
    let after = ViewportAssertion::new(&test.editor).viewport_top_visual_row();

    assert_eq!(
        before - after,
        1,
        "Ctrl-Y scrolls the view up exactly one visual row (into the wrapped \
         line's last segment, not over the whole logical line)"
    );
}
