//! Phase-05 Step F: projection is the sole source of truth for decoration
//! positions. This file exercises `project_offset` against real interactive
//! editing sequences — typing, deleting, undo, redo, newline splits — with
//! the edit log captured end-to-end, proving that the edit log + projection
//! yield correct offsets without any accumulator.
//!
//! Originally this file ran dual validation against the accumulator
//! (`DecorationMap::adjust_for_edits`). Step F removed the accumulator; what
//! remains are the scenario-level assertions that projection produces the
//! expected offsets directly.

mod helpers;

use helpers::EditorTest;
use ovim_core::editor::decoration::{
    project_offset, Decoration, DecorationPlacement, DecorationSource, DecorationStyle,
};

/// Tracks an original anchor so tests can project it forward through the
/// edit log. Only `source_offset` + `source_version` are needed: projection
/// is pure, so we don't need to look the decoration up after the fact.
#[derive(Debug, Clone, Copy)]
struct AnchoredDecoration {
    text_marker: &'static str,
    source_offset: usize,
    source_version: u64,
}

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

/// Look up a decoration's stored (source-version) offset by its text marker.
/// With the accumulator gone, this offset is frozen at placement time; we
/// return it so tests can still reason about decorations by identity.
fn find_stored_offset(test: &EditorTest, marker: &str) -> Option<usize> {
    test.editor
        .decorations
        .iter_all()
        .find(|(_, d)| d.text == marker)
        .map(|(_, d)| d.placement.char_offset())
}

/// Project each anchored decoration through the edit log and assert that
/// projection lands on a sensible offset (or `None` if its anchor was
/// engulfed by a delete). Also compares against `find_stored_offset`
/// to confirm the decoration is either still present with its frozen
/// source offset (projection will survive) or structurally unaffected
/// (deletes don't evict — decorations are immutable post-placement).
fn assert_projection_matches_expected(
    test: &EditorTest,
    anchors: &[(AnchoredDecoration, Option<usize>)],
) {
    for (anchor, expected) in anchors {
        let edits = test
            .editor
            .buffer()
            .edit_log()
            .edits_since(anchor.source_version)
            .unwrap_or_else(|| {
                panic!(
                    "edit log evicted source_version={} (test went past ring capacity)",
                    anchor.source_version
                )
            });
        let projected = project_offset(anchor.source_offset, &edits);
        assert_eq!(
            projected, *expected,
            "projection for {:?}: expected {:?}, got {:?}",
            anchor.text_marker, expected, projected
        );

        // The decoration itself is never evicted by projection — its stored
        // offset is still the source-version offset. Confirm it's in the map.
        let stored = find_stored_offset(test, anchor.text_marker);
        assert_eq!(
            stored,
            Some(anchor.source_offset),
            "decoration {:?} should still hold its source-version offset (immutable post Step F)",
            anchor.text_marker
        );
    }
}

fn place_inlay(
    test: &mut EditorTest,
    line: usize,
    char_idx_in_line: usize,
    text: &'static str,
) -> AnchoredDecoration {
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

    AnchoredDecoration {
        text_marker: text,
        source_offset,
        source_version,
    }
}

fn place_eol(test: &mut EditorTest, line: usize, text: &'static str) -> AnchoredDecoration {
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

    AnchoredDecoration {
        text_marker: text,
        source_offset,
        source_version,
    }
}

// ---------------------------------------------------------------------------
// Scenario tests
// ---------------------------------------------------------------------------

#[test]
fn projection_insert_before_anchor_shifts_forward() {
    let mut test = EditorTest::new("let x = 1;\n");
    let anchor = place_inlay(&mut test, 0, 5, ": i32");

    test.keys("gg").press('i').type_text("foo").press_esc();

    // 3 chars inserted before anchor → projects from 5 to 8.
    assert_projection_matches_expected(&test, &[(anchor, Some(8))]);
}

#[test]
fn projection_insert_after_anchor_is_unchanged() {
    let mut test = EditorTest::new("let x = 1;\n");
    let anchor = place_inlay(&mut test, 0, 5, ": i32");

    // Append at end of line (after the anchor).
    test.press('A').type_text("bar").press_esc();

    assert_projection_matches_expected(&test, &[(anchor, Some(5))]);
}

#[test]
fn projection_delete_before_anchor_shifts_back() {
    let mut test = EditorTest::new("foobar = 1;\n");
    let anchor = place_inlay(&mut test, 0, 8, ": i32");

    // Delete 3 chars at start of line.
    test.keys("gg").press('3').press('x');

    assert_projection_matches_expected(&test, &[(anchor, Some(5))]);
}

#[test]
fn projection_delete_engulfs_anchor_drops_to_none() {
    let mut test = EditorTest::new("let x = 1;\n");
    let anchor = place_inlay(&mut test, 0, 5, ": i32");

    // Delete "x = 1" (5 chars from col 4).
    test.keys("gg").keys("llll").press('5').press('x');

    assert_projection_matches_expected(&test, &[(anchor, None)]);
}

#[test]
fn projection_rapid_typing_accumulates_shift() {
    let mut test = EditorTest::new("let x = 1;\n");
    let anchor = place_inlay(&mut test, 0, 5, ": i32");

    test.keys("gg")
        .press('i')
        .type_text("aaaaaaaaaaaa")
        .press_esc();

    // 12 chars before anchor → 5 + 12 = 17.
    assert_projection_matches_expected(&test, &[(anchor, Some(17))]);
}

#[test]
fn projection_multiple_decorations_mixed_edits() {
    let mut test = EditorTest::new("hello world\nlet x = 1;\nend\n");
    // Anchor on line 0 at col 6 → source_offset = 6.
    let a = place_inlay(&mut test, 0, 6, ": world");
    // Anchor on line 1 at col 5 → source_offset = 12 + 5 = 17.
    let b = place_inlay(&mut test, 1, 5, ": i32");
    // EOL on line 2 → source_offset = 23 (line-2 start).
    let c = place_eol(&mut test, 2, "warn: unused");

    // "AA" at line 0 start → shifts every anchor forward by 2.
    test.keys("gg").press('i').type_text("AA").press_esc();
    // Append " // trail" to line 1 → doesn't affect anchors a, b (both <= insertion point),
    // doesn't affect c (anchors to line-2 start, which is past line 1).
    test.keys("j").press('A').type_text(" // trail").press_esc();

    assert_projection_matches_expected(
        &test,
        &[
            (a, Some(8)),                                    // 6 + 2
            (b, Some(19)),                                   // 17 + 2
            (c, Some(23 + 2 + " // trail".chars().count())), /* line-2 start shifts by 2 (from "AA") + 9 (from " // trail") */
        ],
    );
}

#[test]
fn projection_survives_undo() {
    let mut test = EditorTest::new("let x = 1;\n");
    let anchor = place_inlay(&mut test, 0, 5, ": i32");

    // Insert then undo — net zero, projection returns to original offset.
    test.keys("gg").press('i').type_text("foo").press_esc();
    test.press('u');

    assert_projection_matches_expected(&test, &[(anchor, Some(5))]);
}

#[test]
fn projection_survives_undo_redo() {
    let mut test = EditorTest::new("let x = 1;\n");
    let anchor = place_inlay(&mut test, 0, 5, ": i32");

    test.keys("gg").press('i').type_text("foo").press_esc();
    test.press('u');
    test.press_with(ovim_core::KeyCode::Char('r'), ovim_core::Modifiers::CONTROL);

    // Redo reapplies +3 → projection to 8 again.
    assert_projection_matches_expected(&test, &[(anchor, Some(8))]);
}

#[test]
fn projection_insert_then_delete_same_region() {
    let mut test = EditorTest::new("let x = 1;\n");
    let anchor = place_inlay(&mut test, 0, 5, ": i32");

    // Insert "foo" at 0, then delete "foo" (3 chars) at 0 — net zero.
    test.keys("gg").press('i').type_text("foo").press_esc();
    test.keys("gg").press('3').press('x');

    assert_projection_matches_expected(&test, &[(anchor, Some(5))]);
}

#[test]
fn projection_newline_insert_moves_line_index() {
    let mut test = EditorTest::new("let x = 1;\n");
    let anchor = place_inlay(&mut test, 0, 5, ": i32");

    // Insert a newline at col 4 — anchor shifts by +1, crosses into line 1.
    test.keys("gg")
        .keys("llll")
        .press('i')
        .press_enter()
        .press_esc();

    assert_projection_matches_expected(&test, &[(anchor, Some(6))]);

    // Confirm the renderer's line lookup agrees.
    let line1 = test.editor.decorations.for_line_projected(
        1,
        test.editor.buffer().rope(),
        test.editor.buffer().edit_log(),
    );
    assert_eq!(line1.len(), 1, "decoration projects onto line 1");
}
