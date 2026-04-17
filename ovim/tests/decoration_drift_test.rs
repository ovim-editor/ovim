/// Regression test: decorations (inlay hints, diagnostic EOL markers) must
/// shift as text is typed in insert mode, not drift left by one per keystroke.
///
/// Before the fix, `apply_change_and_record` mutated the rope without calling
/// `adjust_for_edits`, so between individual keystrokes the stored char offsets
/// pointed to stale positions. Only on insert-mode exit (when
/// `push_recorded_undo` ran) did decorations catch up.
mod helpers;
use helpers::EditorTest;
use ovim_core::editor::decoration::{
    Decoration, DecorationPlacement, DecorationSource, DecorationStyle,
};

fn inlay_at(offset: usize) -> Decoration {
    Decoration {
        placement: DecorationPlacement::Inline {
            char_offset: offset,
        },
        source: DecorationSource::InlayHint,
        text: ": i32".to_string(),
        display_width: 5,
        style: DecorationStyle::new(ovim_core::color::Color::Gray),
        priority: 0,
        source_version: 0,
    }
}

#[test]
fn inlay_hint_follows_insert_in_insert_mode() {
    let mut test = EditorTest::new("let x = 1;\n");
    let rope = test.editor.buffer().rope().clone();
    test.editor
        .decorations
        .replace_source(DecorationSource::InlayHint, vec![inlay_at(5)], &rope);
    assert_eq!(
        test.editor.decorations.for_line(0)[0]
            .placement
            .char_offset(),
        5
    );

    test.press('i').press('X');

    assert_eq!(
        test.editor.decorations.for_line(0)[0]
            .placement
            .char_offset(),
        6,
        "inlay hint should shift right by 1 after typing one char before it"
    );
}

#[test]
fn inlay_hint_follows_multiple_inserts() {
    let mut test = EditorTest::new("let x = 1;\n");
    let rope = test.editor.buffer().rope().clone();
    test.editor
        .decorations
        .replace_source(DecorationSource::InlayHint, vec![inlay_at(5)], &rope);

    test.press('i');
    for c in "abcdef".chars() {
        test.press(c);
    }

    assert_eq!(
        test.editor.decorations.for_line(0)[0]
            .placement
            .char_offset(),
        11,
        "inlay hint should shift right by 6 after typing 6 chars before it"
    );
}

#[test]
fn inlay_hint_follows_backspace() {
    let mut test = EditorTest::new("abclet x = 1;\n");
    let rope = test.editor.buffer().rope().clone();
    test.editor
        .decorations
        .replace_source(DecorationSource::InlayHint, vec![inlay_at(8)], &rope);

    test.press('g').press('g').press('l').press('l').press('l');
    test.press('i').press_backspace();

    assert_eq!(
        test.editor.decorations.for_line(0)[0]
            .placement
            .char_offset(),
        7,
        "inlay hint should shift left by 1 after deleting one char before it"
    );
}
