//! Popups that open around the cursor line must not cover it or each other:
//! parameter hints, the completion menu and its documentation.

mod helpers;

use helpers::EditorTest;
use lsp_types::{
    CompletionItem, Documentation, MarkupContent, MarkupKind, ParameterInformation, ParameterLabel,
    SignatureHelp, SignatureInformation,
};
use ovim::ui::Renderer;
use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};

fn render(test: &mut EditorTest, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut Default::default()))
        .unwrap();
    rows(terminal.backend().buffer())
}

fn rows(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|row| {
            (0..buffer.area.width)
                .map(|col| buffer[(col, row)].symbol())
                .collect::<String>()
        })
        .collect()
}

fn items(count: usize) -> Vec<CompletionItem> {
    (0..count)
        .map(|i| CompletionItem {
            label: format!("candidate_{i:02}"),
            sort_text: Some(format!("{i:02}")),
            ..Default::default()
        })
        .collect()
}

fn signature() -> SignatureHelp {
    let label = "compute(first: i32, second: i32)";
    let parameter = |text: &str| ParameterInformation {
        label: ParameterLabel::Simple(text.to_string()),
        documentation: None,
    };
    SignatureHelp {
        signatures: vec![SignatureInformation {
            label: label.to_string(),
            documentation: None,
            parameters: Some(vec![parameter("first: i32"), parameter("second: i32")]),
            active_parameter: None,
        }],
        active_signature: Some(0),
        active_parameter: Some(0),
    }
}

/// An editor typing at the end of `line_text` on `row`, among filler lines,
/// in a `width` x `height` terminal.
fn typing(line_text: &str, row: usize, width: u16, height: u16) -> EditorTest {
    let mut text = String::new();
    for n in 0..30 {
        if n == row {
            text.push_str(line_text);
        } else {
            text.push_str(&format!("filler line {n}"));
        }
        text.push('\n');
    }
    let mut test = EditorTest::new(&text);
    ovim::frontend::handle_viewport_resize(&mut test.editor, width, height);
    test.keys(&format!("{}G", row + 1));
    test.keys("A");
    test
}

fn row_of(all: &[String], needle: &str) -> Option<usize> {
    all.iter().position(|row| row.contains(needle))
}

/// Parameter hints used to drop below the line when the top had no room, onto
/// the very rows the completion menu opens in.
#[test]
fn parameter_hints_do_not_cover_the_completion_menu() {
    // First line of the file: nothing above the cursor for the hints.
    let mut test = typing("let x = compute(", 0, 80, 24);
    test.editor
        .completion_menu_mut()
        .show(items(5), 16, String::new());
    assert!(test.editor.show_signature_help(&signature()));

    let all = render(&mut test, 80, 24);
    for n in 0..5 {
        assert!(
            row_of(&all, &format!("candidate_{n:02}")).is_some(),
            "item {n} is covered:\n{}",
            all.join("\n")
        );
    }
    let hints = row_of(&all, "compute(first: i32").expect("hints are shown");
    let first = row_of(&all, "candidate_00").unwrap();
    let last = row_of(&all, "candidate_04").unwrap();
    assert!(
        hints < first.saturating_sub(1) || hints > last + 1,
        "hints row {hints} inside the menu rows {first}..={last}:\n{}",
        all.join("\n")
    );
}

/// Hints and menu both want the rows above the cursor at the bottom of the
/// window: the hints stack above the menu instead of overlapping it.
#[test]
fn parameter_hints_stack_clear_of_a_menu_that_opened_upward() {
    let mut test = typing("let x = compute(", 29, 80, 14);
    test.editor
        .completion_menu_mut()
        .show(items(3), 16, String::new());
    assert!(test.editor.show_signature_help(&signature()));
    let all = render(&mut test, 80, 14);
    let first = row_of(&all, "candidate_00").expect("menu is shown");
    let last = row_of(&all, "candidate_02").expect("menu is complete");
    let typed = row_of(&all, "let x = compute(").unwrap();
    assert!(
        last < typed,
        "menu is above the typed line:\n{}",
        all.join("\n")
    );
    let hints = row_of(&all, "compute(first: i32").expect("hints are shown");
    assert!(
        hints + 1 < first,
        "hints row {hints} touches the menu rows {first}..={last}:\n{}",
        all.join("\n")
    );
}

/// A menu that fits neither above nor below used to be pushed over the line
/// being typed.
#[test]
fn the_completion_menu_never_covers_the_line_being_typed() {
    let typed = "let value = compute_result(arg);";
    for (width, height, row) in [(80, 10, 3), (80, 12, 4), (80, 9, 2), (60, 16, 7)] {
        let mut test = typing(typed, row, width, height);
        test.editor
            .completion_menu_mut()
            .show(items(12), 0, String::new());
        let all = render(&mut test, width, height);
        let line = row_of(&all, "let value = compute_result");
        let line = line.unwrap_or_else(|| panic!("typed line is covered:\n{}", all.join("\n")));
        assert!(
            all[line].contains(typed),
            "{width}x{height}: typed line is partly covered:\n{}",
            all.join("\n")
        );
    }
}

/// Documentation beside a menu that opened upward stays above the cursor line.
#[test]
fn documentation_beside_an_upward_menu_does_not_cover_the_typed_line() {
    let typed = "let value = candidate_0";
    let mut test = typing(typed, 29, 120, 14);
    let mut documented = items(3);
    documented[0].documentation = Some(Documentation::MarkupContent(MarkupContent {
        kind: MarkupKind::Markdown,
        value: (0..8)
            .map(|n| format!("Documentation line {n} for the selected candidate."))
            .collect::<Vec<_>>()
            .join("\n\n"),
    }));
    test.editor
        .completion_menu_mut()
        .show(documented, 12, "candidate_0".to_string());
    let all = render(&mut test, 120, 14);
    let line = row_of(&all, "let value = candidate_0").expect("typed line");
    let docs = all
        .iter()
        .enumerate()
        .filter(|(_, row)| row.contains("Documentation line"))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    assert!(!docs.is_empty(), "docs are shown:\n{}", all.join("\n"));
    assert!(
        docs.iter().all(|&row| row < line),
        "docs {docs:?} reach the typed line {line}:\n{}",
        all.join("\n")
    );
}
