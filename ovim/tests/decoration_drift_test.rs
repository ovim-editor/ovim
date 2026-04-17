/// Regression test: decorations (inlay hints, diagnostic EOL markers) must
/// shift as text is typed in insert mode, not drift left by one per keystroke.
///
/// Before Phase-05, `apply_change_and_record` mutated the rope without calling
/// `adjust_for_edits`, so between individual keystrokes the stored char offsets
/// pointed to stale positions. Only on insert-mode exit (when
/// `push_recorded_undo` ran) did decorations catch up.
///
/// Step F removed the accumulator entirely — positions are projected on
/// demand from the edit log. These tests now assert that the projected
/// offsets follow the edits, which is what the renderer actually consumes.
mod helpers;
use helpers::EditorTest;
use ovim_core::editor::decoration::{
    Decoration, DecorationPlacement, DecorationSource, DecorationStyle,
};

fn inlay_at(offset: usize, source_version: u64) -> Decoration {
    Decoration {
        placement: DecorationPlacement::Inline {
            char_offset: offset,
        },
        source: DecorationSource::InlayHint,
        text: ": i32".to_string(),
        display_width: 5,
        style: DecorationStyle::new(ovim_core::color::Color::Gray),
        priority: 0,
        source_version,
    }
}

/// Projected inline-decoration pairs on a line, the way the renderer reads them.
fn projected_inline(test: &EditorTest, line: usize) -> Vec<(usize, usize)> {
    test.editor
        .decorations
        .inline_decorations_for_line_projected(
            line,
            test.editor.buffer().rope(),
            test.editor.buffer().edit_log(),
        )
}

#[test]
fn inlay_hint_follows_insert_in_insert_mode() {
    let mut test = EditorTest::new("let x = 1;\n");
    let rope = test.editor.buffer().rope().clone();
    let source_version = test.editor.buffer().version() as u64;
    test.editor.decorations.replace_source(
        DecorationSource::InlayHint,
        vec![inlay_at(5, source_version)],
        &rope,
    );
    assert_eq!(projected_inline(&test, 0), vec![(5, 5)]);

    test.press('i').press('X');

    // Typing 1 char at col 0 shifts the anchor by 1: char_idx_in_line = 6.
    let pairs = projected_inline(&test, 0);
    assert_eq!(pairs.len(), 1);
    assert_eq!(
        pairs[0].0, 6,
        "inlay hint should shift right by 1 after typing one char before it"
    );
}

#[test]
fn inlay_hint_follows_multiple_inserts() {
    let mut test = EditorTest::new("let x = 1;\n");
    let rope = test.editor.buffer().rope().clone();
    let source_version = test.editor.buffer().version() as u64;
    test.editor.decorations.replace_source(
        DecorationSource::InlayHint,
        vec![inlay_at(5, source_version)],
        &rope,
    );

    test.press('i');
    for c in "abcdef".chars() {
        test.press(c);
    }

    let pairs = projected_inline(&test, 0);
    assert_eq!(pairs.len(), 1);
    assert_eq!(
        pairs[0].0, 11,
        "inlay hint should shift right by 6 after typing 6 chars before it"
    );
}

#[test]
fn inlay_hint_follows_backspace() {
    let mut test = EditorTest::new("abclet x = 1;\n");
    let rope = test.editor.buffer().rope().clone();
    let source_version = test.editor.buffer().version() as u64;
    test.editor.decorations.replace_source(
        DecorationSource::InlayHint,
        vec![inlay_at(8, source_version)],
        &rope,
    );

    test.press('g').press('g').press('l').press('l').press('l');
    test.press('i').press_backspace();

    let pairs = projected_inline(&test, 0);
    assert_eq!(pairs.len(), 1);
    assert_eq!(
        pairs[0].0, 7,
        "inlay hint should shift left by 1 after deleting one char before it"
    );
}
