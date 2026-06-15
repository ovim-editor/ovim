//! Clicking in wrap mode when the viewport is scrolled *into* a wrapped line
//! (`scroll_subrow > 0`) must resolve to the right logical line.
//!
//! The screen→buffer mapping derives the clicked visual row from the viewport's
//! visual-row origin, which is `logical_to_visual(scroll_offset) + scroll_subrow`.
//! Omitting the sub-row term made every click land `scroll_subrow` visual rows
//! too high — the inverse of the cursor-position bug (OV-00019). With a tall
//! wrapped line at the top, that meant clicks selected the wrong line entirely.

use ovim::editor::{handle_mouse_event, Editor};
use ovim_core::{MouseButton, MouseEvent, MouseEventKind, Rect};

const WIDTH: u16 = 20;

fn left_click(editor: &mut Editor, col: u16, row: u16) {
    handle_mouse_event(
        editor,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
        },
    )
    .unwrap();
}

#[test]
fn click_resolves_correct_line_when_scrolled_into_wrapped_line() {
    // Line 0 is 100 chars → 5 visual rows at width 20. Lines 1.. are short.
    let content = format!("{}\nLINE1\nLINE2\nLINE3", "a".repeat(100));
    let mut editor = Editor::with_content(&content);
    editor.options.clipboard = String::new();
    editor.options.wrap = true;
    editor.options.scrolloff = 0;
    editor.init_window_manager(WIDTH, 8);
    editor.set_viewport_height(8);
    editor.ensure_wrap_map(WIDTH as usize);

    // Pretend a frame was rendered: full-width buffer area, no gutter, top-left.
    editor.render_cache.last_buffer_area = Some(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: 8,
    });
    editor.render_cache.last_gutter_width = 0;
    editor.render_cache.last_blame_width = 0;

    // Scroll 3 wrapped rows into line 0. The viewport top is now visual row 3,
    // so the screen shows: line 0's rows 3 & 4, then LINE1 (screen row 2),
    // LINE2 (row 3), LINE3 (row 4).
    if let Some(wm) = editor.window_manager_mut() {
        if let Some(window) = wm.focused_window_mut() {
            window.set_scroll_position(0, 3);
        }
    }

    // Click screen row 2 — that is LINE1. Pre-fix, the omitted sub-row offset
    // mapped row 2 to absolute visual row 2 (still inside line 0).
    left_click(&mut editor, 0, 2);
    assert_eq!(
        editor.buffer().cursor().line(),
        1,
        "click on screen row 2 must select LINE1, not a wrapped segment of line 0"
    );

    // Click screen row 4 — LINE3.
    left_click(&mut editor, 0, 4);
    assert_eq!(
        editor.buffer().cursor().line(),
        3,
        "click on screen row 4 must select LINE3"
    );
}
