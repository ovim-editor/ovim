use lsp_types::{Position, Range, TextEdit, WorkspaceEdit};

mod helpers;
use helpers::EditorTest;

fn text_edit_replace_hello_with_world() -> TextEdit {
    TextEdit {
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
    }
}

fn workspace_edit_for_uri(uri: lsp_types::Uri, edit: TextEdit) -> WorkspaceEdit {
    let mut changes = std::collections::HashMap::new();
    changes.insert(uri, vec![edit]);
    WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    }
}

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

    let we = workspace_edit_for_uri(uri, text_edit_replace_hello_with_world());

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

#[test]
fn apply_workspace_edit_current_buffer_is_undoable() {
    editor_flow_test! {
        content "hello\n";
        setup |test| {
            let tmp = tempfile::Builder::new()
                .prefix("ovim_lsp_edits_undo_current_")
                .suffix(".rs")
                .tempfile()
                .unwrap();
            std::fs::write(tmp.path(), "hello\n").unwrap();
            let tmp = std::fs::canonicalize(tmp.path()).unwrap();
            let tmp = tmp.to_string_lossy().to_string();
            test.set_file_path(tmp.clone());

            let uri = ovim::lsp::uri_from_file_path(&tmp).expect("uri_from_file_path failed");
            let we = workspace_edit_for_uri(uri, text_edit_replace_hello_with_world());
            test.editor.apply_workspace_edit(we).unwrap();
            assert_eq!(test.editor.buffer().line(0).unwrap(), "world\n");
        }
        step "u" => |test| {
            assert_eq!(test.buffer_content(), "hello\n");
        }
        step "<C-r>" => |test| {
            assert_eq!(test.buffer_content(), "world\n");
        }
    }
}

#[test]
fn apply_workspace_edit_non_current_buffer_is_undoable() {
    let tmp_current = tempfile::Builder::new()
        .prefix("ovim_lsp_edits_undo_current_buf_")
        .suffix(".rs")
        .tempfile()
        .unwrap();
    std::fs::write(tmp_current.path(), "hello\n").unwrap();
    let tmp_current = std::fs::canonicalize(tmp_current.path()).unwrap();
    let tmp_current = tmp_current.to_string_lossy().to_string();

    let tmp_other = tempfile::Builder::new()
        .prefix("ovim_lsp_edits_undo_other_buf_")
        .suffix(".rs")
        .tempfile()
        .unwrap();
    std::fs::write(tmp_other.path(), "alpha\n").unwrap();
    let tmp_other = std::fs::canonicalize(tmp_other.path()).unwrap();
    let tmp_other = tmp_other.to_string_lossy().to_string();

    let mut t = EditorTest::new("hello\n");
    t.set_file_path(tmp_current);

    // Pre-open the other file as a background buffer to avoid file-loading/runtime
    // concerns in tests; workspace edits should still apply to non-current buffers.
    let mut other = ovim::buffer::Buffer::new_from_str("alpha\n");
    other.set_file_path(tmp_other.clone());
    t.editor.add_buffer(other);
    t.editor.switch_to_buffer(0); // Keep original buffer current.

    let uri_other = ovim::lsp::uri_from_file_path(&tmp_other).expect("uri_from_file_path failed");
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
        new_text: "beta".to_string(),
    };
    let we = workspace_edit_for_uri(uri_other, edit);
    t.editor.apply_workspace_edit(we).unwrap();

    // Workspace edit edits the other file without switching away.
    assert_eq!(t.editor.current_buffer_index(), 0);
    let names = t.editor.buffer_names();
    let other_idx = names
        .iter()
        .position(|name| name == &tmp_other)
        .expect("other file buffer should be loaded");
    t.editor.switch_to_buffer(other_idx);
    assert_eq!(t.editor.buffer().line(0).unwrap(), "beta\n");

    t.keys("u");
    assert_eq!(t.editor.buffer().line(0).unwrap(), "alpha\n");
    t.keys("<C-r>");
    assert_eq!(t.editor.buffer().line(0).unwrap(), "beta\n");
}
