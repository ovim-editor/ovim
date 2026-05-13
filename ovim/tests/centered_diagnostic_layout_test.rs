//! Diagnostic vtext placement in centered (textwidth) mode.
//!
//! When `textwidth` centering is enabled, the desired model: the centered
//! code-box (width = `text_width`) holds document content + inline
//! decorations (inlay hints) — code never bleeds past the code-box edge.
//! The diagnostic sits immediately after the rendered code (incl. hints)
//! with a 2-column gap, and is free to extend into the right margin all
//! the way to `render_width`. Short lines get the diagnostic close to the
//! code; lines that fill (or would exceed) the code-box get the diagnostic
//! at the box edge.
//!
//! The "fair game" is the space between the end of rendered code and the
//! right edge of the screen — never the visual reading column to the left
//! of where the code ends.
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

/// Short line, no hint: diagnostic should sit right after the code with a
/// 2-column gap — close to the line, not pinned to the far box edge.
///
/// `let x = 1;` is 10 cols at cols 28..38; gap at 38..40; diag at col 40.
#[test]
fn centered_short_line_no_hint_diag_close_to_code() {
    let mut test = centered("let x = 1;\n", false);
    replace_diags(&mut test, vec![diag(0, "unused")]);

    let rows = render_rows(&mut test);
    let row = &rows[0];

    let code_end = TEXT_AREA_X + 10; // 38
    let diag_start = code_end + DIAG_GAP; // 40

    assert_eq!(
        cols(row, TEXT_AREA_X, 10),
        "let x = 1;",
        "code should render at TEXT_AREA_X; row was {row:?}"
    );
    assert_eq!(
        cols(row, code_end, DIAG_GAP),
        "  ",
        "expected 2-column gap between code and diagnostic; row was {row:?}"
    );
    assert_eq!(
        cols(row, diag_start, 6),
        "unused",
        "diagnostic should start at code_end + gap (col {diag_start}); row was {row:?}"
    );
}

/// Short line WITH inlay hint: diagnostic anchors after the *rendered*
/// line (code + hint) — never overlapping the hint or code.
///
/// Rendered: `let x: i32 = 1;` = 15 cols. Diag at col 28 + 15 + 2 = 45.
#[test]
fn centered_short_line_with_hint_diag_after_rendered_line() {
    let mut test = centered("let x = 1;\n", false);
    replace_hints(&mut test, vec![hint(5, ": i32")]);
    replace_diags(&mut test, vec![diag(0, "unused")]);

    let rows = render_rows(&mut test);
    let row = &rows[0];

    let rendered_end = TEXT_AREA_X + 15; // 43
    let diag_start = rendered_end + DIAG_GAP; // 45

    assert_eq!(
        cols(row, TEXT_AREA_X, 15),
        "let x: i32 = 1;",
        "rendered line with hint should be intact; row was {row:?}"
    );
    assert_eq!(
        cols(row, rendered_end, DIAG_GAP),
        "  ",
        "expected 2-column gap after rendered line; row was {row:?}"
    );
    assert_eq!(
        cols(row, diag_start, 6),
        "unused",
        "diagnostic should anchor at rendered_end + gap (col {diag_start}); row was {row:?}"
    );
}

/// Longer line plus hint that still fits inside the code-box: diagnostic
/// anchors right after the rendered line (NOT at the box edge — the box
/// edge is the *limit*, not the anchor).
///
/// `let xs = vec![1];` (17) with `: Vec` hint at offset 7 → 22 cols
/// rendered. Still < text_width=27. Diag at col 28 + 22 + 2 = 52.
#[test]
fn centered_longer_line_with_hint_diag_after_rendered_line() {
    let mut test = centered("let xs = vec![1];\n", false);
    replace_hints(&mut test, vec![hint(7, ": Vec")]);
    replace_diags(&mut test, vec![diag(0, "warn")]);

    let rows = render_rows(&mut test);
    let row = &rows[0];

    let rendered = "let xs : Vec= vec![1];";
    let rendered_end = TEXT_AREA_X + rendered.len(); // 50
    let diag_start = rendered_end + DIAG_GAP; // 52

    assert_eq!(
        cols(row, TEXT_AREA_X, rendered.len()),
        rendered,
        "rendered line with hint should be intact; row was {row:?}"
    );
    assert_eq!(
        cols(row, rendered_end, DIAG_GAP),
        "  ",
        "expected 2-column gap after rendered line; row was {row:?}"
    );
    assert_eq!(
        cols(row, diag_start, 4),
        "warn",
        "diagnostic should anchor at rendered_end + gap (col {diag_start}); row was {row:?}"
    );
}

/// Line that fills the entire code-box: diagnostic lands at the box edge
/// (still right after the code, because the code happens to end there).
/// The diagnostic message is free to spill into the right margin.
#[test]
fn centered_full_width_line_diag_at_box_edge() {
    // 27-char line exactly fills text_width.
    let line: String = (0..27)
        .map(|i| char::from_digit(i % 10, 10).unwrap())
        .collect();
    let mut test = centered(&format!("{line}\n"), false);
    replace_diags(&mut test, vec![diag(0, "warn")]);

    let rows = render_rows(&mut test);
    let row = &rows[0];

    let diag_start = TEXT_AREA_RIGHT + DIAG_GAP; // 57

    assert_eq!(
        cols(row, TEXT_AREA_X, 27),
        line,
        "full-width line should render unclipped; row was {row:?}"
    );
    assert_eq!(
        cols(row, diag_start, 4),
        "warn",
        "diagnostic should anchor at the box edge + gap (col {diag_start}); row was {row:?}"
    );
}

/// Multi-row wrapped line: diagnostic attaches to the LAST visual row,
/// anchored after the last row's rendered text (NOT at the box edge —
/// the last row is short).
///
/// A 60-char line in text_width=27 wraps to 27 + 27 + 6 chars. Last row
/// is 6 cols; diag at col 28 + 6 + 2 = 36.
#[test]
fn centered_wrap_diag_attaches_to_last_visual_row() {
    let line: String = (0..60)
        .map(|i| char::from_digit((i % 10) as u32, 10).unwrap())
        .collect();
    let mut test = centered(&format!("{line}\n"), true);
    replace_diags(&mut test, vec![diag(0, "unused")]);

    let rows = render_rows(&mut test);

    // Earlier wrapped rows: no diagnostic anywhere in or past the code-box.
    for (i, row) in rows.iter().take(2).enumerate() {
        let tail = cols(row, TEXT_AREA_X, 80 - TEXT_AREA_X);
        assert!(
            !tail.contains("unused"),
            "wrapped row {i} should not carry the diagnostic; saw {tail:?}"
        );
    }

    // Last wrapped row: 6 chars of code (chars 54..60 → "456789"), gap + diag.
    let last = &rows[2];
    let diag_start = TEXT_AREA_X + 6 + DIAG_GAP; // 36
    assert_eq!(
        cols(last, TEXT_AREA_X, 6),
        "456789",
        "last wrapped row should render its 6 chars; row was {last:?}"
    );
    assert_eq!(
        cols(last, diag_start, 6),
        "unused",
        "diagnostic on last row should anchor at code_end + gap (col {diag_start}); row was {last:?}"
    );
}

/// Wrap mode counterpart of the short-line-with-hint case. With wrap on,
/// the line + hint still fits on a single visual row (15 cols < text_width=27),
/// so the diagnostic should land on row 0 right after the rendered line.
#[test]
fn centered_wrap_short_line_with_hint_diag_after_rendered_line() {
    let mut test = centered("let x = 1;\n", true);
    replace_hints(&mut test, vec![hint(5, ": i32")]);
    replace_diags(&mut test, vec![diag(0, "unused")]);

    let rows = render_rows(&mut test);
    let row = &rows[0];

    let diag_start = TEXT_AREA_X + 15 + DIAG_GAP; // 45

    assert_eq!(
        cols(row, TEXT_AREA_X, 15),
        "let x: i32 = 1;",
        "wrap-mode rendered line should be intact; row was {row:?}"
    );
    assert_eq!(
        cols(row, diag_start, 6),
        "unused",
        "wrap-mode diagnostic should anchor at rendered_end + gap (col {diag_start}); row was {row:?}"
    );
}

/// Sanity: the diagnostic is allowed to extend past the code-box right
/// edge into the right margin (up to render_width). Verifies a long
/// diagnostic on a short line spans across the box edge.
#[test]
fn centered_diag_extends_into_right_margin() {
    let mut test = centered("x = 1\n", false);
    replace_diags(&mut test, vec![diag(0, "this message is rather long")]);

    let rows = render_rows(&mut test);
    let row = &rows[0];

    let code_end = TEXT_AREA_X + 5; // 33
    let diag_start = code_end + DIAG_GAP; // 35

    // Diagnostic begins inside the box (col 35 < TEXT_AREA_RIGHT=55)...
    assert_eq!(
        cols(row, diag_start, 4),
        "this",
        "diagnostic should start right after the code; row was {row:?}"
    );
    // ...and continues across the box edge into the right margin.
    let past_edge = cols(row, TEXT_AREA_RIGHT, 4);
    assert!(
        past_edge.chars().any(|c| !c.is_whitespace()),
        "expected the long diagnostic to continue past the box edge into the margin; saw {past_edge:?}"
    );
}
