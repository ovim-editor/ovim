use ovim_core::KeyCode;

mod helpers;
use helpers::EditorTest;

fn completion_item(label: &str) -> lsp_types::CompletionItem {
    lsp_types::CompletionItem {
        label: label.to_string(),
        insert_text: Some(label.to_string()),
        ..Default::default()
    }
}

#[test]
fn completion_tab_accepts_and_undo_redo_work() {
    let mut t = EditorTest::new("let x = fo");
    t.keys("A");

    // Cursor is at end; completion should replace the "fo" prefix.
    let trigger_col = "let x = ".chars().count();
    t.editor
        .completion_menu_mut()
        .show(vec![completion_item("foo")], trigger_col, "fo".to_string());

    t.press_key(KeyCode::Tab);
    assert_eq!(t.editor.buffer().line(0).unwrap(), "let x = foo\n");

    // Insert-mode changes are finalized on exit, so undo after leaving insert mode.
    t.keys("<Esc>");
    t.keys("u");
    assert_eq!(t.editor.buffer().line(0).unwrap(), "let x = fo\n");

    t.keys("<C-r>");
    assert_eq!(t.editor.buffer().line(0).unwrap(), "let x = foo\n");
}

#[test]
fn completion_ctrl_y_accepts() {
    let mut t = EditorTest::new("let x = fo");
    t.keys("A");

    let trigger_col = "let x = ".chars().count();
    t.editor
        .completion_menu_mut()
        .show(vec![completion_item("foo")], trigger_col, "fo".to_string());

    t.keys("<C-y>");
    assert_eq!(t.editor.buffer().line(0).unwrap(), "let x = foo\n");
}

#[test]
fn completion_arrows_navigate_menu_without_moving_cursor() {
    let mut t = EditorTest::new("let x = f");
    t.keys("A");

    let trigger_col = "let x = ".chars().count();
    t.editor.completion_menu_mut().show(
        vec![completion_item("foo"), completion_item("far")],
        trigger_col,
        "f".to_string(),
    );

    let before = (t.editor.buffer().cursor().line(), t.editor.buffer().cursor().col());
    t.press_key(KeyCode::Down);
    let after = (t.editor.buffer().cursor().line(), t.editor.buffer().cursor().col());
    assert_eq!(before, after);
    assert_eq!(t.editor.completion_menu().selected_index(), 1);

    t.press_key(KeyCode::Up);
    assert_eq!(t.editor.completion_menu().selected_index(), 0);
}
