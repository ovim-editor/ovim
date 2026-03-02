/// Comprehensive tests for viewport scroll commands: zt, zz, zb, z<CR>, z., z-
///
/// Vim semantics reference:
///   zt      — Scroll so cursor line is at top of viewport. Cursor column unchanged.
///   zz      — Scroll so cursor line is at center of viewport. Cursor column unchanged.
///   zb      — Scroll so cursor line is at bottom of viewport. Cursor column unchanged.
///   z<CR>   — Like zt, but cursor moves to first non-blank of line.
///   z.      — Like zz, but cursor moves to first non-blank of line.
///   z-      — Like zb, but cursor moves to first non-blank of line.
///
/// [count] for all of the above means "use line [count] instead of cursor line"
/// and moves the cursor to that line. (1-indexed, so 5zt = line 5 at top.)
mod helpers;
#[path = "helpers/viewport_assertions.rs"]
mod viewport_assertions;

use helpers::EditorTest;
use viewport_assertions::ViewportAssertion;

/// Generate N lines of "Line {i}" content (1-indexed text, 0-indexed buffer lines).
fn make_content(n: usize) -> String {
    (1..=n)
        .map(|i| format!("Line {}", i))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generate N lines with leading whitespace for first-non-blank testing.
fn make_indented_content(n: usize) -> String {
    (1..=n)
        .map(|i| format!("    Line {}", i))
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

// ==========================================================================
// zt — cursor line at top
// ==========================================================================

#[test]
fn zt_basic() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j"); // cursor on line 24 (0-indexed)
    test.keys("zt");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24, "cursor stays on line 24");
    assert_eq!(vp.scroll_offset(), 24, "line 24 is at viewport top");
    assert_eq!(vp.line_at_viewport_position(0), 24);
}

#[test]
fn zt_cursor_at_line_zero() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("zt"); // cursor already at line 0

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 0);
    assert_eq!(vp.scroll_offset(), 0);
}

#[test]
fn zt_near_end_of_file() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("48j"); // cursor on line 48
    test.keys("zt");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 48);
    // Can't scroll past EOF: max_scroll = 50 - 20 = 30
    assert_eq!(vp.scroll_offset(), 30, "clamped at EOF");
}

#[test]
fn zt_preserves_column() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j5l"); // line 24, col 5
    test.keys("zt");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24);
    assert_eq!(vp.cursor_col(), 5, "zt preserves cursor column");
}

#[test]
fn zt_is_idempotent() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j");
    test.keys("zt");
    let offset1 = ViewportAssertion::new(&test.editor).scroll_offset();
    test.keys("zt");
    let offset2 = ViewportAssertion::new(&test.editor).scroll_offset();
    assert_eq!(offset1, offset2, "zt applied twice should be the same");
}

// ==========================================================================
// zz — cursor line at center
// ==========================================================================

#[test]
fn zz_basic() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j");
    test.keys("zz");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24);
    // center_offset = 20/2 = 10, scroll = 24 - 10 = 14
    assert_eq!(vp.scroll_offset(), 14);
}

#[test]
fn zz_cursor_at_line_zero() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("zz");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 0);
    assert_eq!(vp.scroll_offset(), 0, "can't scroll negative");
}

#[test]
fn zz_near_end_of_file() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("49j"); // line 49 (last line)
    test.keys("zz");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 49);
    // center would be 49 - 10 = 39, but max_scroll = 50-20 = 30
    assert_eq!(vp.scroll_offset(), 30, "clamped at EOF");
}

#[test]
fn zz_preserves_column() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j3l");
    test.keys("zz");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_col(), 3, "zz preserves cursor column");
}

#[test]
fn zz_is_idempotent() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j");
    test.keys("zz");
    let offset1 = ViewportAssertion::new(&test.editor).scroll_offset();
    test.keys("zz");
    let offset2 = ViewportAssertion::new(&test.editor).scroll_offset();
    assert_eq!(offset1, offset2);
}

// ==========================================================================
// zb — cursor line at bottom
// ==========================================================================

#[test]
fn zb_basic() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j");
    test.keys("zb");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24);
    // bottom_position = 19, scroll = 24 - 19 = 5
    assert_eq!(vp.scroll_offset(), 5);
    assert_eq!(vp.line_at_viewport_position(19), 24);
}

#[test]
fn zb_cursor_near_top() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("5j"); // line 5, not enough lines above to fill viewport
    test.keys("zb");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 5);
    assert_eq!(vp.scroll_offset(), 0, "can't scroll negative");
}

#[test]
fn zb_cursor_at_line_zero() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("zb");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 0);
    assert_eq!(vp.scroll_offset(), 0);
}

#[test]
fn zb_preserves_column() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j4l");
    test.keys("zb");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_col(), 4, "zb preserves cursor column");
}

#[test]
fn zb_is_idempotent() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j");
    test.keys("zb");
    let offset1 = ViewportAssertion::new(&test.editor).scroll_offset();
    test.keys("zb");
    let offset2 = ViewportAssertion::new(&test.editor).scroll_offset();
    assert_eq!(offset1, offset2);
}

// ==========================================================================
// z<CR> — like zt, but cursor moves to first non-blank
// ==========================================================================

#[test]
fn z_enter_scrolls_like_zt() {
    let content = make_indented_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j5l"); // line 24, col 5
    test.keys("z<CR>");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24);
    assert_eq!(vp.scroll_offset(), 24, "z<CR> scrolls like zt");
}

#[test]
fn z_enter_moves_to_first_non_blank() {
    let content = make_indented_content(50); // 4-space indent
    let mut test = setup(&content, 20);
    test.keys("24j"); // cursor at col 0
    test.keys("z<CR>");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_col(), 4, "z<CR> moves cursor to first non-blank (col 4)");
}

// ==========================================================================
// z. — like zz, but cursor moves to first non-blank
// ==========================================================================

#[test]
fn z_dot_scrolls_like_zz() {
    let content = make_indented_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j5l");
    test.keys("z.");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24);
    assert_eq!(vp.scroll_offset(), 14, "z. scrolls like zz");
}

#[test]
fn z_dot_moves_to_first_non_blank() {
    let content = make_indented_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j");
    test.keys("z.");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_col(), 4, "z. moves to first non-blank");
}

// ==========================================================================
// z- — like zb, but cursor moves to first non-blank
// ==========================================================================

#[test]
fn z_minus_scrolls_like_zb() {
    let content = make_indented_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j5l");
    test.keys("z-");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24);
    assert_eq!(vp.scroll_offset(), 5, "z- scrolls like zb");
}

#[test]
fn z_minus_moves_to_first_non_blank() {
    let content = make_indented_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j");
    test.keys("z-");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_col(), 4, "z- moves to first non-blank");
}

// ==========================================================================
// [count] with viewport commands — count is a LINE NUMBER (1-indexed)
// ==========================================================================

#[test]
fn count_zt_moves_cursor_to_target_line() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    // 10zt should put line 10 (1-indexed) at top and move cursor there
    test.keys("10zt");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 9, "10zt moves cursor to line 9 (0-indexed)");
    assert_eq!(vp.scroll_offset(), 9, "line 9 at viewport top");
}

#[test]
fn count_zz_moves_cursor_to_target_line() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    // 25zz should center line 25 (1-indexed = line 24 0-indexed)
    test.keys("25zz");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24, "25zz moves cursor to line 24 (0-indexed)");
    assert_eq!(vp.scroll_offset(), 14, "line 24 centered: 24 - 10 = 14");
}

#[test]
fn count_zb_moves_cursor_to_target_line() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    // 25zb should put line 25 (1-indexed = line 24 0-indexed) at bottom
    test.keys("25zb");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24, "25zb moves cursor to line 24 (0-indexed)");
    assert_eq!(vp.scroll_offset(), 5, "line 24 at bottom: 24 - 19 = 5");
}

#[test]
fn count_z_enter_moves_to_first_non_blank() {
    let content = make_indented_content(50);
    let mut test = setup(&content, 20);
    test.keys("10z<CR>");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 9, "10z<CR> moves to line 9 (0-indexed)");
    assert_eq!(vp.scroll_offset(), 9, "line 9 at top");
    assert_eq!(vp.cursor_col(), 4, "z<CR> moves to first non-blank");
}

#[test]
fn count_z_dot_moves_to_first_non_blank() {
    let content = make_indented_content(50);
    let mut test = setup(&content, 20);
    test.keys("25z.");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24, "25z. moves to line 24 (0-indexed)");
    assert_eq!(vp.scroll_offset(), 14, "centered");
    assert_eq!(vp.cursor_col(), 4, "z. moves to first non-blank");
}

#[test]
fn count_z_minus_moves_to_first_non_blank() {
    let content = make_indented_content(50);
    let mut test = setup(&content, 20);
    test.keys("25z-");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24, "25z- moves to line 24 (0-indexed)");
    assert_eq!(vp.scroll_offset(), 5, "line 24 at bottom");
    assert_eq!(vp.cursor_col(), 4, "z- moves to first non-blank");
}

// ==========================================================================
// Edge cases: file smaller than viewport
// ==========================================================================

#[test]
fn zt_file_smaller_than_viewport() {
    let content = make_content(10); // 10 lines, viewport 20
    let mut test = setup(&content, 20);
    test.keys("5j");
    test.keys("zt");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 5);
    // max_scroll = max(0, 10 - 20) = 0
    assert_eq!(vp.scroll_offset(), 0, "can't scroll when file fits in viewport");
}

#[test]
fn zz_file_smaller_than_viewport() {
    let content = make_content(10);
    let mut test = setup(&content, 20);
    test.keys("5j");
    test.keys("zz");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 5);
    assert_eq!(vp.scroll_offset(), 0, "can't scroll when file fits in viewport");
}

#[test]
fn zb_file_smaller_than_viewport() {
    let content = make_content(10);
    let mut test = setup(&content, 20);
    test.keys("5j");
    test.keys("zb");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 5);
    assert_eq!(vp.scroll_offset(), 0, "can't scroll when file fits in viewport");
}

// ==========================================================================
// Persistence: viewport position persists after cursor movement
// ==========================================================================

#[test]
fn zt_persists_after_j() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j");
    test.keys("zt");
    test.keys("j"); // move down within viewport

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 25);
    assert_eq!(vp.scroll_offset(), 24, "scroll_offset persists after j");
}

#[test]
fn zz_persists_after_j() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j");
    test.keys("zz");
    test.keys("j");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 25);
    assert_eq!(vp.scroll_offset(), 14, "scroll_offset persists after j");
}

#[test]
fn zt_then_k_scrolls_cursor_back_in() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j");
    test.keys("zt"); // scroll=24, viewport shows 24-43
    test.keys("k"); // cursor goes to 23, above viewport

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 23);
    assert_eq!(vp.scroll_offset(), 23, "viewport scrolls to keep cursor visible");
}

#[test]
fn zb_then_j_scrolls_cursor_back_in() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j");
    test.keys("zb"); // scroll=5, viewport shows 5-24
    test.keys("j"); // cursor goes to 25, below viewport

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 25);
    assert_eq!(vp.scroll_offset(), 6, "viewport scrolls to keep cursor visible");
}

// ==========================================================================
// Sequences: combining viewport commands
// ==========================================================================

#[test]
fn zt_then_zb() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j");
    test.keys("zt"); // scroll=24
    test.keys("zb"); // cursor still on 24, now at bottom → scroll=5

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24);
    assert_eq!(vp.scroll_offset(), 5);
}

#[test]
fn zb_then_zt() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j");
    test.keys("zb"); // scroll=5
    test.keys("zt"); // cursor still on 24, now at top → scroll=24

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24);
    assert_eq!(vp.scroll_offset(), 24);
}

#[test]
fn zt_then_zz_then_zb() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j");
    test.keys("zt");
    assert_eq!(ViewportAssertion::new(&test.editor).scroll_offset(), 24);
    test.keys("zz");
    assert_eq!(ViewportAssertion::new(&test.editor).scroll_offset(), 14);
    test.keys("zb");
    assert_eq!(ViewportAssertion::new(&test.editor).scroll_offset(), 5);
}

// ==========================================================================
// Scrolloff interaction — invocation time
// ==========================================================================
//
// Vim behavior: zt/zz/zb themselves ignore the scrolloff setting.
// The cursor goes right to the edge. Scrolloff only re-engages on
// subsequent cursor movements (j/k/etc).

#[test]
fn zt_respects_scrolloff() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 5;
    test.keys("24j");
    test.keys("zt");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24);
    // zt respects scrolloff: cursor is scrolloff lines from top
    assert_eq!(vp.scroll_offset(), 19, "zt respects scrolloff: 24-5=19");
}

#[test]
fn zz_ignores_scrolloff() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 5;
    test.keys("24j");
    test.keys("zz");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24);
    assert_eq!(vp.scroll_offset(), 14, "zz centers at 24-10=14, ignoring scrolloff");
}

#[test]
fn zb_respects_scrolloff() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 5;
    test.keys("24j");
    test.keys("zb");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24);
    // zb respects scrolloff: cursor is scrolloff lines from bottom
    // bottom_position = 19 - 5 = 14, scroll = 24 - 14 = 10
    assert_eq!(vp.scroll_offset(), 10, "zb respects scrolloff: 24-(19-5)=10");
}

#[test]
fn z_enter_respects_scrolloff() {
    let content = make_indented_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 5;
    test.keys("24j");
    test.keys("z<CR>");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.scroll_offset(), 19, "z<CR> respects scrolloff like zt: 24-5=19");
}

#[test]
fn z_dot_ignores_scrolloff() {
    let content = make_indented_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 5;
    test.keys("24j");
    test.keys("z.");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.scroll_offset(), 14, "z. ignores scrolloff like zz");
}

#[test]
fn z_minus_respects_scrolloff() {
    let content = make_indented_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 5;
    test.keys("24j");
    test.keys("z-");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.scroll_offset(), 10, "z- respects scrolloff like zb: 24-(19-5)=10");
}

// ==========================================================================
// Scrolloff interaction — re-engagement after movement
// ==========================================================================
//
// After a viewport command positions the cursor at an edge, the very next
// j/k should cause the viewport to scroll so scrolloff is respected again.

#[test]
fn zt_then_j_no_scroll_needed() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 5;
    test.keys("24j");
    test.keys("zt"); // scroll=19, cursor at row 5 (scrolloff)
    test.keys("j");  // cursor moves to line 25, row 6 — still in safe zone

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 25);
    // Cursor at row 6 (25-19), safe zone is [5, 14]. No scroll needed.
    assert_eq!(vp.scroll_offset(), 19,
        "after zt+j, cursor stays in safe zone — no jump");
}

#[test]
fn zb_then_k_no_scroll_needed() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 5;
    test.keys("24j");
    test.keys("zb"); // scroll=10, cursor at row 14 (scrolloff from bottom)
    test.keys("k");  // cursor moves to line 23, row 13 — still in safe zone

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 23);
    // Cursor at row 13 (23-10), safe zone is [5, 14]. No scroll needed.
    assert_eq!(vp.scroll_offset(), 10,
        "after zb+k, cursor stays in safe zone — no jump");
}

#[test]
fn zz_then_j_within_scrolloff_does_not_scroll() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 3;
    test.keys("24j");
    test.keys("zz"); // cursor at center (row 10), scroll=14

    // Moving down 1 line — cursor is at center, plenty of room below
    test.keys("j");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 25);
    assert_eq!(vp.scroll_offset(), 14,
        "j from center doesn't trigger scroll (cursor still within scrolloff bounds)");
}

#[test]
fn zt_then_multiple_j_scrolloff_tracks() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 3;
    test.keys("24j");
    test.keys("zt"); // scroll=21 (24-3), cursor at row 3

    // First j: cursor at line 25, row 4. Safe zone [3, 16]. In zone.
    test.keys("j");
    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.scroll_offset(), 21,
        "after zt + 1j: cursor in safe zone, no scroll");

    // Second j: cursor at line 26, row 5. In zone.
    test.keys("j");
    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.scroll_offset(), 21,
        "after zt + 2j: cursor within safe zone, no scroll");

    // Third j: cursor at line 27, row 6. Still in zone.
    test.keys("j");
    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.scroll_offset(), 21,
        "after zt + 3j: cursor within safe zone, no scroll");
}

#[test]
fn zb_then_multiple_k_scrolloff_tracks() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 3;
    test.keys("24j");
    test.keys("zb"); // scroll=8 (24-16), cursor at row 16

    // First k: cursor at line 23, row 15. Safe zone [3, 16]. In zone.
    test.keys("k");
    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.scroll_offset(), 8,
        "after zb + 1k: cursor in safe zone, no scroll");

    // Second k: cursor at line 22, row 14. In zone.
    test.keys("k");
    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.scroll_offset(), 8,
        "after zb + 2k: cursor within safe zone, no scroll");
}

// ==========================================================================
// Scrolloff — large values (scrolloff > viewport/2)
// ==========================================================================

#[test]
fn zt_with_scrolloff_larger_than_half_viewport() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 15; // > 20/2, clamped to 9
    test.keys("24j");
    test.keys("zt");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24);
    // scrolloff=15 clamped to (20-1)/2=9, scroll = 24-9 = 15
    assert_eq!(vp.scroll_offset(), 15, "zt with large scrolloff clamped to 9: 24-9=15");
}

#[test]
fn zb_with_scrolloff_larger_than_half_viewport() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 15; // clamped to 9
    test.keys("24j");
    test.keys("zb");

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24);
    // scrolloff=15 clamped to 9, bottom_position = 19-9 = 10, scroll = 24-10 = 14
    assert_eq!(vp.scroll_offset(), 14, "zb with large scrolloff clamped to 9: 24-10=14");
}

// ==========================================================================
// Scrolloff — interaction with [count] variants
// ==========================================================================

#[test]
fn count_zt_with_scrolloff_respects_scrolloff() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 5;
    test.keys("25zt"); // line 25 (1-indexed) → line 24 (0-indexed)

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24);
    assert_eq!(vp.scroll_offset(), 19, "[count]zt respects scrolloff: 24-5=19");
}

#[test]
fn count_zb_with_scrolloff_respects_scrolloff() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 5;
    test.keys("25zb"); // line 25 at bottom

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 24);
    assert_eq!(vp.scroll_offset(), 10, "[count]zb respects scrolloff: 24-(19-5)=10");
}

#[test]
fn count_zt_then_j_no_scroll_needed() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 5;
    test.keys("25zt"); // scroll=19, cursor at row 5
    test.keys("j");    // cursor 25, row 6 — in safe zone

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 25);
    assert_eq!(vp.scroll_offset(), 19,
        "[count]zt + j: cursor in safe zone, no scroll");
}

// ==========================================================================
// Scrolloff — various scrolloff values
// ==========================================================================

#[test]
fn zt_then_j_with_scrolloff_0() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 0;
    test.keys("24j");
    test.keys("zt"); // scroll=24
    test.keys("j");  // line 25, scrolloff=0 → no scroll needed, cursor at row 1

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 25);
    assert_eq!(vp.scroll_offset(), 24, "scrolloff=0: no scroll needed after j");
}

#[test]
fn zt_then_j_with_scrolloff_1() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 1;
    test.keys("24j");
    test.keys("zt"); // scroll=23 (24-1), cursor at row 1
    test.keys("j");  // line 25, row 2. Safe zone [1,18]. In zone.

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 25);
    assert_eq!(vp.scroll_offset(), 23,
        "scrolloff=1: cursor in safe zone, no scroll");
}

#[test]
fn zt_then_j_with_scrolloff_10() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 10;
    test.keys("24j");
    // scrolloff=10 clamped to 9. zt: scroll = 24-9 = 15, cursor at row 9
    test.keys("zt");
    test.keys("j");  // cursor 25, row 10. Safe zone [9, 10]. In zone.

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 25);
    assert_eq!(vp.scroll_offset(), 15,
        "scrolloff=10 clamped to 9: cursor in safe zone, no scroll");
}

#[test]
fn zb_then_k_with_scrolloff_1() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.editor.options.scrolloff = 1;
    test.keys("24j");
    test.keys("zb"); // scroll=6 (24-18), cursor at row 18
    test.keys("k");  // cursor 23, row 17. Safe zone [1,18]. In zone.

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 23);
    assert_eq!(vp.scroll_offset(), 6,
        "scrolloff=1: cursor in safe zone, no scroll");
}

// ==========================================================================
// Count edge cases
// ==========================================================================

#[test]
fn count_1_zt_is_same_as_going_to_line_1() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("24j"); // start at line 24
    test.keys("1zt"); // should go to line 1 (0-indexed: 0) and put it at top

    let vp = ViewportAssertion::new(&test.editor);
    assert_eq!(vp.cursor_line(), 0, "1zt goes to line 0 (0-indexed)");
    assert_eq!(vp.scroll_offset(), 0);
}

#[test]
fn count_beyond_file_clamps_to_last_line() {
    let content = make_content(50);
    let mut test = setup(&content, 20);
    test.keys("999zt");

    let vp = ViewportAssertion::new(&test.editor);
    // Should clamp to last line
    assert_eq!(vp.cursor_line(), 49, "count beyond EOF clamps to last line");
}
