use super::*;

/// A buffer line long enough to wrap at narrow widths but not at wide ones.
const LONG_LINE: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"; // 52 chars

#[test]
fn ensure_wrap_map_for_window_uses_per_window_width() {
    let mut editor = Editor::with_content(&format!("{LONG_LINE}\n"));
    editor.options.wrap = true;
    editor.init_window_manager(80, 24);
    editor.split_window_vertical(); // windows 0 and 1, same buffer

    // Build each window's map at a *different* content width.
    editor.ensure_wrap_map_for_window(0, 16);
    editor.ensure_wrap_map_for_window(1, 40);

    let wm = editor.window_manager().expect("window manager");
    let w0 = wm.get_window(0).and_then(|w| w.wrap_map()).expect("w0 map");
    let w1 = wm.get_window(1).and_then(|w| w.wrap_map()).expect("w1 map");

    assert_eq!(w0.wrap_width(), 16, "window 0 wraps at its own width");
    assert_eq!(w1.wrap_width(), 40, "window 1 wraps at its own width");
    // 52 chars: at width 16 → 4 rows; at width 40 → 2 rows.
    assert_eq!(w0.visual_lines_for(0), 4);
    assert_eq!(w1.visual_lines_for(0), 2);
    // The editor-global fallback slot is *not* touched when a WindowManager
    // exists — it's only the headless path.
    assert!(editor.viewport.wrap_map.is_none());
}

#[test]
fn wrap_map_accessor_follows_focus() {
    let mut editor = Editor::with_content(&format!("{LONG_LINE}\n"));
    editor.options.wrap = true;
    editor.init_window_manager(80, 24);
    editor.split_window_vertical();

    editor.ensure_wrap_map_for_window(0, 16);
    editor.ensure_wrap_map_for_window(1, 40);

    // Focus is on window 0 after a vertical split.
    assert_eq!(editor.window_manager().unwrap().focused_window_index(), 0);
    assert_eq!(editor.wrap_map().unwrap().wrap_width(), 16);

    // Move focus to window 1 — the accessor must follow.
    editor.focus_next_window();
    assert_eq!(editor.window_manager().unwrap().focused_window_index(), 1);
    assert_eq!(editor.wrap_map().unwrap().wrap_width(), 40);

    // And `ensure_wrap_map` (no explicit index) drives the focused window.
    editor.ensure_wrap_map(24);
    assert_eq!(editor.wrap_map().unwrap().wrap_width(), 24);
    // window 0's map untouched by the focused-window rebuild.
    assert_eq!(
        editor
            .window_manager()
            .unwrap()
            .get_window(0)
            .and_then(|w| w.wrap_map())
            .unwrap()
            .wrap_width(),
        16
    );
}

#[test]
fn no_window_manager_uses_editor_global_slot() {
    // Headless / test-harness path: no WindowManager → ViewportState slot.
    let mut editor = Editor::with_content(&format!("{LONG_LINE}\n"));
    editor.options.wrap = true;
    assert!(editor.window_manager().is_none());

    editor.ensure_wrap_map(20);
    assert_eq!(editor.wrap_map().unwrap().wrap_width(), 20);
    assert_eq!(editor.viewport.wrap_map.as_ref().unwrap().wrap_width(), 20);
    // 52 chars at width 20 → 3 rows.
    assert_eq!(editor.wrap_map().unwrap().visual_lines_for(0), 3);

    // Turning wrap off clears the slot.
    editor.options.wrap = false;
    editor.ensure_wrap_map(20);
    assert!(editor.wrap_map().is_none());
}

#[test]
fn ensure_wrap_map_for_window_is_noop_without_that_window() {
    let mut editor = Editor::with_content("x\n");
    editor.options.wrap = true;
    editor.init_window_manager(80, 24);
    // Only window 0 exists; targeting index 5 must not panic / allocate.
    editor.ensure_wrap_map_for_window(5, 20);
    assert!(editor.window_manager().unwrap().get_window(5).is_none());
}
