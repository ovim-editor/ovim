//! Behavioral tests for Ctrl-E / Ctrl-Y (line-at-a-time viewport scrolling).
//!
//! Regression coverage for the bug where `scroll_viewport_down/up` read the
//! focused window's *stale* internal cursor (never synced from the buffer
//! cursor during normal motion), causing the cursor to teleport and scrolloff
//! to undo the scroll.

mod helpers;
#[path = "helpers/viewport_assertions.rs"]
mod viewport_assertions;

use helpers::EditorTest;
use viewport_assertions::ViewportAssertion;

fn long_buffer() -> String {
    (1..=100)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n")
}

fn editor_20x80(scrolloff: usize) -> EditorTest {
    let mut test = EditorTest::new(&long_buffer());
    test.editor.init_window_manager(80, 20);
    test.editor.set_viewport_height(20);
    test.editor.options.scrolloff = scrolloff;
    test
}

#[test]
fn ctrl_e_at_top_scrolls_and_keeps_cursor_at_scrolloff_margin() {
    let mut test = editor_20x80(5);

    // Cursor at line 0, scroll 0. Ctrl-E scrolls the view down one line.
    test.keys("<C-e>");

    let v = ViewportAssertion::new(&test.editor);
    assert_eq!(
        v.scroll_offset(),
        1,
        "view should scroll down exactly one line"
    );
    assert_eq!(
        v.cursor_line(),
        6,
        "cursor should be pushed to the scrolloff margin (offset 1 + scrolloff 5)"
    );
}

#[test]
fn ctrl_e_in_middle_keeps_cursor_on_its_line() {
    let mut test = editor_20x80(5);

    // Move to line 29 (0-indexed); viewport ends up at offset 15, cursor far
    // from both margins.
    test.keys("30G");
    let before = ViewportAssertion::new(&test.editor);
    let before_offset = before.scroll_offset();
    assert_eq!(before.cursor_line(), 29);

    test.keys("<C-e>");

    let after = ViewportAssertion::new(&test.editor);
    assert_eq!(
        after.scroll_offset(),
        before_offset + 1,
        "view scrolls down one line"
    );
    assert_eq!(
        after.cursor_line(),
        29,
        "cursor stays on its line when it has margin to spare"
    );
}

#[test]
fn ctrl_e_with_count_scrolls_multiple_lines() {
    let mut test = editor_20x80(5);

    test.keys("3<C-e>");

    let v = ViewportAssertion::new(&test.editor);
    assert_eq!(v.scroll_offset(), 3, "count prefix scrolls that many lines");
    assert_eq!(v.cursor_line(), 8, "cursor at scrolloff margin (3 + 5)");
}

#[test]
fn ctrl_y_at_bottom_scrolls_up_and_keeps_cursor_at_scrolloff_margin() {
    let mut test = editor_20x80(5);

    // Move cursor to line 20; with scrolloff=5 the view sits at offset 6 and the
    // cursor is exactly on the bottom margin.
    test.keys("21G");
    let before = ViewportAssertion::new(&test.editor);
    assert_eq!(before.cursor_line(), 20);
    let before_offset = before.scroll_offset();

    test.keys("<C-y>");

    let after = ViewportAssertion::new(&test.editor);
    assert_eq!(
        after.scroll_offset(),
        before_offset - 1,
        "view scrolls up one line"
    );
    assert_eq!(
        after.cursor_line(),
        19,
        "cursor follows up to stay at the bottom scrolloff margin"
    );
}

#[test]
fn ctrl_e_does_not_scroll_past_eof() {
    let mut test = editor_20x80(0);

    // Jump near the end and try to keep scrolling down.
    test.keys("G");
    for _ in 0..50 {
        test.keys("<C-e>");
    }

    let v = ViewportAssertion::new(&test.editor);
    // 100 lines, height 20 -> last possible top line is 80.
    assert_eq!(
        v.scroll_offset(),
        80,
        "view cannot scroll past the last full screen"
    );
    assert_eq!(v.cursor_line(), 99, "cursor remains on the last line");
}

#[test]
fn ctrl_e_then_ctrl_y_round_trips() {
    let mut test = editor_20x80(3);

    test.keys("40G");
    let start = ViewportAssertion::new(&test.editor).scroll_offset();

    test.keys("<C-e>");
    test.keys("<C-y>");

    let v = ViewportAssertion::new(&test.editor);
    assert_eq!(
        v.scroll_offset(),
        start,
        "scroll down then up returns to the original offset"
    );
}
