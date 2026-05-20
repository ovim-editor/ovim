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

    // accept_completion pauses the insert-mode recording session, applies
    // its own record() block as a separate Recorded entry, then resumes
    // the paused session — whose retained [Insert{8,"f"}, Insert{9,"o"}]
    // now reference offsets that no longer match the rope.
    t.editor.accept_completion();
    assert_eq!(
        t.editor.buffer().line_text(0).unwrap(),
        "let x = fooBarBazExtended"
    );

    // Type one more character — recorded with the *current* offset, into
    // the same resumed session that still carries the stale pre-completion
    // edits.
    t.type_text("Y");
    assert_eq!(
        t.editor.buffer().line_text(0).unwrap(),
        "let x = fooBarBazExtendedY"
    );

    // Exit insert mode — finalize_change_building combines the stale
    // pre-completion edits with the fresh post-completion edit into a
    // single Recorded entry.
    t.keys("<Esc>");

    // First undo should roll the insert-mode session back to the
    // post-completion state ("let x = fooBarBazExtended"). The LSP
    // completion was pushed as a separate Recorded entry, so it should
    // still be visible after one undo.
    t.keys("u");
    assert_eq!(
        t.editor.buffer().line_text(0).unwrap(),
        "let x = fooBarBazExtended",
        "after one undo: insert session should roll back to post-completion state"
    );

    // Second undo should roll back the LSP completion to "let x = fo".
    t.keys("u");
    assert_eq!(
        t.editor.buffer().line_text(0).unwrap(),
        "let x = fo",
        "after second undo: LSP completion should roll back"
    );

    // Redo twice should round-trip back to the final state.
    t.keys("<C-r>");
    assert_eq!(
        t.editor.buffer().line_text(0).unwrap(),
        "let x = fooBarBazExtended",
        "after first redo: completion reapplied"
    );
    t.keys("<C-r>");
    assert_eq!(
        t.editor.buffer().line_text(0).unwrap(),
        "let x = fooBarBazExtendedY",
        "after second redo: session reapplied"
    );
}
