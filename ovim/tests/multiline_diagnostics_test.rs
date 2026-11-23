mod helpers;

use helpers::EditorTest;
use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
use ovim::ui::Renderer;
use ovim_core::editor::decoration::{decorations_from_diagnostics, DecorationSource};
use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
    Terminal,
};

fn diagnostic(start: (u32, u32), end: (u32, u32), message: &str) -> Diagnostic {
    Diagnostic {
        range: Range::new(Position::new(start.0, start.1), Position::new(end.0, end.1)),
        severity: Some(DiagnosticSeverity::ERROR),
        message: message.into(),
        ..Default::default()
    }
}

fn publish(test: &mut EditorTest, diagnostics: Vec<Diagnostic>) {
    let rope = test.editor.buffer().rope().clone();
    let version = test.editor.buffer().version() as u64;
    let decorations = decorations_from_diagnostics(&diagnostics, &rope, version);
    test.editor.set_test_diagnostics(diagnostics);
    test.editor
        .decorations
        .replace_source(DecorationSource::Diagnostic, decorations, &rope);
    test.editor.mark_dirty();
}

fn render(test: &mut EditorTest, width: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
    terminal
        .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut Default::default()))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn underlined_rows(cells: &Buffer, count: u16) -> Vec<String> {
    (0..count)
        .map(|row| {
            (0..cells.area.width)
                .filter_map(|col| {
                    let cell = &cells[(col, row)];
                    cell.modifier
                        .contains(Modifier::UNDERLINED)
                        .then(|| cell.symbol())
                })
                .collect()
        })
        .collect()
}

#[test]
fn every_covered_line_has_a_float_and_underline_but_only_one_eol_message() {
    let mut test = EditorTest::new("prefix(\n  first,\n  last) suffix\nclean\n");
    publish(&mut test, vec![diagnostic((0, 6), (2, 7), "call failed")]);
    test.editor.set_status_message("LSP: test ready");
    assert_eq!(
        underlined_rows(&render(&mut test, 60), 4),
        ["(", "  first,", "  last)", ""]
    );
    for line in 0..3 {
        test.set_cursor(line, 0);
        test.keys("<Space>e");
        assert!(test.editor.hover_info().unwrap().contains("call failed"));
        assert!(test.editor.status_message().is_empty());
        test.keys("<Esc>");
    }
    let cells = render(&mut test, 60);
    let text: String = cells.content.iter().map(|cell| cell.symbol()).collect();
    // One EOL message and one cursor-line echo; no copies on continuation rows.
    assert_eq!(text.matches("call failed").count(), 2);
    assert!(!test.editor.has_diagnostics_on_line(3));
}

#[test]
fn an_exclusive_endpoint_does_not_beat_the_diagnostic_under_the_cursor() {
    let mut test = EditorTest::new("abcde\n");
    publish(
        &mut test,
        vec![
            diagnostic((0, 0), (0, 3), "ends here"),
            diagnostic((0, 3), (0, 4), "starts here"),
        ],
    );
    test.set_cursor(0, 3);
    test.keys("<Space>e");
    assert!(test.editor.hover_info().unwrap().contains("starts here"));
}

#[test]
fn diagnostic_ranges_do_not_split_combining_characters_or_emoji() {
    for wrap in [false, true] {
        let mut test = EditorTest::new("éx\n👩‍💻y\n");
        test.editor.options.wrap = wrap;
        // UTF-16 endpoints inside the combining sequence and the ZWJ emoji.
        publish(
            &mut test,
            vec![
                diagnostic((0, 0), (0, 1), "accent"),
                diagnostic((1, 2), (1, 3), "emoji"),
            ],
        );
        assert_eq!(
            underlined_rows(&render(&mut test, 50), 2),
            ["é", "👩‍💻"],
            "wrap={wrap}"
        );
    }
}

#[test]
fn ansi_export_preserves_wide_diagnostic_text_and_screen_width() {
    let content = "é 👩‍💻 界 ❤️x";
    let mut test = EditorTest::new(content);
    publish(&mut test, vec![diagnostic((0, 0), (0, 12), "unicode")]);
    let cells = render(&mut test, 50);
    let plain = ovim::ui::strip_ansi(&ovim::ui::buffer_to_ansi(&cells));
    assert!(plain.lines().next().unwrap().contains(content), "{plain}");
    for row in plain.lines() {
        assert_eq!(unicode_width::UnicodeWidthStr::width(row), 50, "{row}");
    }
}

#[test]
fn scrolling_inside_a_wide_character_keeps_diagnostic_columns_aligned() {
    let mut test = EditorTest::new("界word more text beyond the viewport");
    test.editor.options.wrap = false;
    publish(&mut test, vec![diagnostic((0, 1), (0, 5), "word")]);
    // Leave room for the intentional EOL message overlay at the right edge.
    test.editor.init_window_manager(30, 12);
    test.editor
        .window_manager_mut()
        .unwrap()
        .focused_window_mut()
        .unwrap()
        .set_horizontal_offset(1);
    assert_eq!(underlined_rows(&render(&mut test, 30), 1), ["word"]);
}

#[test]
fn continuation_range_changes_invalidate_the_cached_row() {
    let mut test = EditorTest::new("begin\nabcdef\nend\n");
    let mut terminal = Terminal::new(TestBackend::new(50, 12)).unwrap();
    let mut cache = Default::default();
    // Same start line, message and buffer version; only the continuation changes.
    for (end, expected) in [(2, "ab"), (5, "abcde"), (0, "")] {
        publish(
            &mut test,
            vec![diagnostic((0, 0), (1, end), "same message")],
        );
        terminal
            .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut cache))
            .unwrap();
        assert_eq!(underlined_rows(terminal.backend().buffer(), 2)[1], expected);
    }
    for (severity, color) in [
        (DiagnosticSeverity::ERROR, Color::Red),
        (DiagnosticSeverity::WARNING, Color::Yellow),
    ] {
        let mut diag = diagnostic((0, 0), (1, 2), "same message");
        diag.severity = Some(severity);
        publish(&mut test, vec![diag]);
        terminal
            .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut cache))
            .unwrap();
        let cells = terminal.backend().buffer();
        assert_eq!(underlined_rows(cells, 2)[1], "ab");
        let mut underlined = (0..cells.area.width)
            .map(|col| &cells[(col, 1)])
            .filter(|cell| cell.modifier.contains(Modifier::UNDERLINED));
        assert!(underlined.all(|cell| cell.fg == color));
    }
    publish(&mut test, vec![]);
    terminal
        .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut cache))
        .unwrap();
    assert_eq!(underlined_rows(terminal.backend().buffer(), 2), ["", ""]);
}

#[tokio::test]
async fn code_action_context_respects_the_same_half_open_line_range() {
    let manager = ovim::lsp::LspManager::new();
    let uri = ovim::lsp::uri_from_file_path("/tmp/multiline.rs").unwrap();
    manager
        .set_diagnostics(
            uri.clone(),
            "test",
            vec![diagnostic((0, 1), (2, 0), "span")],
            Some(1),
        )
        .await;
    assert_eq!(manager.get_diagnostics_for_line(&uri, 1).await.len(), 1);
    assert!(manager.get_diagnostics_for_line(&uri, 2).await.is_empty());
}

#[test]
fn multiline_ranges_follow_edits_and_undo() {
    let mut test = EditorTest::new("begin\nmiddle\nend\nclean\n");
    publish(&mut test, vec![diagnostic((0, 2), (2, 2), "span")]);
    test.keys("ggOabove<Esc>");
    assert_eq!(
        underlined_rows(&render(&mut test, 60), 5),
        ["", "gin", "middle", "en", ""]
    );
    assert_eq!(
        test.editor.project_diagnostics().covering_line(3)[0]
            .range
            .end,
        Position::new(3, 2)
    );
    test.keys("u");
    assert_eq!(
        underlined_rows(&render(&mut test, 60), 4),
        ["gin", "middle", "en", ""]
    );
    test.keys("ggjdd");
    assert_eq!(
        underlined_rows(&render(&mut test, 60), 3),
        ["gin", "en", ""]
    );
    test.keys("u");
    assert_eq!(
        underlined_rows(&render(&mut test, 60), 4),
        ["gin", "middle", "en", ""]
    );
}

#[test]
fn tabs_line_endings_and_blank_lines_preserve_half_open_coverage() {
    for newline in ["\n", "\r\n"] {
        let content = ["a\tb", "", "  👩‍💻z", "clean", ""].join(newline);
        let mut test = EditorTest::new(&content);
        publish(&mut test, vec![diagnostic((0, 1), (3, 0), "span")]);
        assert_eq!(
            underlined_rows(&render(&mut test, 60), 4),
            ["   b", "", "  👩‍💻z", ""]
        );
        test.set_cursor(1, 0);
        assert_eq!(test.editor.current_diagnostic().as_deref(), Some("span"));
        assert!(!test.editor.has_diagnostics_on_line(3));
    }
}

#[test]
fn a_multiline_underline_survives_soft_wrapping() {
    let mut test = EditorTest::new("start 1234567890abcdef\nlast\n");
    test.editor.options.wrap = true;
    publish(&mut test, vec![diagnostic((0, 6), (1, 2), "span")]);
    assert_eq!(
        underlined_rows(&render(&mut test, 18), 6).concat(),
        "1234567890abcdefla"
    );
}

#[test]
fn empty_ranges_and_oversized_spans_remain_reachable_without_expanding_the_span() {
    let mut test = EditorTest::new("a\n\nb\n");
    publish(
        &mut test,
        vec![diagnostic((0, 0), (u32::MAX, u32::MAX), "file wide")],
    );
    assert_eq!(
        test.editor
            .project_diagnostics()
            .covering_line(1_000_000)
            .len(),
        1
    );
    assert_eq!(underlined_rows(&render(&mut test, 60), 3), ["a", "", "b"]);
    publish(&mut test, vec![diagnostic((1, 0), (1, 0), "missing token")]);
    test.set_cursor(1, 0);
    test.keys("<Space>e");
    assert!(test.editor.hover_info().unwrap().contains("missing token"));
    assert!(!test.editor.has_diagnostics_on_line(2));
}

#[tokio::test]
async fn invalid_ranges_are_removed_at_ingestion_without_losing_valid_diagnostics() {
    let manager = ovim::lsp::LspManager::new();
    let uri = ovim::lsp::uri_from_file_path("/tmp/invalid-range.rs").unwrap();
    manager
        .set_diagnostics(
            uri.clone(),
            "test",
            vec![
                diagnostic((2, 0), (0, 3), "reversed lines"),
                diagnostic((0, 3), (0, 1), "reversed columns"),
                diagnostic((0, 1), (0, 2), "valid"),
            ],
            Some(1),
        )
        .await;
    let diagnostics = manager.get_diagnostics(&uri).await;
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].message, "valid");
}
