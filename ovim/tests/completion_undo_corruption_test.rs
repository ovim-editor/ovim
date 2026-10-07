// Regression: typing in insert mode, accepting an LSP completion, typing
// more, then undoing must not corrupt the rope. The previous
// `pause_recording` / `resume_recording` flow inside `accept_completion`
// retained pre-completion edits with stale absolute char offsets and
// concatenated them with post-completion edits into a single Recorded
// entry — undo then applied those stale offsets against a buffer whose
// layout had shifted, gouging characters out of the inserted completion
// text. Fix: finalize the insert-mode session before completion edits
// land, then restart it after, so each batch of edits forms its own
// Recorded entry at offsets that match the rope state it was captured in.

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
fn typing_then_completion_then_typing_then_undo_corrupts_buffer() {
    let mut t = EditorTest::new("let x = ");

    // Append at end of line; cursor lands at offset 8.
    t.keys("A");

    // Type "fo" — recorded as Insert{8,"f"}, Insert{9,"o"} in the
    // insert-mode recording session.
    t.type_text("fo");
    assert_eq!(t.editor.buffer().line_text(0).unwrap(), "let x = fo");

    // Show a completion that replaces the typed "fo" with a much longer
    // identifier so positional drift is unmistakable.
    let trigger_col = "let x = ".chars().count();
    t.editor.completion_menu_mut().show(
        vec![completion_item("fooBarBazExtended")],
        trigger_col,
        "fo".to_string(),
    );

    // The completion joins the insert session: its edits are recorded in
    // order with the typed ones, so every offset still matches the rope at
    // the moment it is undone or redone.
    t.editor.accept_completion();
    assert_eq!(
        t.editor.buffer().line_text(0).unwrap(),
        "let x = fooBarBazExtended"
    );

    // Type one more character into the same session.
    t.type_text("Y");
    assert_eq!(
        t.editor.buffer().line_text(0).unwrap(),
        "let x = fooBarBazExtendedY"
    );
    t.keys("<Esc>");

    // Typing, completing and typing on are ONE insert: one undo removes all
    // of it (vim: `ofoo<C-n> baz<Esc>` then `u` removes the whole line).
    t.keys("u");
    assert_eq!(
        t.editor.buffer().line_text(0).unwrap(),
        "let x = ",
        "one undo takes the whole insert back, without corrupting the rope"
    );

    // Redo round-trips back to the final state.
    t.keys("<C-r>");
    assert_eq!(
        t.editor.buffer().line_text(0).unwrap(),
        "let x = fooBarBazExtendedY",
        "redo reapplies the whole insert"
    );

    // And undo/redo can be repeated without drift.
    t.keys("u");
    assert_eq!(t.editor.buffer().line_text(0).unwrap(), "let x = ");
    t.keys("<C-r>");
    assert_eq!(
        t.editor.buffer().line_text(0).unwrap(),
        "let x = fooBarBazExtendedY"
    );
}

/// `.` repeats the insert with the completed text, as vim does.
/// Reference: `nvim --clean`: `ofoo<C-n> baz<Esc>` over a buffer holding
/// `foobar`, then `.`, gives a second `foobar baz` line.
#[test]
fn dot_repeats_an_insert_that_used_a_completion() {
    let mut t = EditorTest::new("let x = \nlet y = ");
    t.keys("A");
    t.type_text("fo");
    let trigger_col = "let x = ".chars().count();
    t.editor.completion_menu_mut().show(
        vec![completion_item("fooBar")],
        trigger_col,
        "fo".to_string(),
    );
    t.editor.accept_completion();
    t.type_text("!");
    t.keys("<Esc>");
    assert_eq!(t.editor.buffer().line_text(0).unwrap(), "let x = fooBar!");

    t.keys("j.");
    assert_eq!(
        t.editor.buffer().line_text(1).unwrap(),
        "let y = fooBar!",
        "the repeat types the completed word, not just what was typed after it"
    );
}
