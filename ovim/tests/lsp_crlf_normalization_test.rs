//! OV-00251: LSP servers running on Windows (or returning text from CRLF
//! source files) ship `\r\n` in `TextEdit.newText`. The internal rope is
//! LF-only by convention — verify that the seams normalize at insert time
//! so no `^M` artifacts survive.

use lsp_types::{Position, Range, TextEdit, WorkspaceEdit};

mod helpers;
use helpers::EditorTest;

// Uri's interior mutability is an internal cache; it doesn't affect Hash/Eq.
#[allow(clippy::mutable_key_type)]
fn workspace_edit_for_path(path: &str, edit: TextEdit) -> WorkspaceEdit {
    let mut changes = std::collections::HashMap::new();
    let uri = ovim::lsp::uri_from_file_path(path).expect("uri_from_file_path");
    changes.insert(uri, vec![edit]);
    WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    }
}

#[test]
fn apply_workspace_edit_strips_crlf_from_new_text() {
    // NamedTempFile must outlive the test — `find_buffer_by_path`
    // canonicalizes, which fails on a deleted file and triggers an LSP
    // load path that needs a Tokio runtime.
    let _tmp_guard = tempfile::Builder::new()
        .prefix("ovim_lsp_crlf_")
        .suffix(".rs")
        .tempfile()
        .unwrap();
    std::fs::write(_tmp_guard.path(), "seed\n").unwrap();
    let path = std::fs::canonicalize(_tmp_guard.path()).unwrap();
    let path = path.to_string_lossy().to_string();

    let mut t = EditorTest::new("seed\n");
    t.set_file_path(path.clone());

    // Replace "seed" (chars 0..4 on line 0) with "first\r\nsecond" — the
    // sort of payload a Windows-hosted LSP server might send for a
    // multi-line snippet.
    let edit = TextEdit {
        range: Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 4,
            },
        },
        new_text: "first\r\nsecond".to_string(),
    };

    t.editor
        .apply_workspace_edit(workspace_edit_for_path(&path, edit))
        .unwrap();

    let content = t.editor.buffer().rope().to_string();
    assert!(
        !content.contains('\r'),
        "rope should be LF-only after LSP edit, got: {content:?}"
    );
    assert!(content.starts_with("first\nsecond"));
}

#[test]
fn apply_workspace_edit_strips_bare_cr_from_new_text() {
    let _tmp_guard = tempfile::Builder::new()
        .prefix("ovim_lsp_crlf_bare_")
        .suffix(".rs")
        .tempfile()
        .unwrap();
    std::fs::write(_tmp_guard.path(), "seed\n").unwrap();
    let path = std::fs::canonicalize(_tmp_guard.path()).unwrap();
    let path = path.to_string_lossy().to_string();

    let mut t = EditorTest::new("seed\n");
    t.set_file_path(path.clone());

    let edit = TextEdit {
        range: Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 4,
            },
        },
        new_text: "a\rb\rc".to_string(),
    };

    t.editor
        .apply_workspace_edit(workspace_edit_for_path(&path, edit))
        .unwrap();

    let content = t.editor.buffer().rope().to_string();
    assert!(!content.contains('\r'));
    assert!(content.starts_with("a\nb\nc"));
}

#[test]
fn apply_workspace_edit_lf_only_text_is_unchanged() {
    // Sanity check: the no-CR fast path doesn't disturb the existing LSP
    // edit semantics.
    let _tmp_guard = tempfile::Builder::new()
        .prefix("ovim_lsp_crlf_lf_")
        .suffix(".rs")
        .tempfile()
        .unwrap();
    std::fs::write(_tmp_guard.path(), "hello\n").unwrap();
    let path = std::fs::canonicalize(_tmp_guard.path()).unwrap();
    let path = path.to_string_lossy().to_string();

    let mut t = EditorTest::new("hello\n");
    t.set_file_path(path.clone());

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

    t.editor
        .apply_workspace_edit(workspace_edit_for_path(&path, edit))
        .unwrap();

    assert_eq!(t.editor.buffer().line_text(0).unwrap(), "world");
}
