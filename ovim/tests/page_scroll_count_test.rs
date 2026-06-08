//! Tests for `[count]` prefixes on full-page scrolling (Ctrl-F / Ctrl-B).
//!
//! Vim semantics:
//!   [count]CTRL-F   — scroll [count] pages forward (down).
//!   [count]CTRL-B   — scroll [count] pages backward (up).
//!
//! A bare `<C-f>` scrolls one page; `2<C-f>` must scroll exactly two pages,
//! i.e. be equivalent to pressing `<C-f>` twice. Regression coverage for the
//! count being read-then-discarded in the Ctrl-F/Ctrl-B dispatch.

mod helpers;
#[path = "helpers/viewport_assertions.rs"]
mod viewport_assertions;

use helpers::EditorTest;
use viewport_assertions::ViewportAssertion;

fn make_content(n: usize) -> String {
    (1..=n)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n")
}

fn setup(content: &str, viewport_height: usize) -> EditorTest {
    let mut test = EditorTest::new(content);
    test.editor.init_window_manager(80, viewport_height as u16);
    test.editor.set_viewport_height(viewport_height);
    test.editor.options.scrolloff = 0;
    test
}

#[test]
fn count_ctrl_f_scrolls_multiple_pages() {
    let content = make_content(200);

    // Reference: two single page-downs.
    let mut twice = setup(&content, 20);
    twice.keys("<C-f>");
    twice.keys("<C-f>");
    let expected_scroll = ViewportAssertion::new(&twice.editor).scroll_offset();
    let expected_line = ViewportAssertion::new(&twice.editor).cursor_line();

    // 2<C-f> must match two single page-downs.
    let mut counted = setup(&content, 20);
    counted.keys("2<C-f>");
    let vp = ViewportAssertion::new(&counted.editor);

    assert_eq!(
        vp.scroll_offset(),
        expected_scroll,
        "2<C-f> should scroll two pages (same as <C-f><C-f>)"
    );
    assert_eq!(
        vp.cursor_line(),
        expected_line,
        "2<C-f> should move the cursor two pages down"
    );
    assert!(
        expected_scroll > 0,
        "sanity: two page-downs should move the viewport"
    );
}

#[test]
fn count_ctrl_b_scrolls_multiple_pages() {
    let content = make_content(200);

    // Reference: scroll to the bottom, then two single page-ups.
    let mut twice = setup(&content, 20);
    twice.keys("G");
    twice.keys("<C-b>");
    twice.keys("<C-b>");
    let expected_scroll = ViewportAssertion::new(&twice.editor).scroll_offset();
    let expected_line = ViewportAssertion::new(&twice.editor).cursor_line();

    // 2<C-b> must match two single page-ups from the same starting point.
    let mut counted = setup(&content, 20);
    counted.keys("G");
    counted.keys("2<C-b>");
    let vp = ViewportAssertion::new(&counted.editor);

    assert_eq!(
        vp.scroll_offset(),
        expected_scroll,
        "2<C-b> should scroll two pages up (same as <C-b><C-b>)"
    );
    assert_eq!(
        vp.cursor_line(),
        expected_line,
        "2<C-b> should move the cursor two pages up"
    );
}

#[test]
fn bare_ctrl_f_still_scrolls_one_page() {
    // Guard: adding count support must not change the no-count behavior.
    let content = make_content(200);
    let mut test = setup(&content, 20);
    test.keys("<C-f>");
    let vp = ViewportAssertion::new(&test.editor);
    // One page with height 20 advances the top by height-2 = 18 lines.
    assert_eq!(
        vp.scroll_offset(),
        18,
        "bare <C-f> scrolls exactly one page"
    );
}
