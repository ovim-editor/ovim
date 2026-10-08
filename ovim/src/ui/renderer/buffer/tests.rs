use super::decorations::{
    apply_eol_decorations, line_decoration_cache_hash, pad_line_to, place_eol_on_line,
    EolPlacement, EOL_DIAG_GAP,
};
use super::gutter::line_is_in_walkthrough;
use super::indexed::{render_indexed_fragments, resolve_indexed_syntax, IndexedRowStyles};
use super::legacy::{render_line_with_highlights, RemappedDiagnostic};
use super::rows::split_line_into_rows;
use super::viewport::{
    display_col_to_char_idx, expanded_char_to_display_col, slice_horizontal_viewport,
};
use super::*;
use crate::syntax::HighlightGroup;
use crate::ui::renderer::helpers::expand_tabs_with_mapping;
use ratatui::{
    style::Modifier,
    text::{Line, Span},
};

#[test]
fn walkthrough_range_marks_every_inclusive_logical_line() {
    let range = Some((4, 6));
    assert!(!line_is_in_walkthrough(range, 3));
    assert!(line_is_in_walkthrough(range, 4));
    assert!(line_is_in_walkthrough(range, 5));
    assert!(line_is_in_walkthrough(range, 6));
    assert!(!line_is_in_walkthrough(range, 7));
    assert!(!line_is_in_walkthrough(None, 5));
}

fn plain_indexed_styles(theme: &Theme) -> IndexedRowStyles<'_> {
    IndexedRowStyles {
        theme,
        syntax: Vec::new(),
        selected: None,
        search: &[],
        diagnostics: Vec::new(),
        backgrounds: Vec::new(),
        cursorline: false,
        yank: None,
        ai: None,
        links: &[],
        bracket: None,
        walkthrough: false,
    }
}

#[test]
fn indexed_visible_rows_match_legacy_tabs_wide_and_overlay_priority() {
    let theme = Theme::default();
    let text = "a\t中e\u{301}yz";
    let index = ovim_core::text_index::LineIndex::from_text(text);
    let geometry =
        ovim_core::line_layout::IndexedLineLayout::new(index, 5, 4, std::sync::Arc::from([]));
    let expanded = expand_tabs_with_mapping(text, 4);
    let legacy = split_line_into_rows(Line::from(expanded.text), 5);
    let rows = geometry.row_fragments(0..geometry.row_count());
    let plain = plain_indexed_styles(&theme);
    let rendered: Vec<_> = rows
        .iter()
        .map(|row| {
            let mut line = render_indexed_fragments(&row.fragments, &plain, &[]);
            pad_line_to(&mut line, 5);
            line
        })
        .collect();
    let strings = |lines: &[Line<'_>]| {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(strings(&rendered), strings(&legacy));

    let mut styled = plain_indexed_styles(&theme);
    styled.selected = Some(1..3); // tab and CJK glyph, including split-tab spaces
    styled.search = &[(0, 4)];
    styled.diagnostics = vec![RemappedDiagnostic {
        start: 1,
        end: 3,
        color: Color::Red,
    }];
    let row = render_indexed_fragments(&rows[0].fragments, &styled, &[]);
    let tab = row
        .spans
        .iter()
        .find(|span| span.content.as_ref() == "   ")
        .unwrap();
    assert_eq!(
        tab.style.bg,
        Some(crate::key_convert::convert_core_color(
            theme.get_ui_color(UiGroup::Visual)
        ))
    );
    assert_eq!(tab.style.fg, Some(Color::Red));
    assert!(tab.style.add_modifier.contains(Modifier::UNDERLINED));
}

#[test]
fn indexed_syntax_sweep_preserves_original_specificity_after_clipping() {
    let highlights = vec![
        (0..100, HighlightGroup::String),
        (45..55, HighlightGroup::Keyword),
        (49..51, HighlightGroup::Function),
    ];
    let resolved = resolve_indexed_syntax(&highlights, 48..53);
    assert_eq!(
        resolved,
        vec![
            (48..49, HighlightGroup::Keyword),
            (49..51, HighlightGroup::Function),
            (51..53, HighlightGroup::Keyword)
        ]
    );
}

#[test]
fn long_line_subrow_renderer_seeks_directly_and_continues_to_next_line() {
    use ratatui::{backend::TestBackend, Terminal};
    let mut editor = Editor::with_content(&format!("{}END\ntail\n", "a".repeat(10_000)));
    editor.options.wrap = true;
    editor.options.cursorline = false;
    editor.options.showmatch = false;
    editor.ensure_wrap_map(5);
    let layout = BufferLayout {
        buffer_area: Rect::new(0, 0, 5, 3),
        render_area: Rect::new(0, 0, 5, 3),
        gutter_width: 0,
        text_width: 5,
        line_num_width: 0,
        blame_width: 0,
        fold_width: 0,
        scrollbar_area: None,
    };
    let context = WindowRenderContext {
        scroll_offset: Some(0),
        scroll_subrow: Some(2000),
        ..Default::default()
    };
    let mut terminal = Terminal::new(TestBackend::new(5, 3)).unwrap();
    let mut cache = super::super::line_cache::LineRenderCache::new();
    terminal
        .draw(|frame| {
            render_buffer(
                frame,
                &editor,
                &Theme::default(),
                &layout,
                &mut cache,
                Some(&context),
            );
        })
        .unwrap();
    let cells = terminal.backend().buffer();
    let row = |y| (0..5).map(|x| cells[(x, y)].symbol()).collect::<String>();
    assert_eq!(row(0), "END  ");
    assert_eq!(row(1), "tail ");
    assert_eq!(row(2), "     ");
}

/// A long line scrolled to a wide glyph: the `<` covers the first visible
/// cell, so a glyph cut by the edge is hidden (blank for its other half)
/// and every later character sits exactly `h_offset` columns left of its
/// display column.
#[test]
fn long_nowrap_viewport_covers_the_wide_glyph_cut_by_the_left_edge() {
    use ratatui::{backend::TestBackend, Terminal};
    let mut editor = Editor::with_content(&format!("{}界word", "x".repeat(5000)));
    editor.options.wrap = false;
    editor.options.cursorline = false;
    editor.options.showmatch = false;
    let layout = BufferLayout {
        buffer_area: Rect::new(0, 0, 5, 1),
        render_area: Rect::new(0, 0, 5, 1),
        gutter_width: 0,
        text_width: 5,
        line_num_width: 0,
        blame_width: 0,
        fold_width: 0,
        scrollbar_area: None,
    };
    // 界 fills columns 5000-5001 and `w` is column 5002.
    for (h_offset, expected) in [
        (5001, "<word"),
        (5000, "< wo>"),
        (4999, "<界 w>"),
        (4998, "<x界 >"),
    ] {
        let context = WindowRenderContext {
            scroll_offset: Some(0),
            horizontal_offset: Some(h_offset),
            ..Default::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(5, 1)).unwrap();
        let mut cache = super::super::line_cache::LineRenderCache::new();
        terminal
            .draw(|frame| {
                render_buffer(
                    frame,
                    &editor,
                    &Theme::default(),
                    &layout,
                    &mut cache,
                    Some(&context),
                );
            })
            .unwrap();
        let cells = terminal.backend().buffer();
        let row: String = (0..5).map(|x| cells[(x, 0)].symbol()).collect();
        assert_eq!(row, expected, "h_offset {h_offset}");
    }
}

#[test]
fn long_concealed_line_uses_cached_source_mapping_for_visible_link_style() {
    use ratatui::{backend::TestBackend, Terminal};
    let mut editor = Editor::with_content(&format!(
        "cursor\n{}[link](https://example.test)x\n",
        "a".repeat(5000)
    ));
    editor.buffer_mut().set_file_path("long.md".to_string());
    editor.options.wrap = true;
    editor.options.markdown_conceal = true;
    editor.options.cursorline = false;
    editor.options.showmatch = false;
    editor.ensure_wrap_map(5);
    let layout = BufferLayout {
        buffer_area: Rect::new(0, 0, 5, 1),
        render_area: Rect::new(0, 0, 5, 1),
        gutter_width: 0,
        text_width: 5,
        line_num_width: 0,
        blame_width: 0,
        fold_width: 0,
        scrollbar_area: None,
    };
    let context = WindowRenderContext {
        scroll_offset: Some(1),
        scroll_subrow: Some(1000),
        ..Default::default()
    };
    let mut terminal = Terminal::new(TestBackend::new(5, 1)).unwrap();
    let mut cache = super::super::line_cache::LineRenderCache::new();
    terminal
        .draw(|frame| {
            render_buffer(
                frame,
                &editor,
                &Theme::default(),
                &layout,
                &mut cache,
                Some(&context),
            );
        })
        .unwrap();
    let cells = terminal.backend().buffer();
    assert_eq!(
        (0..5).map(|x| cells[(x, 0)].symbol()).collect::<String>(),
        "linkx"
    );
    assert_eq!(cells[(0, 0)].fg, Color::Rgb(100, 149, 237));
    assert!(cells[(0, 0)].modifier.contains(Modifier::UNDERLINED));
    assert!(!cells[(4, 0)].modifier.contains(Modifier::UNDERLINED));
}

#[test]
fn test_split_line_wide_char_at_boundary() {
    // Width=4, content "abc世d"
    // 'a'=1, 'b'=1, 'c'=1, '世'=2 -> doesn't fit (3+2=5 > 4), pad row 1
    // Row 1: "abc " (padded), Row 2: "世d  " (padded)
    let line = Line::from(vec![Span::raw("abc世d")]);
    let rows = split_line_into_rows(line, 4);
    assert_eq!(rows.len(), 2);

    let row0_text: String = rows[0].spans.iter().map(|s| s.content.as_ref()).collect();
    let row1_text: String = rows[1].spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(row0_text, "abc ");
    assert_eq!(row1_text, "世d "); // 世=2 + d=1 = 3, pad 1 to fill width 4
}

#[test]
fn test_split_line_ascii_no_wide() {
    let line = Line::from(vec![Span::raw("abcdefgh")]);
    let rows = split_line_into_rows(line, 4);
    assert_eq!(rows.len(), 2);

    let row0_text: String = rows[0].spans.iter().map(|s| s.content.as_ref()).collect();
    let row1_text: String = rows[1].spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(row0_text, "abcd");
    assert_eq!(row1_text, "efgh");
}

#[test]
fn test_split_line_fits_in_one_row() {
    let line = Line::from(vec![Span::raw("ab")]);
    let rows = split_line_into_rows(line, 4);
    assert_eq!(rows.len(), 1);

    let text: String = rows[0].spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "ab  "); // padded
}

#[test]
fn horizontal_viewport_preserves_graphemes_and_excludes_indicators_and_padding() {
    for (line, offset, width, expected, highlighted) in [
        ("hello", 0, 10, "hello", Some(0..5)),
        ("hello world!", 0, 6, "hello>", Some(0..5)),
        // The `<` covers the first visible cell: `l` (column 3) is hidden
        // and `o` (column 4) sits one cell in, 3 columns from the left.
        ("hello world!", 3, 6, "<o wo>", Some(1..5)),
        ("a世b", 0, 5, "a世b", Some(0..3)),
        ("a世b世c", 0, 5, "a世b>", Some(0..3)),
        ("a世b世c", 3, 5, "<世c ", Some(1..3)),
        ("a界b界c", 0, 3, "a >", Some(0..1)),
        // A wide glyph cut by the left edge is hidden; a blank keeps the
        // next character in its column.
        ("a界b界c", 1, 4, "< b>", Some(2..3)),
        ("界word", 1, 5, "<word", Some(1..5)),
        ("hello", 0, 0, "", None),
        ("hello", 0, 1, ">", None),
        ("hello", 3, 1, "<", None),
    ] {
        let viewport = slice_horizontal_viewport(line, offset, width);
        assert_eq!(
            viewport.text, expected,
            "line={line:?}, offset={offset}, width={width}"
        );
        assert_eq!(
            viewport.project_range(0..usize::MAX),
            highlighted,
            "line={line:?}"
        );
    }
}

#[test]
fn horizontal_viewport_clips_ranges_to_the_displayed_source_characters() {
    let viewport = slice_horizontal_viewport("é word tail", 2, 6);
    assert_eq!(viewport.text, "<ord >");
    assert_eq!(viewport.project_range(0..2), None);
    assert_eq!(viewport.project_range(3..5), Some(1..2));
    assert_eq!(viewport.project_range(5..20), Some(2..5));
    assert_eq!(viewport.project_range(8..20), None);
}

// --- Helper function tests ---

#[test]
fn test_expanded_char_to_display_col() {
    // "a世b" → char 0='a'(width 1), char 1='世'(width 2), char 2='b'(width 1)
    assert_eq!(expanded_char_to_display_col("a世b", 0), 0);
    assert_eq!(expanded_char_to_display_col("a世b", 1), 1);
    assert_eq!(expanded_char_to_display_col("a世b", 2), 3);
}

#[test]
fn test_display_col_to_char_idx_basic() {
    // "a世b" display cols: a=0, 世=1-2, b=3
    assert_eq!(display_col_to_char_idx("a世b", 0), 0);
    assert_eq!(display_col_to_char_idx("a世b", 1), 1);
    assert_eq!(display_col_to_char_idx("a世b", 2), 1); // mid-wide → same char
    assert_eq!(display_col_to_char_idx("a世b", 3), 2);
}

#[test]
fn test_apply_eol_decorations_on_padded_wrapped_row() {
    use ovim_core::editor::decoration::*;

    let base = Line::from("let x = 1;".to_string());
    let mut rows = split_line_into_rows(base, 30);
    let mut first = rows.remove(0);

    let dec = Decoration {
        placement: DecorationPlacement::EndOfLine { char_offset: 0 },
        source: DecorationSource::Diagnostic,
        text: "\u{f057} uh oh".to_string(),
        display_width: 7,
        style: DecorationStyle::new(ovim_core::color::Color::Red).with_italic(),
        priority: 0,
        source_version: 0,
    };

    apply_eol_decorations(&mut first, &[&dec], EolPlacement::Append { text_width: 30 });

    let mut rendered = String::new();
    for span in &first.spans {
        rendered.push_str(span.content.as_ref());
    }
    assert!(rendered.contains("uh oh"));

    let display_width: usize = rendered
        .chars()
        .map(crate::display::char_display_width)
        .sum();
    assert_eq!(display_width, 30);
}

/// Regression: under soft wrap (the default), `split_line_into_rows` pads
/// every row to `text_width`. Measuring that padding made
/// `place_eol_on_line` treat every short line as "full" and take the
/// overlay branch — the diagnostic landed right-aligned at the screen
/// edge, capped to a third of the width, far away from its code.
#[test]
fn test_padded_wrapped_row_gets_adjacent_eol_not_edge_overlay() {
    use ovim_core::editor::decoration::*;

    let text_width = 60;
    let base = Line::from("let x = 1;".to_string());
    let mut rows = split_line_into_rows(base, text_width);
    assert_eq!(rows.len(), 1);
    let mut row = rows.remove(0);

    let dec = Decoration {
        placement: DecorationPlacement::EndOfLine { char_offset: 0 },
        source: DecorationSource::Diagnostic,
        text: "unused variable: `x`".to_string(),
        display_width: 20,
        style: DecorationStyle::new(ovim_core::color::Color::Red).with_italic(),
        priority: 0,
        source_version: 0,
    };

    place_eol_on_line(&mut row, &[&dec], text_width, text_width);

    let rendered: String = row.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        rendered.starts_with("let x = 1;  unused variable: `x`"),
        "diagnostic must sit {EOL_DIAG_GAP} columns after the code, not at the far edge; got {rendered:?}"
    );
    let display_width: usize = rendered
        .chars()
        .map(crate::display::char_display_width)
        .sum();
    assert_eq!(display_width, text_width, "row is re-padded to full width");
}

/// A row whose real content fills the box still uses the overlay so the
/// diagnostic stays visible.
#[test]
fn test_full_content_row_still_overlays_at_edge() {
    use ovim_core::editor::decoration::*;

    let text_width = 40;
    let full: String = "x".repeat(text_width);
    let mut row = Line::from(full);

    let dec = Decoration {
        placement: DecorationPlacement::EndOfLine { char_offset: 0 },
        source: DecorationSource::Diagnostic,
        text: "too long".to_string(),
        display_width: 8,
        style: DecorationStyle::new(ovim_core::color::Color::Red).with_italic(),
        priority: 0,
        source_version: 0,
    };

    place_eol_on_line(&mut row, &[&dec], text_width, text_width);

    let rendered: String = row.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        rendered.ends_with("too long"),
        "overlay should place the message at the right edge; got {rendered:?}"
    );
    let display_width: usize = rendered
        .chars()
        .map(crate::display::char_display_width)
        .sum();
    assert_eq!(display_width, text_width);
}

/// OV-00329 regression: a diagnostic republish WITHOUT a buffer edit
/// (save → cargo-check adds a second diagnostic at a new span while the
/// top message stays the same) must change the line's render cache key.
/// The decoration fingerprint alone — the only diagnostic-derived key
/// component before the fix — collides in this scenario, so the cached
/// row (with the old squiggle baked in) would be served until an
/// edit/scroll/resize.
#[test]
fn test_diagnostic_republish_without_edit_changes_line_cache_key() {
    use crate::ui::renderer::line_cache::{LineCacheFrame, LineRenderCache};
    use ovim_core::editor::decoration::{decorations_from_diagnostics, DecorationSource};

    let mut editor = Editor::with_content("let x = 1;\n");

    let top = lsp_types::Diagnostic {
        range: lsp_types::Range::new(
            lsp_types::Position::new(0, 4),
            lsp_types::Position::new(0, 5),
        ),
        severity: Some(lsp_types::DiagnosticSeverity::ERROR),
        message: "top message".to_string(),
        ..lsp_types::Diagnostic::default()
    };
    let extra = lsp_types::Diagnostic {
        range: lsp_types::Range::new(
            lsp_types::Position::new(0, 8),
            lsp_types::Position::new(0, 9),
        ),
        severity: Some(lsp_types::DiagnosticSeverity::WARNING),
        message: "second span".to_string(),
        ..lsp_types::Diagnostic::default()
    };

    let rope = editor.buffer().rope().clone();
    let version = editor.buffer().version() as u64;

    editor.set_test_diagnostics(vec![top.clone()]);
    editor.decorations.replace_source(
        DecorationSource::Diagnostic,
        decorations_from_diagnostics(std::slice::from_ref(&top), &rope, version),
        &rope,
    );
    let decs1 = editor
        .decorations
        .project_all(&rope, editor.buffer().edit_log());
    let h1 = line_decoration_cache_hash(&decs1, &editor.project_diagnostics(), 0);

    // Republish: second diagnostic at a new span, top message unchanged,
    // no buffer edit.
    editor.set_test_diagnostics(vec![top.clone(), extra.clone()]);
    editor.decorations.replace_source(
        DecorationSource::Diagnostic,
        decorations_from_diagnostics(&[top, extra], &rope, version),
        &rope,
    );
    let decs2 = editor
        .decorations
        .project_all(&rope, editor.buffer().edit_log());

    // The pre-fix key component cannot see the change (same best-severity
    // EOL decoration) — this is exactly the collision the fix closes.
    assert_eq!(decs1.line_hash(0), decs2.line_hash(0));

    let h2 = line_decoration_cache_hash(&decs2, &editor.project_diagnostics(), 0);
    assert_ne!(
        h2, h1,
        "cache key must change when the diagnostic set changes"
    );

    // And a row cached under the old key re-renders under the new one.
    let mut cache = LineRenderCache::new();
    let frame = LineCacheFrame {
        buffer_id: 1,
        buffer_version: 1,
        highlight_generation: 0,
        h_offset: 0,
        text_width: 80,
        wrap: false,
        tab_width: 4,
        markdown_conceal: false,
    };
    cache.begin_frame(frame);
    cache.put(frame.key(0, h1), Line::from("row"), true);
    assert!(cache.get(&frame.key(0, h1)).is_some());
    assert!(cache.get(&frame.key(0, h2)).is_none());
}

#[test]
fn test_render_line_empty_string() {
    let theme = Theme::default();
    let line = render_line_with_highlights(&theme, "", None, &[], &[], &[], &[], &[]);
    assert!(line.spans.is_empty());
}

#[test]
fn test_render_line_plain_text_single_span() {
    let theme = Theme::default();
    let line = render_line_with_highlights(&theme, "hello world", None, &[], &[], &[], &[], &[]);
    // No highlights → should coalesce into one span
    assert_eq!(line.spans.len(), 1);
    assert_eq!(line.spans[0].content.as_ref(), "hello world");
}

#[test]
fn test_render_line_syntax_highlight_splits_spans() {
    let theme = Theme::default();
    // Highlight bytes 0..2 ("fn") as Keyword
    let highlights = vec![(0..2, crate::syntax::HighlightGroup::Keyword)];
    let line =
        render_line_with_highlights(&theme, "fn main()", None, &[], &highlights, &[], &[], &[]);
    // Should have at least 2 spans: "fn" (highlighted) and " main()" (default)
    assert!(line.spans.len() >= 2);
    assert_eq!(line.spans[0].content.as_ref(), "fn");
}

#[test]
fn test_render_line_search_match_overrides_syntax() {
    let theme = Theme::default();
    let highlights = vec![(0..5, crate::syntax::HighlightGroup::Function)];
    // Search match on chars 0..5 ("hello")
    let search = vec![(0, 5)];
    let line = render_line_with_highlights(
        &theme,
        "hello world",
        None,
        &search,
        &highlights,
        &[],
        &[],
        &[],
    );
    // First span should be search-highlighted, not syntax-highlighted
    assert!(line.spans.len() >= 2);
    assert_eq!(line.spans[0].content.as_ref(), "hello");
    // Search highlight has bg color (non-default style)
    assert_ne!(line.spans[0].style, Style::default());
}

#[test]
fn test_render_line_multibyte_chars() {
    let theme = Theme::default();
    // "aé" - 'é' is 2 bytes in UTF-8. Highlight byte range 0..1 ("a" only).
    let highlights = vec![(0..1, crate::syntax::HighlightGroup::Keyword)];
    let line = render_line_with_highlights(&theme, "aéb", None, &[], &highlights, &[], &[], &[]);
    assert!(line.spans.len() >= 2);
    assert_eq!(line.spans[0].content.as_ref(), "a");
}

#[test]
fn test_render_line_diagnostic_underline() {
    let theme = Theme::default();
    let diags = vec![RemappedDiagnostic {
        start: 0,
        end: 5,
        color: Color::Red,
    }];
    let line = render_line_with_highlights(&theme, "error here", None, &[], &[], &diags, &[], &[]);
    // First span should have underline modifier
    assert!(line.spans[0]
        .style
        .add_modifier
        .contains(Modifier::UNDERLINED));
}

#[test]
fn test_render_line_most_specific_syntax_wins() {
    let theme = Theme::default();
    // Two overlapping highlights: broad (0..10) and narrow (2..4).
    // The narrow one should win for chars at byte positions 2-3.
    let highlights = vec![
        (0..10, crate::syntax::HighlightGroup::Variable),
        (2..4, crate::syntax::HighlightGroup::Keyword),
    ];
    let line =
        render_line_with_highlights(&theme, "abcdefghij", None, &[], &highlights, &[], &[], &[]);
    // Should have 3 spans: "ab" (Variable), "cd" (Keyword), "efghij" (Variable)
    assert!(line.spans.len() >= 3);
    assert_eq!(line.spans[0].content.as_ref(), "ab");
    assert_eq!(line.spans[1].content.as_ref(), "cd");
}
