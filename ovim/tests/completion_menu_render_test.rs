//! The completion menu as the terminal renders it: kind glyph, label,
//! `labelDetails.detail` right after the label, the description right-aligned
//! and dimmed, deprecated rows struck through, matched characters emphasised
//! and the documentation popup beside the list.

mod helpers;

use helpers::EditorTest;
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionItemLabelDetails, CompletionItemTag,
    Documentation, MarkupContent, MarkupKind,
};
use ovim::ui::Renderer;
use ratatui::{backend::TestBackend, buffer::Buffer, style::Modifier, Terminal};

const WIDTH: u16 = 100;

fn render(test: &mut EditorTest) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, 24)).unwrap();
    terminal
        .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut Default::default()))
        .unwrap();
    terminal.backend().buffer().clone()
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

/// Terminal column of `needle` in `row` (the row is a string of one-cell symbols).
fn column_of(row: &str, needle: &str) -> u16 {
    row[..row.find(needle).unwrap()].chars().count() as u16
}

fn method(label: &str, detail: &str, description: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(CompletionItemKind::METHOD),
        label_details: Some(CompletionItemLabelDetails {
            detail: Some(detail.to_string()),
            description: Some(description.to_string()),
        }),
        ..Default::default()
    }
}

fn open_menu(items: Vec<CompletionItem>, prefix: &str) -> EditorTest {
    let mut test = EditorTest::new(&format!("obj.{prefix}"));
    test.keys("A");
    test.editor
        .completion_menu_mut()
        .show(items, 4, prefix.to_string());
    test
}

#[test]
fn rows_show_kind_label_detail_and_a_right_aligned_description() {
    let mut test = open_menu(
        vec![
            method("getEmail", "()", "String"),
            CompletionItem {
                label: "ArrayList".to_string(),
                kind: Some(CompletionItemKind::CLASS),
                label_details: Some(CompletionItemLabelDetails {
                    detail: None,
                    description: Some("java.util".to_string()),
                }),
                ..Default::default()
            },
        ],
        "",
    );
    let buffer = render(&mut test);
    let all = rows(&buffer);
    let email = all
        .iter()
        .find(|row| row.contains("getEmail"))
        .expect("method row");
    assert!(email.contains(" m getEmail()"), "{email:?}");
    assert!(email.contains("String"), "{email:?}");
    let class = all
        .iter()
        .find(|row| row.contains("ArrayList"))
        .expect("class row");
    assert!(class.contains(" C ArrayList"), "{class:?}");

    // The description sits at the right edge of the menu, not next to the
    // label: both rows end their text in the same column.
    let end_of = |row: &str, needle: &str| row.find(needle).unwrap() + needle.len();
    assert_eq!(end_of(email, "String"), end_of(class, "java.util"));
    assert!(email.find("String").unwrap() > email.find("getEmail()").unwrap() + 4);
}

#[test]
fn deprecated_rows_are_struck_through_and_matches_are_emphasised() {
    let mut deprecated = method("oldCall", "()", "void");
    deprecated.tags = Some(vec![CompletionItemTag::DEPRECATED]);
    let mut test = open_menu(vec![deprecated, method("otherCall", "()", "void")], "oC");
    let buffer = render(&mut test);
    let all = rows(&buffer);
    let row = all.iter().position(|r| r.contains("oldCall")).unwrap() as u16;
    let col = column_of(&all[row as usize], "oldCall");
    assert!(
        buffer[(col + 2, row)]
            .modifier
            .contains(Modifier::CROSSED_OUT),
        "deprecated label must be struck through"
    );
    let other_row = all.iter().position(|r| r.contains("otherCall")).unwrap() as u16;
    let other_col = column_of(&all[other_row as usize], "otherCall");
    assert!(!buffer[(other_col, other_row)]
        .modifier
        .contains(Modifier::CROSSED_OUT));
    // `oC` matches the `o` and the hump `C`; the letters in between are plain.
    let bold = |c: u16| {
        buffer[(other_col + c, other_row)]
            .modifier
            .contains(Modifier::BOLD)
    };
    assert!(bold(0) && bold(5));
    assert!(!bold(2));
}

#[test]
fn the_selected_items_documentation_is_drawn_beside_the_list() {
    let mut documented = method("getEmail", "()", "String");
    documented.detail = Some("String getEmail()".to_string());
    documented.documentation = Some(Documentation::MarkupContent(MarkupContent {
        kind: MarkupKind::Markdown,
        value: "Returns the customer's email address.".to_string(),
    }));
    let mut test = open_menu(vec![documented, method("getName", "()", "String")], "get");
    let all = rows(&render(&mut test));
    assert!(
        all.iter()
            .any(|r| r.contains("Returns the customer's email")),
        "{all:#?}"
    );
    assert!(all.iter().any(|r| r.contains("String getEmail()")));

    // Moving to an item without documentation drops the popup.
    test.editor.completion_next();
    let all = rows(&render(&mut test));
    assert!(!all
        .iter()
        .any(|r| r.contains("Returns the customer's email")));
}

#[test]
fn a_long_list_shows_a_window_that_follows_the_selection_and_a_position_counter() {
    let items: Vec<_> = (0..40)
        .map(|i| CompletionItem {
            label: format!("item{i:02}"),
            sort_text: Some(format!("{i:02}")),
            ..Default::default()
        })
        .collect();
    let mut test = open_menu(items, "");
    for _ in 0..14 {
        test.editor.completion_next();
    }
    let all = rows(&render(&mut test));
    assert!(all.iter().any(|r| r.contains("item14")));
    assert!(!all.iter().any(|r| r.contains("item00")));
    assert!(all.iter().any(|r| r.contains("15/40")), "{all:#?}");
}
