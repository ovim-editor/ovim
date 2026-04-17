//! Phase-05 Step E: the renderer reads projected decoration offsets rather
//! than stored ones. The accumulator still runs in parallel as a safety net,
//! so the two paths must agree. These tests exercise the projected accessors
//! directly (the paths the renderer routes through) and verify they return
//! correct offsets across interactive editing scenarios.
//!
//! In contrast to `decoration_projection_test.rs` — which checks the pure
//! `project_offset` function against the accumulator — this file checks the
//! `*_projected` methods on `DecorationMap`, because those are what the
//! renderer actually calls. Any divergence between "renderer sees projected
//! offset" and "accumulator stores projected offset" would be caught here.

mod helpers;

use helpers::EditorTest;
use ovim_core::editor::decoration::{
    Decoration, DecorationPlacement, DecorationSource, DecorationStyle,
};

fn inlay_decoration(char_offset: usize, text: &'static str, source_version: u64) -> Decoration {
    Decoration {
        placement: DecorationPlacement::Inline { char_offset },
        source: DecorationSource::InlayHint,
        text: text.to_string(),
        display_width: text.chars().count(),
        style: DecorationStyle::new(ovim_core::color::Color::Gray),
        priority: 0,
        source_version,
    }
}

fn eol_decoration(char_offset: usize, text: &'static str, source_version: u64) -> Decoration {
    Decoration {
        placement: DecorationPlacement::EndOfLine { char_offset },
        source: DecorationSource::Diagnostic,
        text: text.to_string(),
        display_width: text.chars().count(),
        style: DecorationStyle::new(ovim_core::color::Color::Red),
        priority: 0,
        source_version,
    }
}

/// Place an inlay decoration anchored to the **current** buffer version,
/// matching Resolution A (what `lsp_integration.rs` does after Step E).
fn place_inlay_at_current_version(
    test: &mut EditorTest,
    line: usize,
    char_idx_in_line: usize,
    text: &'static str,
) {
    let rope = test.editor.buffer().rope().clone();
    let source_offset = rope.line_to_char(line) + char_idx_in_line;
    let source_version = test.editor.buffer().version() as u64;
    let existing: Vec<Decoration> = test
        .editor
        .decorations
        .iter_all()
        .filter(|(_, d)| d.source == DecorationSource::InlayHint)
        .map(|(_, d)| d.clone())
        .collect();
    let mut new_set = existing;
    new_set.push(inlay_decoration(source_offset, text, source_version));
    test.editor
        .decorations
        .replace_source(DecorationSource::InlayHint, new_set, &rope);
}

fn place_eol_at_current_version(test: &mut EditorTest, line: usize, text: &'static str) {
    let rope = test.editor.buffer().rope().clone();
    let source_offset = rope.line_to_char(line);
    let source_version = test.editor.buffer().version() as u64;
    let existing: Vec<Decoration> = test
        .editor
        .decorations
        .iter_all()
        .filter(|(_, d)| d.source == DecorationSource::Diagnostic)
        .map(|(_, d)| d.clone())
        .collect();
    let mut new_set = existing;
    new_set.push(eol_decoration(source_offset, text, source_version));
    test.editor
        .decorations
        .replace_source(DecorationSource::Diagnostic, new_set, &rope);
}

fn projected_inline_for_line(test: &EditorTest, line: usize) -> Vec<(usize, usize)> {
    test.editor
        .decorations
        .inline_decorations_for_line_projected(
            line,
            test.editor.buffer().rope(),
            test.editor.buffer().edit_log(),
        )
}

fn projected_width_before(test: &EditorTest, line: usize, char_idx: usize) -> usize {
    test.editor.decorations.inline_width_before_projected(
        line,
        char_idx,
        test.editor.buffer().rope(),
        test.editor.buffer().edit_log(),
    )
}

fn projected_for_line(test: &EditorTest, line: usize) -> Vec<Decoration> {
    test.editor.decorations.for_line_projected(
        line,
        test.editor.buffer().rope(),
        test.editor.buffer().edit_log(),
    )
}

#[test]
fn step_e_projected_offset_is_stored_offset_at_placement() {
    // Resolution A anchors decorations at the *current* version, so
    // `edits_since(source_version)` is empty at placement and the projected
    // offset equals the stored one.
    let mut test = EditorTest::new("let x = 1;\n");
    place_inlay_at_current_version(&mut test, 0, 5, ": i32");

    let pairs = projected_inline_for_line(&test, 0);
    assert_eq!(pairs.len(), 1, "one inlay decoration projected");
    assert_eq!(pairs[0], (5, 5), "char_idx=5, display_width=5");
}

#[test]
fn step_e_projection_shifts_with_insert_before_anchor() {
    let mut test = EditorTest::new("let x = 1;\n");
    place_inlay_at_current_version(&mut test, 0, 5, ": i32");

    // Type 3 chars before the anchor.
    test.keys("gg").press('i').type_text("foo").press_esc();

    // The renderer now asks: "what are the inline widths on line 0?"
    let pairs = projected_inline_for_line(&test, 0);
    assert_eq!(pairs.len(), 1);
    // Anchor shifted forward by 3 chars (5 → 8).
    assert_eq!(
        pairs[0].0, 8,
        "char_idx_in_line should follow the insert forward"
    );
}

#[test]
fn step_e_projection_survives_after_delete() {
    let mut test = EditorTest::new("foobar = 1;\n");
    // Anchor at char 8 (the '1' position).
    place_inlay_at_current_version(&mut test, 0, 8, ": i32");

    // Delete 3 chars at start of line.
    test.keys("gg").press('3').press('x');

    let pairs = projected_inline_for_line(&test, 0);
    assert_eq!(pairs.len(), 1, "decoration survives the delete");
    // Anchor shifted back from 8 to 5.
    assert_eq!(pairs[0].0, 5);
}

#[test]
fn step_e_projection_drops_when_anchor_engulfed() {
    let mut test = EditorTest::new("let x = 1;\n");
    place_inlay_at_current_version(&mut test, 0, 5, ": i32");

    // Delete "x = 1" (5 chars from col 4) — engulfs the anchor.
    test.keys("gg").keys("llll").press('5').press('x');

    let pairs = projected_inline_for_line(&test, 0);
    assert!(
        pairs.is_empty(),
        "projection drops the decoration once its anchor is engulfed"
    );
}

#[test]
fn step_e_projected_width_before_cursor_matches_expected_sum() {
    let mut test = EditorTest::new("let x = 1;\n");
    // Two inline hints on the same line.
    place_inlay_at_current_version(&mut test, 0, 3, ": i32"); // 5 cols at char 3
    place_inlay_at_current_version(&mut test, 0, 8, ": u8"); // 4 cols at char 8

    // Before both: 0.
    assert_eq!(projected_width_before(&test, 0, 2), 0);
    // At first: 5.
    assert_eq!(projected_width_before(&test, 0, 3), 5);
    // Between: 5.
    assert_eq!(projected_width_before(&test, 0, 7), 5);
    // At second: 5 + 4 = 9.
    assert_eq!(projected_width_before(&test, 0, 8), 9);

    // Type a char at col 0 — both anchors shift forward.
    test.keys("gg").press('i').press('X').press_esc();

    // Anchor 1: now at char 4. Anchor 2: now at char 9.
    assert_eq!(projected_width_before(&test, 0, 3), 0);
    assert_eq!(projected_width_before(&test, 0, 4), 5);
    assert_eq!(projected_width_before(&test, 0, 8), 5);
    assert_eq!(projected_width_before(&test, 0, 9), 9);
}

#[test]
fn step_e_projection_follows_anchor_across_lines_after_newline_insert() {
    let mut test = EditorTest::new("let x = 1;\n");
    place_inlay_at_current_version(&mut test, 0, 5, ": i32");

    // Insert a newline before the 'x' — anchor should cross to line 1.
    test.keys("gg")
        .keys("llll") // move cursor to col 4 (on 'x')
        .press('i')
        .press_enter()
        .press_esc();

    // Line 0 should no longer contain the decoration; line 1 should.
    let line0 = projected_inline_for_line(&test, 0);
    let line1 = projected_inline_for_line(&test, 1);
    assert!(line0.is_empty(), "decoration moved off line 0");
    assert_eq!(line1.len(), 1, "decoration moved to line 1");
}

#[test]
fn step_e_validate_projection_reports_zero_mismatches() {
    // Dual validation: the accumulator and projection must agree. This is
    // the invariant Step E depends on — if it ever breaks, we know to stop.
    let mut test = EditorTest::new("hello world\nlet x = 1;\nend\n");
    place_inlay_at_current_version(&mut test, 0, 6, ": world");
    place_inlay_at_current_version(&mut test, 1, 5, ": i32");
    place_eol_at_current_version(&mut test, 2, "warn: unused");

    // Mix inserts and deletes.
    test.keys("gg").press('i').type_text("AA").press_esc();
    test.keys("j").press('A').type_text(" // trail").press_esc();

    #[cfg(debug_assertions)]
    assert_eq!(
        test.editor.validate_decoration_projection(),
        0,
        "accumulator and projection must agree in steady state"
    );

    // And a sanity check: projected positions still round-trip.
    let line0 = projected_for_line(&test, 0);
    let line1 = projected_for_line(&test, 1);
    let line2 = projected_for_line(&test, 2);
    assert_eq!(line0.len(), 1);
    assert_eq!(line1.len(), 1);
    assert_eq!(line2.len(), 1);
}

#[test]
fn step_e_projection_survives_undo_redo() {
    let mut test = EditorTest::new("let x = 1;\n");
    place_inlay_at_current_version(&mut test, 0, 5, ": i32");

    // Insert then undo: anchor should return to original position.
    test.keys("gg").press('i').type_text("foo").press_esc();
    test.press('u');

    let pairs = projected_inline_for_line(&test, 0);
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].0, 5, "undo restores original anchor position");

    // Redo: anchor should move forward again.
    test.press_with(ovim_core::KeyCode::Char('r'), ovim_core::Modifiers::CONTROL);

    let pairs = projected_inline_for_line(&test, 0);
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].0, 8, "redo shifts anchor forward again");
}

#[test]
fn step_e_projection_tracks_rapid_typing() {
    let mut test = EditorTest::new("let x = 1;\n");
    place_inlay_at_current_version(&mut test, 0, 5, ": i32");

    // Type 12 chars before the anchor in one insert-mode session.
    test.keys("gg")
        .press('i')
        .type_text("aaaaaaaaaaaa")
        .press_esc();

    let pairs = projected_inline_for_line(&test, 0);
    assert_eq!(pairs.len(), 1);
    // Anchor shifts from 5 to 17.
    assert_eq!(pairs[0].0, 17);
}

#[test]
fn step_e_eol_projection_tracks_edits() {
    let mut test = EditorTest::new("let x = 1;\nlet y = 2;\n");
    place_eol_at_current_version(&mut test, 1, "warn: unused");

    // Insert a line at the top — EOL anchor for line 1 should now be line 2.
    test.keys("gg").press('O').type_text("// top").press_esc();

    let line1 = projected_for_line(&test, 1);
    let line2 = projected_for_line(&test, 2);
    assert!(
        line1
            .iter()
            .all(|d| !matches!(d.placement, DecorationPlacement::EndOfLine { .. })),
        "EOL decoration no longer on line 1 after the insert above"
    );
    assert_eq!(
        line2
            .iter()
            .filter(|d| matches!(d.placement, DecorationPlacement::EndOfLine { .. }))
            .count(),
        1,
        "EOL decoration is now on line 2"
    );
}
