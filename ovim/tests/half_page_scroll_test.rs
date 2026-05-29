//! Tests for the half-page scroll amount (Ctrl-D / Ctrl-U).
//!
//! Regression coverage for half_page_scroll() using the whole buffer-area
//! height instead of the focused window's height, which made Ctrl-D/U scroll
//! half of the *entire editor* in a split rather than half of the focused pane.

mod helpers;

use helpers::EditorTest;

fn long_buffer() -> String {
    (1..=100)
        .map(|i| format!("L{}", i))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn half_page_is_half_of_single_window_height() {
    let mut test = EditorTest::new(&long_buffer());
    test.editor.init_window_manager(80, 24);
    test.editor.set_viewport_height(24);

    assert_eq!(test.editor.half_page_scroll(), 12);
}

#[test]
fn half_page_uses_focused_window_height_in_split() {
    let mut test = EditorTest::new(&long_buffer());
    test.editor.init_window_manager(80, 24);
    test.editor.set_viewport_height(24);

    test.editor.split_window_horizontal();
    if let Some(wm) = test.editor.window_manager_mut() {
        wm.update_dimensions(80, 24);
    }

    // The full buffer area is still 24 rows, but the focused pane is only 12.
    // Half-page must follow the pane, not the whole editor.
    let focused_height = test
        .editor
        .window_manager()
        .unwrap()
        .focused_window()
        .unwrap()
        .height() as usize;
    assert_eq!(
        test.editor.half_page_scroll(),
        focused_height / 2,
        "half-page scroll should track the focused pane's height"
    );
    assert!(
        test.editor.half_page_scroll() < 12,
        "split pane half-page must be smaller than the full-editor half-page"
    );
}

#[test]
fn explicit_scroll_option_overrides_window_height() {
    let mut test = EditorTest::new(&long_buffer());
    test.editor.init_window_manager(80, 24);
    test.editor.set_viewport_height(24);
    test.editor.options.scroll = Some(3);

    assert_eq!(
        test.editor.half_page_scroll(),
        3,
        ":set scroll=N takes precedence over the computed half-page"
    );
}
