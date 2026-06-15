//! Tests that markdown conceal is reflected in soft-wrap width / row-count math.
//!
//! The renderer draws *concealed* (shorter) text for non-cursor markdown lines
//! (e.g. `[label](https://long/url)` renders as just `label`). But the wrap map
//! — the single source of truth for how many visual rows each logical line
//! occupies, and therefore for cursor row positioning and scrolling — was built
//! from the *raw, un-concealed* line text. That desync means:
//!
//!   * a long link that fits on one row once concealed is counted as several
//!     wrapped rows, inflating `total_visual_lines`, and
//!   * every line *below* a concealed line is pushed down, so the hardware
//!     cursor lands on the wrong screen row in wrap mode.
//!
//! These tests pin the correct (conceal-aware) behaviour. The cursor line is
//! intentionally NOT concealed by the renderer (so editing isn't blind), so the
//! wrap map must mirror that: full width for the cursor line, concealed width
//! for the rest.

mod helpers;
#[path = "helpers/viewport_assertions.rs"]
mod viewport_assertions;

use helpers::EditorTest;
use viewport_assertions::ViewportAssertion;

const WIDTH: usize = 20;

/// A link whose raw form is 57 chars (3 wrapped rows at width 20) but conceals
/// to just `clickme` (7 chars, 1 row).
const LINK: &str = "[clickme](http://example.com/very/long/path/segment/here)";

fn md_editor(content: &str, height: usize) -> EditorTest {
    let mut test = EditorTest::new(content);
    test.editor
        .buffer_mut()
        .set_file_path("notes.md".to_string());
    test.editor.init_window_manager(WIDTH as u16, height as u16);
    test.editor.set_viewport_height(height);
    test.editor.options.wrap = true;
    test.editor.options.markdown_conceal = true;
    test.editor.options.scrolloff = 0;
    test.editor.ensure_wrap_map(WIDTH);
    test
}

#[test]
fn raw_link_wraps_to_three_rows_without_conceal() {
    // Sanity check on the fixture: the raw link really does span 3 rows so the
    // conceal-aware tests below are meaningful.
    let mut test = md_editor(LINK, 10);
    test.editor.options.markdown_conceal = false;
    test.editor.ensure_wrap_map(WIDTH);
    let map = test.editor.wrap_map().expect("wrap map");
    assert_eq!(
        map.visual_lines_for(0),
        3,
        "raw 57-char link should wrap to 3 rows at width {WIDTH}"
    );
}

#[test]
fn concealed_link_on_non_cursor_line_counts_as_one_row() {
    // Cursor sits on line 0; the link is on line 1, so it is concealed by the
    // renderer and should occupy a single visual row.
    let content = format!("intro\n{LINK}\noutro");
    let test = md_editor(&content, 10);
    let map = test.editor.wrap_map().expect("wrap map");

    assert_eq!(map.visual_lines_for(0), 1, "line 0 'intro' is one row");
    assert_eq!(
        map.visual_lines_for(1),
        1,
        "concealed link 'clickme' (7 cols) is one row, not three"
    );
    assert_eq!(map.visual_lines_for(2), 1, "line 2 'outro' is one row");
    // The buffer adds a trailing newline, so there is a phantom empty 4th line
    // (one row). Total = intro(1) + concealed link(1) + outro(1) + phantom(1).
    // The un-concealed bug would inflate this to 6 (the link counts as 3 rows).
    assert_eq!(
        map.total_visual_lines(),
        4,
        "each logical line is one visual row once the link is concealed"
    );
}

#[test]
fn lines_below_a_concealed_link_are_not_pushed_down() {
    // `logical_to_visual` for the line after the concealed link must reflect the
    // concealed (1-row) height, not the raw (3-row) height.
    let content = format!("intro\n{LINK}\noutro");
    let test = md_editor(&content, 10);
    let map = test.editor.wrap_map().expect("wrap map");

    assert_eq!(map.logical_to_visual(1), 1, "link starts at visual row 1");
    assert_eq!(
        map.logical_to_visual(2),
        2,
        "'outro' starts at visual row 2 (intro=1 + concealed link=1), not 4"
    );
}

#[test]
fn cursor_row_below_concealed_link_is_correct() {
    // Put the cursor on the line *after* the concealed link and check the
    // absolute visual row it maps to. This is what drives the hardware cursor
    // position in wrap mode.
    let content = format!("intro\n{LINK}\noutro");
    let mut test = md_editor(&content, 10);
    test.keys("G"); // jump to last line ("outro")
    test.editor.ensure_wrap_map(WIDTH);

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 2, "cursor on 'outro'");
    assert_eq!(
        vp.cursor_absolute_visual_row(),
        2,
        "cursor on 'outro' is at visual row 2, not 4 (link concealed to 1 row)"
    );
}

#[test]
fn cursor_line_is_revealed_in_wrap_map() {
    // When the cursor is ON the link line, the renderer reveals the raw markdown
    // so editing isn't blind. The wrap map must match: that line occupies its
    // full (3-row) height while the cursor is on it.
    let content = format!("intro\n{LINK}\noutro");
    let mut test = md_editor(&content, 10);
    test.keys("j"); // move cursor onto the link line (line 1)
    test.editor.ensure_wrap_map(WIDTH);

    let map = test.editor.wrap_map().expect("wrap map");
    assert_eq!(
        map.visual_lines_for(1),
        3,
        "the cursor line is revealed (raw), so it occupies its full 3 rows"
    );
    assert_eq!(
        map.logical_to_visual(2),
        4,
        "with the link line revealed, 'outro' sits at visual row 4 (1 + 3)"
    );
}
