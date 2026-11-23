//! Which decoration changes force the wrap map to re-measure lines.
//!
//! Only inline decorations (inlay hints) change how a line wraps. Diagnostics,
//! code lenses and fold markers sit after the text, so publishing them must
//! leave the map alone; a hint change re-measures just the lines that gained
//! or lost one.

use super::decoration::{Decoration, DecorationPlacement, DecorationSource, DecorationStyle};
use super::Editor;
use std::sync::Arc;
use std::time::{Duration, Instant};

const LINES: usize = 20_000;

/// A 20k-line Python-like file.
fn editor() -> Editor {
    let text: String = (0..LINES)
        .map(|n| format!("def function_{n}(argument_one, argument_two):\n"))
        .collect();
    let mut editor = Editor::with_content(&text);
    editor.options.wrap = true;
    editor
}

fn decoration(
    editor: &Editor,
    line: usize,
    source: DecorationSource,
    inline: bool,
    text: &str,
) -> Decoration {
    let start = editor.buffer().rope().line_to_char(line);
    Decoration {
        placement: if inline {
            DecorationPlacement::Inline {
                char_offset: start + 4,
            }
        } else {
            DecorationPlacement::EndOfLine { char_offset: start }
        },
        source,
        text: text.to_string(),
        display_width: text.chars().count(),
        style: DecorationStyle::new(crate::color::Color::Gray),
        priority: 0,
        source_version: editor.buffer().version() as u64,
    }
}

fn replace(editor: &mut Editor, source: DecorationSource, decorations: Vec<Decoration>) {
    let rope = editor.buffer().rope().clone();
    editor
        .decorations
        .replace_source(source, decorations, &rope);
}

fn recomputed(editor: &Editor) -> usize {
    editor.wrap_map().unwrap().last_recomputed_lines()
}

#[test]
fn diagnostics_and_fold_markers_do_not_remeasure_the_wrap_map() {
    let mut editor = editor();
    editor.ensure_wrap_map(80);
    editor.ensure_wrap_map(80);
    assert_eq!(recomputed(&editor), 0, "an unchanged buffer is a no-op");
    let untouched = editor.wrap_map().unwrap().line_layout(7).unwrap().clone();

    let diagnostics: Vec<Decoration> = (0..400)
        .map(|n| {
            decoration(
                &editor,
                n * 50,
                DecorationSource::Diagnostic,
                false,
                "error",
            )
        })
        .collect();
    replace(&mut editor, DecorationSource::Diagnostic, diagnostics);
    let markers: Vec<Decoration> = (0..50)
        .map(|n| decoration(&editor, n * 7, DecorationSource::Fold, false, "  ⋯ 3 lines"))
        .collect();
    replace(&mut editor, DecorationSource::Fold, markers);
    editor.ensure_wrap_map(80);

    assert_eq!(
        recomputed(&editor),
        0,
        "end-of-line text cannot change wrapping"
    );
    assert!(Arc::ptr_eq(
        &untouched,
        editor.wrap_map().unwrap().line_layout(7).unwrap()
    ));
}

#[test]
fn inlay_hint_changes_remeasure_only_the_lines_that_gain_or_lose_a_hint() {
    let mut editor = editor();
    editor.ensure_wrap_map(80);
    let untouched = editor.wrap_map().unwrap().line_layout(9).unwrap().clone();

    // 80 columns fit a bare line, not one plus a 60 column hint.
    let hint = ": a_rather_long_inferred_type_that_pushes_the_line_past_the_edge";
    let first: Vec<Decoration> = [10, 5_000, 19_000]
        .into_iter()
        .map(|line| decoration(&editor, line, DecorationSource::InlayHint, true, hint))
        .collect();
    replace(&mut editor, DecorationSource::InlayHint, first);
    editor.ensure_wrap_map(80);
    assert_eq!(recomputed(&editor), 3);
    assert_eq!(editor.wrap_map().unwrap().visual_lines_for(10), 2);
    assert_eq!(editor.wrap_map().unwrap().visual_lines_for(11), 1);

    // Hints move: line 5000 and 19000 lose theirs, 7000 gains one.
    let second: Vec<Decoration> = [10, 7_000]
        .into_iter()
        .map(|line| decoration(&editor, line, DecorationSource::InlayHint, true, hint))
        .collect();
    replace(&mut editor, DecorationSource::InlayHint, second);
    editor.ensure_wrap_map(80);
    assert!(recomputed(&editor) <= 4, "{}", recomputed(&editor));
    let map = editor.wrap_map().unwrap();
    assert_eq!(map.visual_lines_for(5_000), 1);
    assert_eq!(map.visual_lines_for(19_000), 1);
    assert_eq!(map.visual_lines_for(7_000), 2);
    assert!(Arc::ptr_eq(&untouched, map.line_layout(9).unwrap()));
}

/// The full rebuild after a width change used to project every decoration
/// for every line (0.7 s for 20k lines and 400 diagnostics in release).
#[test]
fn rebuilding_the_wrap_map_costs_lines_not_lines_times_decorations() {
    let mut editor = editor();
    let diagnostics: Vec<Decoration> = (0..400)
        .map(|n| {
            decoration(
                &editor,
                n * 50,
                DecorationSource::Diagnostic,
                false,
                "error",
            )
        })
        .collect();
    replace(&mut editor, DecorationSource::Diagnostic, diagnostics);
    editor.ensure_wrap_map(80);

    let started = Instant::now();
    editor.ensure_wrap_map(60);
    let elapsed = started.elapsed();

    assert_eq!(recomputed(&editor), editor.buffer().rope().len_lines());
    assert!(
        elapsed < Duration::from_millis(1500),
        "width change rebuild took {elapsed:?}"
    );
}

/// An edit refreshes the dirty line only; it must not pay per line either.
#[test]
fn an_edit_with_many_decorations_stays_cheap() {
    let mut editor = editor();
    let diagnostics: Vec<Decoration> = (0..400)
        .map(|n| {
            decoration(
                &editor,
                n * 50,
                DecorationSource::Diagnostic,
                false,
                "error",
            )
        })
        .collect();
    replace(&mut editor, DecorationSource::Diagnostic, diagnostics);
    editor.ensure_wrap_map(80);

    let started = Instant::now();
    for _ in 0..20 {
        editor
            .buffer_mut()
            .insert_text_at(3, crate::unicode::CharCol(1), "x");
        editor.ensure_wrap_map(80);
    }
    let elapsed = started.elapsed();

    assert_eq!(recomputed(&editor), 1);
    assert!(
        elapsed < Duration::from_millis(1500),
        "20 edits took {elapsed:?}"
    );
}
