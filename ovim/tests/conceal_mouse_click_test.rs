//! Clicking on a concealed (non-cursor) markdown line must map the on-screen
//! display column back through the *concealed* text, not the raw source.
//!
//! The renderer draws `![diagram](really/long/path.png)` as just `⧉ diagram`.
//! A click on that short label was being mapped straight through the raw line
//! (`display_col_to_char_col` on the un-concealed string), so the cursor landed
//! several characters off — somewhere inside the hidden image path. The cursor
//! should instead land at the start of the concealed span (where the line
//! reveals itself for editing).

use ovim::editor::{handle_mouse_event, Editor};
use ovim_core::{MouseButton, MouseEvent, MouseEventKind, Rect};

fn md_editor(content: &str) -> Editor {
    let mut editor = Editor::with_content(content);
    editor.options.clipboard = String::new();
    editor.buffer_mut().set_file_path("notes.md".to_string());
    editor.options.markdown_conceal = true;
    editor.options.wrap = false;
    // Pretend a frame was rendered: full-width buffer area, no gutter.
    editor.render_cache.last_buffer_area = Some(Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 24,
    });
    editor.render_cache.last_gutter_width = 0;
    editor.render_cache.last_blame_width = 0;
    editor
}

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
fn click_on_concealed_image_line_lands_in_span() {
    // Line 1 conceals to `⧉ diagram`; raw text is far longer. Cursor starts on
    // line 0, so line 1 is concealed by the renderer.
    let content = "intro\n![diagram](really/long/path/to/image.png)\noutro";
    let mut editor = md_editor(content);

    // Click a few columns into the concealed label on line 1 (row 1, no gutter).
    left_click(&mut editor, 3, 1);

    let cursor = editor.buffer().cursor();
    assert_eq!(cursor.line(), 1, "click selects the concealed image line");
    // The whole line is one concealed span, so any click within the visible
    // label maps to the start of that span (char 0). The pre-fix bug mapped the
    // display column straight through the raw text and landed at char 3 ('i').
    assert_eq!(
        cursor.col().0,
        0,
        "cursor lands at the start of the concealed span, not mid-raw-text"
    );
}

#[test]
fn click_on_cursor_line_is_unaffected_by_conceal() {
    // When the click lands on the line the cursor is already on, that line is
    // revealed (raw) by the renderer, so the column maps 1:1 through raw text.
    let content = "![diagram](really/long/path/to/image.png)\nsecond";
    let mut editor = md_editor(content);
    // Cursor is on line 0 by default → line 0 is revealed.
    left_click(&mut editor, 4, 0);

    let cursor = editor.buffer().cursor();
    assert_eq!(cursor.line(), 0);
    assert_eq!(
        cursor.col().0,
        4,
        "revealed cursor line maps display col straight to raw char col"
    );
}
