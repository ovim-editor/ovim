//! Phase-05 Step-D dual validation: `project_offset` must agree with the
//! decoration accumulator (`DecorationMap::adjust_for_edits`) across every
//! interactive editing scenario we care about — type-before/after, delete,
//! delete-over, rapid typing, undo, redo.
//!
//! The accumulator mutates decoration char_offsets in place; the projection
//! is a pure function that replays the edit log onto the original source
//! offset. Both systems must arrive at the same post-edit offset (or both
//! drop the decoration, when a delete engulfs the anchor).
//!
//! If this test ever fails, the accumulator and the projection disagree, and
//! we cannot safely cut over to projection in Step E without understanding
//! why.

mod helpers;

use helpers::EditorTest;
use ovim_core::editor::decoration::{
    project_offset, Decoration, DecorationPlacement, DecorationSource, DecorationStyle,
};

/// Track the original anchor so we can project it forward through the edit
/// log. The accumulator rewrites the stored char_offset in place; we keep the
/// source-version values aside so the parity check has something to project
/// *from*.
#[derive(Debug, Clone, Copy)]
struct AnchoredDecoration {
    /// The stable identity of the decoration (text + source) — we match by
    /// searching the DecorationMap for a decoration with this text, since
    /// char_offset mutates under adjustment.
    text_marker: &'static str,
    /// Source-version char offset at placement time.
    source_offset: usize,
    /// Buffer version the decoration was placed against.
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

/// Look up the current stored char_offset of the decoration whose text matches
/// `marker`. Returns None if the decoration has been dropped (e.g. a delete
/// engulfed its anchor).
fn find_stored_offset(test: &EditorTest, marker: &str) -> Option<usize> {
    test.editor
        .decorations
        .iter_all()
        .find(|(_, d)| d.text == marker)
        .map(|(_, d)| d.placement.char_offset())
}

/// Assert projection parity for every anchored decoration still in the map,
/// and also assert that decorations whose anchors were engulfed by a delete
/// are absent from both.
fn assert_parity(test: &EditorTest, anchors: &[AnchoredDecoration]) {
    for anchor in anchors {
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
        let stored = find_stored_offset(test, anchor.text_marker);

        match (projected, stored) {
            (Some(proj), Some(stored)) => assert_eq!(
                proj, stored,
                "projection ({}) disagrees with accumulator ({}) for decoration {:?}",
                proj, stored, anchor.text_marker
            ),
            (None, None) => {
                // Both agree: the anchor was engulfed. Good.
            }
            (Some(proj), None) => panic!(
                "accumulator dropped decoration {:?} but projection survived with offset {}",
                anchor.text_marker, proj
            ),
            (None, Some(stored)) => panic!(
                "projection dropped decoration {:?} but accumulator kept it at {}",
                anchor.text_marker, stored
            ),
        }
    }

    // The debug-only validation helper should also report zero mismatches
    // against its own (stored-offset, current-source-version) view.
    #[cfg(debug_assertions)]
    assert_eq!(
        test.editor.validate_decoration_projection(),
        0,
        "validate_decoration_projection reported mismatches"
    );
}

/// Place an inlay decoration at `(line, col)` in the current buffer, recording
/// the source version and the absolute source-version offset so the test can
/// project it forward later.
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
fn parity_insert_before_anchor() {
    let mut test = EditorTest::new("let x = 1;\n");
    let anchor = place_inlay(&mut test, 0, 5, ": i32");

    // Enter insert mode at col 0 and type 3 chars before the anchor.
    test.keys("gg").press('i').type_text("foo").press_esc();

    assert_parity(&test, &[anchor]);
}

#[test]
fn parity_insert_after_anchor() {
    let mut test = EditorTest::new("let x = 1;\n");
    let anchor = place_inlay(&mut test, 0, 5, ": i32");

    // Append at end of line (after the anchor), type 3 chars.
    test.press('A').type_text("bar").press_esc();

    assert_parity(&test, &[anchor]);
}

#[test]
fn parity_delete_before_anchor() {
    let mut test = EditorTest::new("foobar = 1;\n");
    // Anchor at char 8 (the '1' position).
    let anchor = place_inlay(&mut test, 0, 8, ": i32");

    // Delete 3 chars at start of line.
    test.keys("gg").press('3').press('x');

    assert_parity(&test, &[anchor]);
}

#[test]
fn parity_delete_engulfs_anchor() {
    let mut test = EditorTest::new("let x = 1;\n");
    // Anchor at char 5 (on the 'x').
    let anchor = place_inlay(&mut test, 0, 5, ": i32");

    // Delete "x = 1" (5 chars from col 4).
    test.keys("gg").keys("llll").press('5').press('x');

    // Both systems should drop the decoration.
    assert!(
        find_stored_offset(&test, ": i32").is_none(),
        "accumulator should drop engulfed decoration"
    );
    assert_parity(&test, &[anchor]);
}

#[test]
fn parity_rapid_typing() {
    let mut test = EditorTest::new("let x = 1;\n");
    let anchor = place_inlay(&mut test, 0, 5, ": i32");

    // Enter insert mode at start and type 12 chars with interleaved edits.
    test.keys("gg")
        .press('i')
        .type_text("aaaaaaaaaaaa")
        .press_esc();

    assert_parity(&test, &[anchor]);
}

#[test]
fn parity_multiple_decorations_mixed_edits() {
    let mut test = EditorTest::new("hello world\nlet x = 1;\nend\n");
    // Place a decoration on each of three lines.
    let a = place_inlay(&mut test, 0, 6, ": world"); // at col 6 of line 0 (pos 6)
    let b = place_inlay(&mut test, 1, 5, ": i32"); // at col 5 of line 1
    let c = place_eol(&mut test, 2, "warn: unused");

    // Make a variety of edits.
    test.keys("gg").press('i').type_text("AA").press_esc();
    test.keys("j").press('A').type_text(" // trail").press_esc();

    assert_parity(&test, &[a, b, c]);
}

#[test]
fn parity_survives_undo() {
    let mut test = EditorTest::new("let x = 1;\n");
    let anchor = place_inlay(&mut test, 0, 5, ": i32");

    // Insert, then undo — the accumulator replays the inverse edit through
    // adjust_for_edits. The edit log records both the forward and inverse
    // groups, so project_offset(source_offset, edits_since(source_version))
    // composes them and lands on the original offset.
    test.keys("gg").press('i').type_text("foo").press_esc();
    test.press('u');

    assert_parity(&test, &[anchor]);
}

#[test]
fn parity_survives_undo_redo() {
    let mut test = EditorTest::new("let x = 1;\n");
    let anchor = place_inlay(&mut test, 0, 5, ": i32");

    test.keys("gg").press('i').type_text("foo").press_esc();
    test.press('u');
    test.press_with(ovim_core::KeyCode::Char('r'), ovim_core::Modifiers::CONTROL);

    assert_parity(&test, &[anchor]);
}

#[test]
fn parity_insert_then_delete_same_region() {
    let mut test = EditorTest::new("let x = 1;\n");
    let anchor = place_inlay(&mut test, 0, 5, ": i32");

    // Insert, then delete in place. The net edit log contains both groups.
    test.keys("gg").press('i').type_text("foo").press_esc();
    test.keys("gg").press('3').press('x');

    assert_parity(&test, &[anchor]);
}

#[test]
fn parity_newline_insert_moves_line() {
    let mut test = EditorTest::new("let x = 1;\n");
    let anchor = place_inlay(&mut test, 0, 5, ": i32");

    // Insert a newline before the 'x' — decoration moves to line 1.
    test.keys("gg")
        .keys("llll")
        .press('i')
        .press_enter()
        .press_esc();

    assert_parity(&test, &[anchor]);
}
