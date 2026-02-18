use lsp_types::{Position, Range, TextEdit, WorkspaceEdit};

mod helpers;
use helpers::EditorTest;

#[test]
fn apply_workspace_edit_marks_buffer_modified_and_requests_diagnostics_refresh() {
    let tmp = tempfile::Builder::new()
        .prefix("ovim_lsp_edits_sync_")
        .suffix(".rs")
        .tempfile()
        .unwrap();
    std::fs::write(tmp.path(), "hello\n").unwrap();
    let tmp = std::fs::canonicalize(tmp.path()).unwrap();
    let tmp = tmp.to_string_lossy().to_string();

    let mut t = EditorTest::new("hello\n");
    t.set_file_path(tmp.clone());

    let uri = ovim::lsp::uri_from_file_path(&tmp).expect("uri_from_file_path failed");

    let edit = TextEdit {
        range: Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 5,
            },
        },
        new_text: "world".to_string(),
    };

    let mut changes = std::collections::HashMap::new();
    changes.insert(uri, vec![edit]);

    let we = WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    };

    t.editor.apply_workspace_edit(we).unwrap();

    assert_eq!(t.editor.buffer().line(0).unwrap(), "world\n");
    assert_eq!(
        t.editor.lsp_document_is_modified(),
        Some(true),
        "Expected LSP-applied edits to mark document modified"
    );
    assert!(
        t.editor.take_diagnostics_refresh_request(),
        "Expected LSP-applied edits to request diagnostics refresh"
    );
}
