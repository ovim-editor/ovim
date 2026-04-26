//! Diagnostic vtext placement in centered (textwidth) mode.
//!
//! When `textwidth` centering is enabled, all EOL diagnostic rendering goes
//! through `render_diagnostic_virtual_text_overlay`. The desired model: the
//! centered code-box (width = `text_width`) holds document content + inline
//! decorations (inlay hints); diagnostics live in the right margin, always
//! anchored at `text_area_right + 2`, regardless of how long the actual
//! line of code is.
//!
//! These tests pin that model. They fail on `main` because today the
//! overlay places diagnostics at `text_area_x + raw_code_width + 2`, which
//! (a) puts the diagnostic mid-band on short lines and (b) overlaps any
//! inlay hint widening the line beyond raw char count.
mod helpers;

use helpers::EditorTest;
use ovim_core::editor::decoration::{
    Decoration, DecorationPlacement, DecorationSource, DecorationStyle,
};

fn hint(char_offset: usize, text: &str) -> Decoration {
    Decoration {
        placement: DecorationPlacement::Inline { char_offset },
        source: DecorationSource::InlayHint,
        text: text.to_string(),
        display_width: text.chars().count(),
        style: DecorationStyle::new(ovim_core::color::Color::Gray).with_italic(),
        priority: 10,
        source_version: 0,
    }
}

fn diag(line_start_offset: usize, text: &str) -> Decoration {
    Decoration {
        placement: DecorationPlacement::EndOfLine {
            char_offset: line_start_offset,
        },
        source: DecorationSource::Diagnostic,
        text: text.to_string(),
        display_width: text.chars().count(),
        style: DecorationStyle::new(ovim_core::color::Color::Red),
        priority: 10,
        source_version: 0,
    }
}

/// Build a centered editor with textwidth=30 in an 80-wide terminal.
/// Numbers off so gutter is just SIGN_WIDTH(2) + GUTTER_SPACING(1) = 3.
///
/// Layout:
///   margin         = (80 - 30) / 2 = 25
///   buffer_area    = (x=25, width=30)
///   gutter_width   = 3
///   text_area_x    = 25 + 3 = 28
///   text_width     = 30 - 3 = 27
///   text_area_right = 28 + 27 = 55
fn centered(content: &str, wrap: bool) -> EditorTest {
    let mut test = EditorTest::new(content);
    test.editor.options.textwidth = Some(30);
    test.editor.options.number = false;
    test.editor.options.relative_number = false;
    test.editor.options.wrap = wrap;
    test.editor.options.scrolloff = 0;
    test
}

const TEXT_AREA_X: usize = 28;
const TEXT_AREA_RIGHT: usize = 55;
const DIAG_GAP: usize = 2;
const EXPECTED_DIAG_START: usize = TEXT_AREA_RIGHT + DIAG_GAP; // 57

fn render_rows(test: &mut EditorTest) -> Vec<String> {
    let ansi = ovim::ui::render_editor_to_ansi(&mut test.editor, 80, 10).unwrap();
    let plain = ovim::ui::strip_ansi(&ansi);
    plain.split('\n').map(|s| s.to_string()).collect()
}

fn replace_hints(test: &mut EditorTest, hints: Vec<Decoration>) {
    let rope = test.editor.buffer().rope().clone();
    test.editor
        .decorations
        .replace_source(DecorationSource::InlayHint, hints, &rope);
}

fn replace_diags(test: &mut EditorTest, diags: Vec<Decoration>) {
    let rope = test.editor.buffer().rope().clone();
    test.editor
        .decorations
        .replace_source(DecorationSource::Diagnostic, diags, &rope);
}

/// Slice `row` in char units (test rows are ASCII so chars == display columns).
fn cols(row: &str, start: usize, len: usize) -> String {
    row.chars().skip(start).take(len).collect()
}

// ============================================================================
// Tests — desired behavior
// ============================================================================

/// Short line, no hint: diagnostic must sit at the right edge of the centered
/// box (text_area_right + 2), NOT float mid-band right after the code.
///
/// Today: diagnostic appears at `text_area_x + 10 ("let x = 1;") + 2 = 40`,
/// inside the centered band — visually intrudes into the reading column.
#[test]
fn centered_short_line_no_hint_diag_in_right_margin() {
    let mut test = centered("let x = 1;\n", false);
    replace_diags(&mut test, vec![diag(0, "unused")]);

    let rows = render_rows(&mut test);
    let row = &rows[0];

    // Code rendered inside the box at text_area_x.
    assert_eq!(
        cols(row, TEXT_AREA_X, 10),
        "let x = 1;",
        "code should render at TEXT_AREA_X; row was {row:?}"
    );

    // Diagnostic in the right margin at text_area_right + 2.
    assert_eq!(
        cols(row, EXPECTED_DIAG_START, 6),
        "unused",
        "diagnostic should start at TEXT_AREA_RIGHT + 2 (col {EXPECTED_DIAG_START}); row was {row:?}"
    );

    // Two-space gap between box edge and diagnostic.
    assert_eq!(
        cols(row, TEXT_AREA_RIGHT, DIAG_GAP),
        "  ",
        "expected blank gap between code-box and diagnostic; row was {row:?}"
    );

    // Nothing diagnostic-shaped should bleed into the centered band.
    let band_after_code = cols(row, TEXT_AREA_X + 10, TEXT_AREA_RIGHT - (TEXT_AREA_X + 10));
    assert!(
        band_after_code.chars().all(|c| c == ' '),
        "no diagnostic chars should appear inside the code-box past line end; saw {band_after_code:?}"
    );
}

/// Short line WITH inlay hint: hint widens the visible line, diagnostic must
/// still land at text_area_right + 2 — never overlapping the hint or code.
///
/// Today: diagnostic placed at `text_area_x + raw_code_width("let x = 1;") + 2 = 40`,
/// which sits *inside* the rendered "let x: i32 = 1;" (cols 28..43). The
/// overlay's 2-space style then overwrites code that came after the hint.
#[test]
fn centered_short_line_with_hint_diag_does_not_overlap() {
    let mut test = centered("let x = 1;\n", false);
    // Hint at offset 5 ("let x" then ": i32" then " = 1;") widens display to 15.
    replace_hints(&mut test, vec![hint(5, ": i32")]);
    replace_diags(&mut test, vec![diag(0, "unused")]);

    let rows = render_rows(&mut test);
    let row = &rows[0];

    // Rendered code with hint inline:
    //   "let x" (5) + ": i32" (5) + " = 1;" (5) = 15 chars at cols 28..43.
    assert_eq!(
        cols(row, TEXT_AREA_X, 15),
        "let x: i32 = 1;",
        "rendered line with hint should be intact; row was {row:?}"
    );

    // Diagnostic still at margin, NOT eating the hint or code.
    assert_eq!(
        cols(row, EXPECTED_DIAG_START, 6),
        "unused",
        "diagnostic should be at TEXT_AREA_RIGHT + 2; row was {row:?}"
    );

    // Everything between the rendered line end and the diagnostic gap is
    // padding spaces — no diagnostic chars bleeding back into the box.
    let between = cols(row, TEXT_AREA_X + 15, TEXT_AREA_RIGHT - (TEXT_AREA_X + 15));
    assert!(
        between.chars().all(|c| c == ' '),
        "expected only spaces between hint-end and box-edge, got {between:?}"
    );
}

/// Longer line plus hint: ensure the diagnostic still anchors at the box
/// edge and doesn't drift further right just because the rendered line is
/// wider.
///
/// Today: diagnostic at `text_area_x + clipped_code_width + 2`, which lands
/// somewhere mid-box because the overlay slices to wrap_width but uses raw
/// chars for code_width — landing inside the rendered hint span.
#[test]
fn centered_longer_line_with_hint_diag_still_at_box_edge() {
    // 16-char line + 5-char hint at offset 5 → rendered 21 chars (still fits in text_width=27).
    //   "let xs = vec![1];"  is 17 chars; we use a 16-char variant for tidy math.
    let mut test = centered("let xs = vec![1];\n", false);
    replace_hints(&mut test, vec![hint(7, ": Vec")]); // after "let xs "
    replace_diags(&mut test, vec![diag(0, "warn")]);

    let rows = render_rows(&mut test);
    let row = &rows[0];

    // Rendered line: "let xs : Vec= vec![1];" — 22 chars at cols 28..50.
    let rendered = "let xs : Vec= vec![1];";
    assert_eq!(
        cols(row, TEXT_AREA_X, rendered.len()),
        rendered,
        "rendered line with hint should be intact; row was {row:?}"
    );

    // Diagnostic at margin regardless of rendered line width.
    assert_eq!(
        cols(row, EXPECTED_DIAG_START, 4),
        "warn",
        "diagnostic should be at TEXT_AREA_RIGHT + 2; row was {row:?}"
    );

    // Gap between rendered-line-end (col 50) and diagnostic start (col 57)
    // is all spaces — no overlay leakage.
    let between = cols(
        row,
        TEXT_AREA_X + rendered.len(),
        EXPECTED_DIAG_START - (TEXT_AREA_X + rendered.len()),
    );
    assert!(
        between.chars().all(|c| c == ' '),
        "expected only spaces between rendered line and diagnostic, got {between:?}"
    );
}

/// Wrap mode counterpart of the short-line-with-hint case. With wrap on,
/// the line + hint still fits on a single visual row (15 cols < text_width=27),
/// so the diagnostic should land on row 0 at the box edge — same rule.
#[test]
fn centered_wrap_short_line_with_hint_diag_at_box_edge() {
    let mut test = centered("let x = 1;\n", true);
    replace_hints(&mut test, vec![hint(5, ": i32")]);
    replace_diags(&mut test, vec![diag(0, "unused")]);

    let rows = render_rows(&mut test);
    let row = &rows[0];

    assert_eq!(
        cols(row, TEXT_AREA_X, 15),
        "let x: i32 = 1;",
        "wrap-mode rendered line should be intact; row was {row:?}"
    );
    assert_eq!(
        cols(row, EXPECTED_DIAG_START, 6),
        "unused",
        "wrap-mode diagnostic should be at TEXT_AREA_RIGHT + 2; row was {row:?}"
    );
}
