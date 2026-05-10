//! Roadmap 19 / OV-00209: split panes wrap at their *own* width, not the
//! focused pane's. These render a real split layout (via the headless ANSI
//! renderer, which goes through `render_buffer_area` → `render_window_tree`)
//! and check each window's per-window wrap map.

mod helpers;

use helpers::EditorTest;
use ovim::ui::render_editor_to_ansi;

/// A line long enough to wrap at ~half of an 80-col terminal but not at the
/// full width (52 cols → 1 row at ~78, ≥2 rows at ~39).
fn long_line() -> String {
    "x".repeat(52)
}

#[test]
fn split_panes_get_their_own_wrap_map() {
    let mut test = EditorTest::new(&format!("{}\nsecond line\n", long_line()));
    test.editor.options.wrap = true;
    test.keys("<C-w>v"); // vertical split → window 0 (left) + window 1 (right)
    assert_eq!(test.editor.window_count(), 2);

    // A full render wires each pane's wrap map at its own content width.
    let _ = render_editor_to_ansi(&mut test.editor, 80, 24).expect("render");

    let wm = test.editor.window_manager().expect("window manager");
    let w0 = wm.get_window(0).and_then(|w| w.wrap_map()).expect("w0 map");
    let w1 = wm.get_window(1).and_then(|w| w.wrap_map()).expect("w1 map");

    // Both wrap at roughly half the terminal — *not* the full ~78 cols that the
    // editor-global map used pre-19.3 (which made every pane wrap identically).
    assert!(
        w0.wrap_width() < 50,
        "window 0 should wrap near its own ~39-col width, got {}",
        w0.wrap_width()
    );
    assert!(
        w1.wrap_width() < 50,
        "window 1 should wrap near its own ~39-col width, got {}",
        w1.wrap_width()
    );
    // 52-char line: ≥2 visual rows at ~39 cols (would be 1 at ~78).
    assert!(w0.visual_lines_for(0) >= 2, "line should wrap in pane 0");
    assert!(w1.visual_lines_for(0) >= 2, "line should wrap in pane 1");

    // The editor-global fallback slot stays untouched while a WindowManager
    // exists — it's only the headless path.
    assert!(test.editor.viewport.wrap_map.is_none());
}

#[test]
fn focus_switch_does_not_reflow_either_pane() {
    let mut test = EditorTest::new(&format!("{}\n", long_line()));
    test.editor.options.wrap = true;
    test.keys("<C-w>v");
    let _ = render_editor_to_ansi(&mut test.editor, 80, 24).expect("render");

    let rows0_before = test
        .editor
        .window_manager()
        .unwrap()
        .get_window(0)
        .and_then(|w| w.wrap_map())
        .unwrap()
        .visual_lines_for(0);
    let rows1_before = test
        .editor
        .window_manager()
        .unwrap()
        .get_window(1)
        .and_then(|w| w.wrap_map())
        .unwrap()
        .visual_lines_for(0);
    // The focused-window accessor was already at a pane width (not ~78).
    assert!(test.editor.wrap_map().unwrap().wrap_width() < 50);

    test.keys("<C-w>l"); // focus the right pane
    let _ = render_editor_to_ansi(&mut test.editor, 80, 24).expect("render");

    assert_eq!(
        test.editor.window_manager().unwrap().focused_window_index(),
        1
    );
    // Accessor follows focus and is still at a pane width.
    assert!(test.editor.wrap_map().unwrap().wrap_width() < 50);
    // Neither pane's wrap geometry changed just because focus moved.
    assert_eq!(
        test.editor
            .window_manager()
            .unwrap()
            .get_window(0)
            .and_then(|w| w.wrap_map())
            .unwrap()
            .visual_lines_for(0),
        rows0_before,
        "pane 0 reflowed on focus change"
    );
    assert_eq!(
        test.editor
            .window_manager()
            .unwrap()
            .get_window(1)
            .and_then(|w| w.wrap_map())
            .unwrap()
            .visual_lines_for(0),
        rows1_before,
        "pane 1 reflowed on focus change"
    );
}

#[test]
fn single_window_render_still_uses_the_global_slot() {
    // No split → no `WindowManager`-driven per-window path needed; the renderer
    // ensures the global slot and `wrap_map()` reads it. (Regression guard for
    // the headless / single-window case.)
    let mut test = EditorTest::new(&format!("{}\n", long_line()));
    test.editor.options.wrap = true;
    let _ = render_editor_to_ansi(&mut test.editor, 80, 24).expect("render");

    // The renderer creates a 1-window WindowManager, so `wrap_map()` resolves
    // to that window's slot — and it must be populated and at ~full width.
    let map = test.editor.wrap_map().expect("wrap map after render");
    assert!(
        map.wrap_width() > 50,
        "single window should wrap near the full 80-col width, got {}",
        map.wrap_width()
    );
    // 52 chars fit on one row at ~78 cols.
    assert_eq!(map.visual_lines_for(0), 1);
}
