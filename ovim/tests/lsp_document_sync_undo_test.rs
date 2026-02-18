mod helpers;

use helpers::EditorTest;
use std::sync::atomic::{AtomicU64, Ordering};

fn temp_test_path(name: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!("ovim_test_{}_{}", id, name))
        .to_string_lossy()
        .to_string()
}

#[test]
fn undo_marks_lsp_document_modified() {
    let mut t = EditorTest::new("hello\n");
    t.set_file_path(temp_test_path("ovim_sync_undo.rs"));

    // Enter insert mode and type, then exit to finalize the change.
    t.keys("A").type_text("x").keys("<Esc>");

    // The insert-mode exit path should mark modified for LSP.
    assert!(
        t.editor.lsp_document_sync_exists(),
        "Expected document sync state to exist after editing"
    );

    // Undo in normal mode should also mark the document modified so didChange is sent.
    t.keys("u");
    assert_eq!(
        t.editor.lsp_document_is_modified(),
        Some(true),
        "Expected undo to mark buffer_modified"
    );
}

#[test]
fn redo_marks_lsp_document_modified() {
    let mut t = EditorTest::new("hello\n");
    t.set_file_path(temp_test_path("ovim_sync_redo.rs"));

    t.keys("A").type_text("x").keys("<Esc>");
    t.keys("u");

    // Redo should mark modified too.
    t.keys("<C-r>");
    assert_eq!(
        t.editor.lsp_document_is_modified(),
        Some(true),
        "Expected redo to mark buffer_modified"
    );
}
