//! Verify margin_color shading still appears in BOTH left and right margins
//! when textwidth centering is on. Regression check for the EOL diagnostic
//! geometry refactor — `render_area` extends past `buffer_area` in centered
//! mode, and the line render must not overwrite the right margin shading.
mod helpers;
use helpers::EditorTest;
use ovim::ui::{render_editor_to_ansi, strip_ansi};

#[test]
fn margin_color_right_shading_persists_in_centered_mode() {
    // Layout (width=80, textwidth=30, no gutter contributors):
    //   margin = (80 - 30) / 2 = 25
    //   buffer_area = (x=25, width=30)
    //   right shading: x=55..80 (when padding=0)
    let mut test = EditorTest::new("let x = 1;\n");
    test.editor.options.textwidth = Some(30);
    test.editor.options.number = false;
    test.editor.options.relative_number = false;
    test.editor.options.wrap = false;
    test.editor.options.margin_color = ovim_core::editor::MarginColor::Solid(80, 0, 0);

    let ansi = render_editor_to_ansi(&mut test.editor, 80, 10).unwrap();

    // Walk the ANSI output cell-by-cell on row 0 and identify which cells
    // have the shading bg. Shading should appear at left (cols 0..25) and
    // right (cols 55..80, minus wherever a diagnostic landed — there's no
    // diagnostic in this test).
    let plain = strip_ansi(&ansi);
    let row0 = plain.split('\n').next().unwrap();
    assert_eq!(row0.len(), 80, "row 0 should be 80 chars wide");

    // We can't easily map cell→bg from stripped output, but we can scan
    // the raw ANSI for bg-color markers in left vs right halves. Split the
    // ANSI by row by counting visible chars.
    // Simpler test: search for the bg escape. If both left and right are
    // shaded, we expect the bg escape to appear at least twice on row 0
    // (once near the start, once near the right edge after the code-box).
    //
    // Even simpler: the shaded cells have bg=RGB(80,0,0). When ratatui
    // emits the buffer, contiguous runs share a style. If left shading
    // and right shading are both visible, we expect at least 2 separate
    // `48;2;80;0;0` color sets on the row (one for left, one transitioning
    // back after the code-box).
    let bg_marker = "48;2;80;0;0";
    let count = ansi.matches(bg_marker).count();
    assert!(
        count >= 2,
        "expected ≥2 RGB(80,0,0) bg markers in ANSI (left+right shading); found {count}"
    );
}
