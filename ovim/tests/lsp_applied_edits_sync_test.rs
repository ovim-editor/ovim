use lsp_types::{Position, Range, TextEdit, WorkspaceEdit};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

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

fn workspace_edit_for_resource_op(op: lsp_types::ResourceOp) -> WorkspaceEdit {
    WorkspaceEdit {
        changes: None,
        document_changes: Some(lsp_types::DocumentChanges::Operations(vec![
            lsp_types::DocumentChangeOperation::Op(op),
        ])),
        change_annotations: None,
    }
}

fn unique_temp_path(name: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ovim_lsp_resource_{}_{}", id, name))
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

#[test]
fn apply_workspace_resource_create_is_undoable() {
    let anchor = unique_temp_path("anchor_create.rs");
    std::fs::write(&anchor, "anchor\n").unwrap();
    let anchor_str = anchor.to_string_lossy().to_string();

    let created = unique_temp_path("created.txt");
    let _ = std::fs::remove_file(&created);
    let created_str = created.to_string_lossy().to_string();

    editor_flow_test! {
        content "anchor\n";
        setup |test| {
            test.set_file_path(anchor_str.clone());
            let uri = ovim::lsp::uri_from_file_path(&created_str).expect("uri_from_file_path failed");
            let we = workspace_edit_for_resource_op(lsp_types::ResourceOp::Create(lsp_types::CreateFile {
                uri,
                options: None,
                annotation_id: None,
            }));
            test.editor.apply_workspace_edit(we).unwrap();
            assert!(created.exists());
        }
        step "u" => |_test| {
            assert!(!created.exists());
        }
        step "<C-r>" => |_test| {
            assert!(created.exists());
            let _ = std::fs::remove_file(&created);
            let _ = std::fs::remove_file(&anchor);
        }
    }
}

#[test]
fn apply_workspace_resource_rename_is_undoable() {
    let anchor = unique_temp_path("anchor_rename.rs");
    std::fs::write(&anchor, "anchor\n").unwrap();
    let anchor_str = anchor.to_string_lossy().to_string();

    let old_path = unique_temp_path("rename_old.txt");
    let new_path = unique_temp_path("rename_new.txt");
    let _ = std::fs::remove_file(&old_path);
    let _ = std::fs::remove_file(&new_path);
    std::fs::write(&old_path, "rename-me\n").unwrap();
    let old_str = old_path.to_string_lossy().to_string();
    let new_str = new_path.to_string_lossy().to_string();

    editor_flow_test! {
        content "anchor\n";
        setup |test| {
            test.set_file_path(anchor_str.clone());
            let old_uri = ovim::lsp::uri_from_file_path(&old_str).expect("uri_from_file_path failed");
            let new_uri = ovim::lsp::uri_from_file_path(&new_str).expect("uri_from_file_path failed");
            let we = workspace_edit_for_resource_op(lsp_types::ResourceOp::Rename(lsp_types::RenameFile {
                old_uri,
                new_uri,
                options: None,
                annotation_id: None,
            }));
            test.editor.apply_workspace_edit(we).unwrap();
            assert!(!old_path.exists());
            assert_eq!(std::fs::read_to_string(&new_path).unwrap(), "rename-me\n");
        }
        step "u" => |_test| {
            assert_eq!(std::fs::read_to_string(&old_path).unwrap(), "rename-me\n");
            assert!(!new_path.exists());
        }
        step "<C-r>" => |_test| {
            assert!(!old_path.exists());
            assert_eq!(std::fs::read_to_string(&new_path).unwrap(), "rename-me\n");
            let _ = std::fs::remove_file(&new_path);
            let _ = std::fs::remove_file(&anchor);
        }
    }
}

#[test]
fn apply_workspace_resource_delete_is_undoable() {
    let anchor = unique_temp_path("anchor_delete.rs");
    std::fs::write(&anchor, "anchor\n").unwrap();
    let anchor_str = anchor.to_string_lossy().to_string();

    let deleted_path = unique_temp_path("delete_target.txt");
    let _ = std::fs::remove_file(&deleted_path);
    std::fs::write(&deleted_path, "delete-me\n").unwrap();
    let deleted_str = deleted_path.to_string_lossy().to_string();

    editor_flow_test! {
        content "anchor\n";
        setup |test| {
            test.set_file_path(anchor_str.clone());
            let uri = ovim::lsp::uri_from_file_path(&deleted_str).expect("uri_from_file_path failed");
            let we = workspace_edit_for_resource_op(lsp_types::ResourceOp::Delete(lsp_types::DeleteFile {
                uri,
                options: None,
            }));
            test.editor.apply_workspace_edit(we).unwrap();
            assert!(!deleted_path.exists());
        }
        step "u" => |_test| {
            assert_eq!(std::fs::read_to_string(&deleted_path).unwrap(), "delete-me\n");
        }
        step "<C-r>" => |_test| {
            assert!(!deleted_path.exists());
            let _ = std::fs::remove_file(&deleted_path);
            let _ = std::fs::remove_file(&anchor);
        }
    }
}
