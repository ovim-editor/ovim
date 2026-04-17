//! Phase-05 Step A: `/v1/snapshot` exposes the current decoration set.
//!
//! This test pokes decorations directly into `editor.decorations` (mirroring
//! what `decoration_drift_test.rs` does), round-trips through the snapshot
//! JSON, and verifies that the projected `DecorationInfo` list contains the
//! expected line/col/text/source/placement fields.
//!
//! We don't exercise the full HTTP stack here — that's integration territory
//! for the session-spawn smoke check. This test locks in the shape of the
//! projection so Step C (adding `source_version`) has a safety net.
mod helpers;

use helpers::EditorTest;
use ovim_core::color::Color;
use ovim_core::editor::decoration::{
    Decoration, DecorationPlacement, DecorationSource, DecorationStyle,
};

fn inlay(char_offset: usize, text: &str) -> Decoration {
    Decoration {
        placement: DecorationPlacement::Inline { char_offset },
        source: DecorationSource::InlayHint,
        text: text.to_string(),
        display_width: text.chars().count(),
        style: DecorationStyle::new(Color::Gray).with_italic(),
        priority: 10,
        source_version: 0,
    }
}

fn inlay_with_version(char_offset: usize, text: &str, source_version: u64) -> Decoration {
    Decoration {
        placement: DecorationPlacement::Inline { char_offset },
        source: DecorationSource::InlayHint,
        text: text.to_string(),
        display_width: text.chars().count(),
        style: DecorationStyle::new(Color::Gray).with_italic(),
        priority: 10,
        source_version,
    }
}

fn diagnostic_eol(line_start_offset: usize, text: &str) -> Decoration {
    Decoration {
        placement: DecorationPlacement::EndOfLine {
            char_offset: line_start_offset,
        },
        source: DecorationSource::Diagnostic,
        text: text.to_string(),
        display_width: text.chars().count(),
        style: DecorationStyle::new(Color::Red),
        priority: 0,
        source_version: 0,
    }
}

/// Build a snapshot in the same shape that `create_snapshot` produces, then
/// round-trip it through JSON. This mirrors what the HTTP endpoint does and
/// catches any breakage in the serialize/deserialize contract.
///
/// Step F: positions are projected through the edit log, matching the live
/// behaviour of `create_snapshot` in `event_loop.rs`.
fn project_snapshot_decorations(test: &EditorTest) -> serde_json::Value {
    use ovim::api::DecorationInfo;
    use ovim_core::editor::decoration::project_offset;

    let rope = test.editor.buffer().rope();
    let edit_log = test.editor.buffer().edit_log();
    let decs: Vec<DecorationInfo> = test
        .editor
        .decorations
        .iter_all()
        .filter_map(|(stored_line, dec)| {
            let stored_offset = dec.placement.char_offset();
            let projected_offset = match edit_log.edits_since(dec.source_version) {
                Some(edits) => match project_offset(stored_offset, &edits) {
                    Some(off) => off,
                    None => return None,
                },
                None => stored_offset,
            };
            let clamped = projected_offset.min(rope.len_chars());
            let live_line = rope.char_to_line(clamped);
            let line_start = rope.line_to_char(live_line);
            let col = clamped - line_start;
            let line = if projected_offset > rope.len_chars() {
                stored_line
            } else {
                live_line
            };
            let source = match dec.source {
                DecorationSource::InlayHint => "inlay_hint",
                DecorationSource::Diagnostic => "diagnostic",
            }
            .to_string();
            let placement = match dec.placement {
                DecorationPlacement::Inline { .. } => "inline",
                DecorationPlacement::EndOfLine { .. } => "eol",
            }
            .to_string();
            Some(DecorationInfo {
                line,
                char_offset: clamped,
                col,
                text: dec.text.clone(),
                source,
                placement,
                source_version: dec.source_version,
            })
        })
        .collect();

    serde_json::to_value(&decs).unwrap()
}

#[test]
fn snapshot_decorations_empty_by_default() {
    let test = EditorTest::new("let x = 1;\nlet y = 2;\n");
    let decs = project_snapshot_decorations(&test);
    assert!(decs.is_array(), "decorations must be a JSON array");
    assert_eq!(
        decs.as_array().unwrap().len(),
        0,
        "no LSP → no decorations in the snapshot"
    );
}

#[test]
fn snapshot_projects_inlay_hints_with_line_col_and_source() {
    let mut test = EditorTest::new("let x = 1;\nlet y = 2;\n");
    let rope = test.editor.buffer().rope().clone();
    // char 5 on line 0 = offset 5 (just after "let x"); char 16 on line 1 = offset 16
    test.editor.decorations.replace_source(
        DecorationSource::InlayHint,
        vec![inlay(5, ": i32"), inlay(16, ": u64")],
        &rope,
    );

    let decs = project_snapshot_decorations(&test);
    let arr = decs.as_array().unwrap();
    assert_eq!(arr.len(), 2, "both inlay hints should surface");

    // iter_all() walks lines in order, so line 0 comes first.
    assert_eq!(arr[0]["line"], 0);
    assert_eq!(arr[0]["col"], 5);
    assert_eq!(arr[0]["char_offset"], 5);
    assert_eq!(arr[0]["text"], ": i32");
    assert_eq!(arr[0]["source"], "inlay_hint");
    assert_eq!(arr[0]["placement"], "inline");
    // Step C: source_version is always present, defaulting to 0 for
    // test-synthesised decorations that didn't specify one.
    assert_eq!(
        arr[0]["source_version"], 0,
        "source_version is always present post-Step-C (0 for test decorations)"
    );

    assert_eq!(arr[1]["line"], 1);
    assert_eq!(arr[1]["col"], 5); // 16 - 11 (line 1 starts at offset 11)
    assert_eq!(arr[1]["char_offset"], 16);
    assert_eq!(arr[1]["text"], ": u64");
}

#[test]
fn snapshot_carries_source_version_through_to_json() {
    let mut test = EditorTest::new("let x = 1;\nlet y = 2;\n");
    let rope = test.editor.buffer().rope().clone();
    test.editor.decorations.replace_source(
        DecorationSource::InlayHint,
        vec![inlay_with_version(5, ": i32", 42)],
        &rope,
    );

    let decs = project_snapshot_decorations(&test);
    let arr = decs.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(
        arr[0]["source_version"], 42,
        "decoration's source_version should appear verbatim in the snapshot JSON"
    );
}

#[test]
fn snapshot_projects_diagnostics_as_eol_placement() {
    let mut test = EditorTest::new("let x = 1;\nlet y = 2;\n");
    let rope = test.editor.buffer().rope().clone();
    // Line 1 starts at offset 11 — this is what the diagnostic anchors to.
    test.editor.decorations.replace_source(
        DecorationSource::Diagnostic,
        vec![diagnostic_eol(11, " unused variable")],
        &rope,
    );

    let decs = project_snapshot_decorations(&test);
    let arr = decs.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["line"], 1);
    assert_eq!(arr[0]["source"], "diagnostic");
    assert_eq!(arr[0]["placement"], "eol");
    assert_eq!(arr[0]["text"], " unused variable");
}

#[test]
fn snapshot_decorations_include_both_sources_in_line_order() {
    let mut test = EditorTest::new("let x = 1;\nlet y = 2;\n");
    let rope = test.editor.buffer().rope().clone();
    test.editor.decorations.replace_source(
        DecorationSource::InlayHint,
        vec![inlay(5, ": i32")],
        &rope,
    );
    test.editor.decorations.replace_source(
        DecorationSource::Diagnostic,
        vec![diagnostic_eol(11, " error on line 1")],
        &rope,
    );

    let decs = project_snapshot_decorations(&test);
    let arr = decs.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    // Line 0 (inlay_hint) before line 1 (diagnostic).
    assert_eq!(arr[0]["source"], "inlay_hint");
    assert_eq!(arr[0]["line"], 0);
    assert_eq!(arr[1]["source"], "diagnostic");
    assert_eq!(arr[1]["line"], 1);
}
