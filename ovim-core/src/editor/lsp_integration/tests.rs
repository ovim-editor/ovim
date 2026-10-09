use super::document_sync::DocumentSyncRequestAction;
use super::*;
use crate::editor::lsp_slot::{CompletionResult, InlayHintResult};
use crate::editor::lsp_state::InlayHintRequestKey;
use crate::lsp::uri_from_file_path;
use lsp_types::{CompletionItem, InlayHint, InlayHintLabel, Location, Position, Range};
use std::sync::Arc;
use tokio::sync::oneshot;

fn location(path: &std::path::Path, line: u32, character: u32) -> Location {
    Location {
        uri: uri_from_file_path(path).unwrap(),
        range: Range::new(
            Position::new(line, character),
            Position::new(line, character),
        ),
    }
}

/// OV-00454: an LSP jump is a jump. `<C-o>` returns to where gd/gi/gr was
/// pressed (also across files) and `<C-i>` goes forward again.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn ctrl_o_and_ctrl_i_follow_lsp_jumps_across_files() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("A.java");
    let b = dir.path().join("B.java");
    std::fs::write(&a, "a0\na1\na2\na3\na4\n").unwrap();
    std::fs::write(&b, "b0\nb1\nb2\nb3\nb4\nb5\n").unwrap();
    let mut editor = Editor::default();
    editor.load_file(&a).unwrap();
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(2, crate::unicode::GraphemeCol(1));

    // gd from A:3 into B:5 (cross-file), then a second jump inside B.
    assert!(editor.handle_goto_location(Some(location(&b, 4, 0)), "Definition", "t", false));
    assert!(editor.handle_goto_location(Some(location(&b, 1, 0)), "Definition", "t", false));
    assert_eq!(editor.buffer().cursor().line(), 1);

    assert!(editor.jump_back());
    assert_eq!(
        editor.buffer().file_path().map(std::path::Path::new),
        Some(b.canonicalize().unwrap().as_path())
    );
    assert_eq!(editor.buffer().cursor().line(), 4);
    assert!(editor.jump_back());
    assert_eq!(
        editor.buffer().file_path().map(std::path::Path::new),
        Some(a.canonicalize().unwrap().as_path())
    );
    assert_eq!(
        (
            editor.buffer().cursor().line(),
            editor.buffer().cursor().col().0
        ),
        (2, 1)
    );
    assert!(!editor.jump_back(), "nothing older than the first jump");

    assert!(editor.jump_forward());
    assert_eq!(editor.buffer().cursor().line(), 4);
    assert!(editor.jump_forward());
    assert_eq!(editor.buffer().cursor().line(), 1);
    assert!(!editor.jump_forward());
}

/// The server keeps a document open while its buffer is loaded, so the
/// diagnostics of files the user switched away from stay available (the
/// Problems view). Deleting the buffer is what closes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn switching_buffers_keeps_documents_open_and_deleting_closes_them() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("A.java");
    let b = dir.path().join("B.java");
    std::fs::write(&a, "class A {}\n").unwrap();
    std::fs::write(&b, "class B {}\n").unwrap();
    let a = a.canonicalize().unwrap();
    let b = b.canonicalize().unwrap();
    let mut editor = Editor::default();
    editor.enable_lsp();
    editor.load_file(&a).unwrap();
    editor.load_file(&b).unwrap();
    for path in [&a, &b] {
        editor
            .lsp
            .state
            .document_sync
            .entry(path.to_string_lossy().to_string())
            .or_default();
    }
    let a_key = a.to_string_lossy().to_string();

    // load_file(B) queued a close for A, but A's buffer is still loaded.
    editor.send_lsp_close_if_needed().await;
    assert!(editor.lsp.state.document_sync.contains_key(&a_key));

    // Deleting A's buffer closes it.
    let index = editor
        .buffers
        .iter()
        .position(|buffer| buffer.file_path() == Some(a_key.as_str()))
        .unwrap();
    editor.switch_to_buffer(index);
    editor.delete_current_buffer();
    editor.send_lsp_close_if_needed().await;
    assert!(!editor.lsp.state.document_sync.contains_key(&a_key));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_handle_location_result_new_tab_updates_current_file_register() {
    let test_dir = tempfile::tempdir().expect("tempdir");

    let source = test_dir.path().join("source.rs");
    let target = test_dir.path().join("target.rs");

    std::fs::write(&source, "source\n").unwrap();
    std::fs::write(&target, "target\n").unwrap();

    let source_path = std::fs::canonicalize(&source)
        .unwrap()
        .to_string_lossy()
        .to_string();
    let target_path = std::fs::canonicalize(&target)
        .unwrap()
        .to_string_lossy()
        .to_string();

    let mut editor = Editor::with_content("source\n");
    editor.set_file_path(source_path);

    let uri = uri_from_file_path(&target).unwrap();
    let location = Location::new(uri, Range::new(Position::new(0, 0), Position::new(0, 0)));

    let handled =
        editor.handle_location_result(Ok(Some(location)), "Definition", "LSP-DEFINITION", true);
    assert!(handled);
    assert_eq!(editor.registers().get(Some('%')), target_path);
}

/// Ctrl-G on a definition in a file that is already open (with unsaved
/// changes) opens a tab on that buffer, not a second copy loaded from disk.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn definition_in_new_tab_reuses_an_open_buffer_with_its_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.rs");
    std::fs::write(&path, "fn symbol() {}\n").unwrap();
    let path = path.canonicalize().unwrap();
    let mut editor = Editor::new();
    editor.open_file(&path).unwrap();
    editor
        .buffer_mut()
        .insert_text_at(0, crate::unicode::CharCol(0), "// unsaved\n");
    let buffers = editor.buffers.len();
    let tabs = editor.tab_count();

    let location = Location::new(
        uri_from_file_path(&path).unwrap(),
        Range::new(Position::new(1, 3), Position::new(1, 9)),
    );
    assert!(editor.handle_location_result(Ok(Some(location)), "Definition", "LSP", true));

    assert_eq!(editor.tab_count(), tabs + 1);
    assert_eq!(editor.buffers.len(), buffers, "no second copy of the file");
    assert_eq!(
        editor.buffer().rope().to_string(),
        "// unsaved\nfn symbol() {}\n"
    );
    assert_eq!(editor.buffer().cursor().line(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn definition_quit_retraces_chain_until_manual_navigation_or_file_change() {
    use crate::commands::execute_command;
    let dir = tempfile::tempdir().unwrap();
    let paths: Vec<_> = ["a.rs", "b.rs", "c.rs", "other.rs"]
        .into_iter()
        .map(|name| {
            let path = dir.path().join(name);
            std::fs::write(&path, "fn symbol() {}\n").unwrap();
            path.canonicalize().unwrap()
        })
        .collect();
    let follow = |editor: &mut Editor, index: usize| {
        let location = Location::new(
            uri_from_file_path(&paths[index]).unwrap(),
            Range::new(Position::new(0, 0), Position::new(0, 0)),
        );
        assert!(editor.handle_location_result(
            Ok(Some(location)),
            "Definition",
            "LSP-DEFINITION",
            true
        ));
    };
    for action in ["chain", "manual", "file", "file_back", "rename"] {
        let mut editor = Editor::new();
        editor.open_file(&paths[0]).unwrap();
        editor.new_tab();
        editor.open_file(&paths[3]).unwrap();
        editor.goto_tab(0);
        let origin = editor.tab_page_manager.current_tab().id();
        follow(&mut editor, 1);
        follow(&mut editor, 2);
        match action {
            "manual" => {
                editor.previous_tab();
                editor.next_tab();
            }
            "file" => {
                editor.open_file(&paths[0]).unwrap();
            }
            "file_back" => {
                editor.open_file(&paths[0]).unwrap();
                editor.open_file(&paths[2]).unwrap();
            }
            "rename" => {
                editor.set_file_path(paths[0].to_string_lossy().into_owned());
            }
            _ => {}
        }
        execute_command(&mut editor, "q");
        assert!(!editor.should_quit());
        if action == "chain" {
            assert_eq!(editor.buffer().file_path(), paths[1].to_str());
            execute_command(&mut editor, "q");
            assert_eq!(editor.tab_page_manager.current_tab().id(), origin);
            assert_eq!(editor.buffer().file_path(), paths[0].to_str());
        } else {
            // Ordinary tab closing selects the tab on the right.
            assert_eq!(editor.buffer().file_path(), paths[3].to_str(), "{action}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn failed_definition_file_does_not_create_a_tab() {
    let mut editor = Editor::new();
    let dir = tempfile::tempdir().unwrap();
    let location = Location::new(
        uri_from_file_path(dir.path().join("missing.rs")).unwrap(),
        Range::new(Position::new(0, 0), Position::new(0, 0)),
    );
    assert!(!editor.handle_location_result(
        Ok(Some(location)),
        "Definition",
        "LSP-DEFINITION",
        true
    ));
    assert_eq!(editor.tab_count(), 1);
}

#[test]
fn document_sync_request_plan_flushes_already_queued_content() {
    let mut editor = Editor::with_content("class Test {}\n");
    let file_path = "/tmp/Test.java".to_string();
    editor.set_file_path(file_path.clone());

    let state = editor
        .lsp
        .state
        .document_sync
        .entry(file_path.clone())
        .or_default();
    state.did_open_sent = true;
    state.buffer_modified = true;
    state.last_flushed_content = Some(Arc::from("class Test {\n"));
    state.last_queued_content = Some(Arc::from("class Test {}\n"));
    state.target_lsp_version = Some(4);

    let plan = editor.document_sync_request_plan(&file_path, "class Test {}\n");
    assert_eq!(plan.action, DocumentSyncRequestAction::FlushQueued);
    assert!(plan.old_content.is_none());
}

#[test]
fn reconcile_document_sync_with_manager_promotes_flushed_queue() {
    let mut editor = Editor::with_content("class Test {}\n");
    let file_path = "/tmp/Test.java".to_string();
    editor.set_file_path(file_path.clone());

    let state = editor
        .lsp
        .state
        .document_sync
        .entry(file_path.clone())
        .or_default();
    state.did_open_sent = true;
    state.mark_change_queued(Arc::from("class Test {}\n"), 4);

    editor.reconcile_document_sync_with_manager(&file_path, Some("class Test {}\n"), 4, 4);

    let state = editor
        .lsp
        .state
        .document_sync
        .get(&file_path)
        .expect("document sync state");
    assert!(!state.buffer_modified);
    assert!(state.target_lsp_version.is_none());
    assert!(state.last_queued_content.is_none());
    assert_eq!(
        state.last_flushed_content.as_deref(),
        Some("class Test {}\n")
    );
}

#[test]
fn reconcile_document_sync_with_manager_keeps_dirty_flag_for_newer_buffer_content() {
    let mut editor = Editor::with_content("class Test { int value; }\n");
    let file_path = "/tmp/Test.java".to_string();
    editor.set_file_path(file_path.clone());

    let state = editor
        .lsp
        .state
        .document_sync
        .entry(file_path.clone())
        .or_default();
    state.did_open_sent = true;
    state.mark_change_queued(Arc::from("class Test {}\n"), 4);

    editor.reconcile_document_sync_with_manager(
        &file_path,
        Some("class Test { int value; }\n"),
        4,
        4,
    );

    let state = editor
        .lsp
        .state
        .document_sync
        .get(&file_path)
        .expect("document sync state");
    assert!(state.buffer_modified);
    assert!(state.target_lsp_version.is_none());
    assert_eq!(
        state.last_flushed_content.as_deref(),
        Some("class Test {}\n")
    );
}

/// Helper: fire a pre-built `InlayHintResult` into the inlay hints slot.
fn fire_inlay_hint_result(editor: &mut Editor, result: InlayHintResult) {
    let (tx, rx) = oneshot::channel::<anyhow::Result<InlayHintResult>>();
    tx.send(Ok(result)).unwrap();
    let task = tokio::spawn(async {});
    editor.lsp.slots.inlay_hints.fire(task, rx);
}

#[tokio::test(flavor = "current_thread")]
async fn poll_pending_inlay_hint_response_applies_latest_result() {
    let mut editor = Editor::with_content("class Test {}\n");
    let file_path = "/tmp/Test.java".to_string();
    editor.set_file_path(file_path.clone());
    editor.set_viewport_height(20);

    let request_key = InlayHintRequestKey {
        file_path: file_path.clone(),
        start_line: 0,
        end_line: 30,
        lsp_version: 4,
    };
    let hint = InlayHint {
        position: Position::new(0, 5),
        label: InlayHintLabel::String(": Test".to_string()),
        kind: None,
        text_edits: None,
        tooltip: None,
        padding_left: Some(true),
        padding_right: None,
        data: None,
    };

    let bv = editor.buffer().version();
    fire_inlay_hint_result(
        &mut editor,
        InlayHintResult {
            request_key: request_key.clone(),
            buffer_version: bv,
            synced_content: Some("class Test {}\n".to_string()),
            synced_lsp_version: Some(4),
            hints: vec![hint],
        },
    );

    assert!(editor.poll_pending_inlay_hint_response());
    assert_eq!(editor.lsp.state.current_file_lsp_version, 4);
    assert_eq!(editor.lsp.state.current_file_lsp_sent_version, 4);
    assert_eq!(editor.lsp.state.inlay_hints.len(), 1);
    // TrackedSlot: after a successful poll, the slot is no longer stale
    // (the result was applied for the generation that was current at fire time).
    assert!(!editor.lsp.slots.inlay_hints.is_stale());

    let sync_state = editor
        .lsp
        .state
        .document_sync
        .get(&file_path)
        .expect("document sync state");
    assert!(sync_state.did_open_sent);
    assert_eq!(
        sync_state.last_flushed_content.as_deref(),
        Some("class Test {}\n")
    );
}

/// OV-00470: a definition whose URI is not `file:` (a server's own
/// scheme) is fetched from the server and shown read-only and
/// unmodifiable, never opened as an editable file.
#[tokio::test(flavor = "current_thread")]
async fn virtual_document_opens_unmodifiable_at_the_definition() {
    let mut editor = Editor::with_content("class Caller {}\n");
    let document = crate::editor::lsp_slot::VirtualDocumentResult {
        uri: "jdt://contents/java.base/java.util/ArrayList.class"
            .parse()
            .unwrap(),
        text: "package java.util;\npublic class ArrayList {\n}\n".into(),
        range: lsp_types::Range {
            start: lsp_types::Position::new(1, 13),
            end: lsp_types::Position::new(1, 22),
        },
        origin: editor.request_origin(),
    };
    assert!(editor.open_virtual_document_result(Ok(document)));
    assert!(editor.buffer().is_read_only());
    assert!(!editor.buffer().is_modifiable());
    assert_eq!(editor.buffer().cursor().line(), 1);
    assert!(editor.buffer().rope().to_string().contains("ArrayList"));
    let before = editor.buffer().rope().to_string();
    for ch in "ixdd".chars() {
        let _ = crate::editor::InputHandler::handle_key_event(
            &mut editor,
            crate::KeyEvent::new(crate::KeyCode::Char(ch), crate::Modifiers::NONE),
        );
    }
    assert_eq!(editor.buffer().rope().to_string(), before);
}

#[tokio::test(flavor = "current_thread")]
async fn poll_pending_inlay_hint_response_drops_stale_buffer_result() {
    // OV-00258: when the buffer version has advanced since the LSP
    // request was spawned, the hint positions index into the wrong
    // rope. Render-skip is correct; the next tick re-requests against
    // the current version.
    let mut editor = Editor::with_content("class Test {}\n");
    let file_path = "/tmp/Test.java".to_string();
    editor.set_file_path(file_path.clone());
    editor.set_viewport_height(20);

    let request_key = InlayHintRequestKey {
        file_path: file_path.clone(),
        start_line: 0,
        end_line: 30,
        lsp_version: 4,
    };

    // Reply was prepared against a buffer version one ahead of the
    // current version — i.e., the user typed something between
    // request-spawn and reply-arrival.
    let stale_buffer_version = editor.buffer().version() + 1;
    let stale_hint = InlayHint {
        position: Position::new(0, 5),
        label: InlayHintLabel::String(": Stale".to_string()),
        kind: None,
        text_edits: None,
        tooltip: None,
        padding_left: Some(true),
        padding_right: None,
        data: None,
    };
    fire_inlay_hint_result(
        &mut editor,
        InlayHintResult {
            request_key: request_key.clone(),
            buffer_version: stale_buffer_version,
            synced_content: None,
            synced_lsp_version: None,
            hints: vec![stale_hint],
        },
    );

    // Drop the response — return value is `false` (no UI mutation).
    assert!(!editor.poll_pending_inlay_hint_response());

    // Cached hints and InlayHint decorations must remain empty —
    // we did not commit a mis-aligned render.
    assert!(
        editor.lsp.state.inlay_hints.is_empty(),
        "stale hints must not enter `lsp.state.inlay_hints`"
    );
    assert!(
        !editor
            .decorations
            .iter_all()
            .any(|(_, d)| d.source == crate::editor::decoration::DecorationSource::InlayHint),
        "no InlayHint decoration may be placed when the reply is stale"
    );

    // Slot is invalidated so the next event-loop tick re-requests.
    assert!(editor.lsp.slots.inlay_hints.is_stale());
}

#[tokio::test(flavor = "current_thread")]
async fn poll_pending_inlay_hint_response_drops_result_behind_current_sent_version() {
    let mut editor = Editor::with_content("class Test {}\n");
    let file_path = "/tmp/Test.java".to_string();
    editor.set_file_path(file_path.clone());
    editor.set_viewport_height(20);
    editor.lsp.state.current_file_lsp_sent_version = 5;

    let request_key = InlayHintRequestKey {
        file_path,
        start_line: 0,
        end_line: 30,
        lsp_version: 4,
    };

    let bv = editor.buffer().version();
    fire_inlay_hint_result(
        &mut editor,
        InlayHintResult {
            request_key: request_key.clone(),
            buffer_version: bv,
            synced_content: None,
            synced_lsp_version: None,
            hints: Vec::new(),
        },
    );

    assert!(!editor.poll_pending_inlay_hint_response());
    assert!(editor.lsp.state.inlay_hints.is_empty());
    // Slot is stale (invalidated) because the result was dropped.
    assert!(editor.lsp.slots.inlay_hints.is_stale());
}

/// Helper: fire a pre-built `CompletionResult` into the completion slot so
/// `poll_pending_completion_response` can pick it up immediately.
fn fire_completion_result(editor: &mut Editor, result: CompletionResult) {
    let (tx, rx) = oneshot::channel::<anyhow::Result<CompletionResult>>();
    tx.send(Ok(result)).unwrap();
    let task = tokio::spawn(async {});
    editor.lsp.slots.completion.fire(task, rx);
}

fn anchor_of(editor: &Editor) -> crate::editor::CompletionAnchor {
    let line = editor.buffer().cursor().line();
    crate::editor::CompletionAnchor {
        line,
        col: editor.buffer().cursor_char_col().0,
        line_text: editor
            .buffer()
            .line_text(line)
            .unwrap_or_default()
            .to_string(),
        line_count: editor.buffer().line_count(),
    }
}

fn completion_item(label: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        insert_text: Some(label.to_string()),
        ..Default::default()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn poll_pending_completion_response_shows_menu_on_fresh_result() {
    let mut editor = Editor::with_content("let x = fo");
    editor.set_file_path("/tmp/a.rs".to_string());
    editor.set_mode(crate::mode::Mode::Insert);

    // Read the EFFECTIVE path back: set_file_path canonicalizes when the
    // file happens to exist on the host (macOS /tmp → /private/tmp), and
    // the poll validates result.file_path against it by string equality.
    // Hard-coding "/tmp/a.rs" made this test depend on whether a stray
    // /tmp/a.rs existed on the machine.
    let effective_path = editor.buffer().file_path().unwrap().to_string();

    let bv = editor.buffer().version();
    let anchor = anchor_of(&editor);
    fire_completion_result(
        &mut editor,
        CompletionResult {
            items: vec![completion_item("foo")],
            is_incomplete: false,
            anchor,
            file_path: effective_path,
            buffer_version: bv,
            synced_content: None,
            synced_lsp_version: None,
            sources: Vec::new(),
        },
    );

    assert!(editor.poll_pending_completion_response());
    assert!(editor.completion_menu().is_visible());
    assert_eq!(editor.completion_menu().len(), 1);
}

/// OV-00456: an empty completion answer must not leave the
/// "Requesting completions..." status on screen.
#[tokio::test(flavor = "current_thread")]
async fn empty_completion_result_clears_the_requesting_status() {
    let mut editor = Editor::with_content("let x = fo");
    editor.set_file_path("/tmp/a.rs".to_string());
    editor.set_mode(crate::mode::Mode::Insert);
    let effective_path = editor.buffer().file_path().unwrap().to_string();
    let bv = editor.buffer().version();
    editor.set_lsp_status(lsp_modules::completion::REQUESTING_STATUS.to_string());
    let anchor = anchor_of(&editor);
    fire_completion_result(
        &mut editor,
        CompletionResult {
            items: Vec::new(),
            is_incomplete: false,
            anchor,
            file_path: effective_path,
            buffer_version: bv,
            synced_content: None,
            synced_lsp_version: None,
            sources: Vec::new(),
        },
    );
    editor.poll_pending_completion_response();
    assert_ne!(
        editor.lsp_status(),
        lsp_modules::completion::REQUESTING_STATUS
    );
}

#[tokio::test(flavor = "current_thread")]
async fn poll_pending_completion_response_drops_result_from_old_file() {
    let mut editor = Editor::with_content("let x = fo");
    editor.set_file_path("/tmp/b.rs".to_string());
    editor.set_mode(crate::mode::Mode::Insert);

    let bv = editor.buffer().version();
    // Response was fired for /tmp/a.rs but the user has since switched
    // to /tmp/b.rs. Without validation this would apply to the wrong file.
    let anchor = anchor_of(&editor);
    fire_completion_result(
        &mut editor,
        CompletionResult {
            items: vec![completion_item("foo")],
            is_incomplete: false,
            anchor,
            file_path: "/tmp/a.rs".to_string(),
            buffer_version: bv,
            synced_content: None,
            synced_lsp_version: None,
            sources: Vec::new(),
        },
    );

    assert!(!editor.poll_pending_completion_response());
    assert!(!editor.completion_menu().is_visible());
    assert!(editor.completion_menu().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn poll_pending_completion_response_drops_result_from_stale_buffer_version() {
    let mut editor = Editor::with_content("let x = fo");
    editor.set_file_path("/tmp/a.rs".to_string());
    editor.set_mode(crate::mode::Mode::Insert);

    // Simulate: request fired at version N, user kept typing (bumping
    // the buffer version), response arrived carrying the old N. Without
    // validation the stale items would populate the menu.
    let effective_path = editor.buffer().file_path().unwrap().to_string();
    let stale_version = editor.buffer().version();
    let stale_anchor = anchor_of(&editor);
    editor
        .buffer_mut()
        .insert_text_at(0, crate::unicode::CharCol(10), "o");
    assert!(editor.buffer().version() > stale_version);

    fire_completion_result(
        &mut editor,
        CompletionResult {
            items: vec![completion_item("foo")],
            is_incomplete: false,
            anchor: stale_anchor,
            file_path: effective_path,
            buffer_version: stale_version,
            synced_content: None,
            synced_lsp_version: None,
            sources: Vec::new(),
        },
    );

    assert!(!editor.poll_pending_completion_response());
    assert!(!editor.completion_menu().is_visible());
    assert!(editor.completion_menu().is_empty());
}

fn fire_format_result(editor: &mut Editor, result: crate::editor::lsp_slot::FormatResult) {
    let (tx, rx) = oneshot::channel::<anyhow::Result<crate::editor::lsp_slot::FormatResult>>();
    tx.send(Ok(result)).unwrap();
    let task = tokio::spawn(async {});
    editor.lsp.slots.format.fire(task, rx);
}

fn whole_buffer_replace_edit(new_text: &str, end_line: u32) -> lsp_types::TextEdit {
    lsp_types::TextEdit {
        range: Range::new(Position::new(0, 0), Position::new(end_line, 0)),
        new_text: new_text.to_string(),
    }
}

/// OV-00327: a format response computed against buffer version N must
/// not be applied after the user kept editing (version N+k) — the edits
/// would splice at stale offsets, reverting or garbling what was typed.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn stale_format_result_is_discarded() {
    let mut editor = Editor::with_content("fn main( ){}\n");
    editor.set_file_path("/tmp/fmt.rs".to_string());

    let stale_version = editor.buffer().version();
    let file_path = editor.buffer().file_path().unwrap().to_string();

    // User keeps typing while the format request is in flight.
    editor
        .buffer_mut()
        .insert_text_at(0, crate::unicode::CharCol(0), "// note\n");
    let typed_content = editor.buffer().rope().to_string();

    fire_format_result(
        &mut editor,
        crate::editor::lsp_slot::FormatResult {
            edits: vec![whole_buffer_replace_edit("fn main() {}\n", 1)],
            file_path,
            buffer_version: stale_version,
        },
    );

    editor.poll_action_slots();
    assert_eq!(
        editor.buffer().rope().to_string(),
        typed_content,
        "stale format edits must not be applied over newer typing"
    );
}

/// Companion: a format response for the current buffer version applies.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn current_format_result_applies() {
    let mut editor = Editor::with_content("fn main( ){}\n");
    editor.set_file_path("/tmp/fmt.rs".to_string());

    let file_path = editor.buffer().file_path().unwrap().to_string();
    let buffer_version = editor.buffer().version();
    fire_format_result(
        &mut editor,
        crate::editor::lsp_slot::FormatResult {
            edits: vec![whole_buffer_replace_edit("fn main() {}\n", 1)],
            file_path,
            buffer_version,
        },
    );

    editor.poll_action_slots();
    assert_eq!(editor.buffer().rope().to_string(), "fn main() {}\n");
}

/// OV-00327: accepting a completion item whose textEdit range targets an
/// older buffer version must fall back to trigger-prefix replacement
/// instead of splicing the stale range into the current text.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn stale_completion_accept_falls_back_to_prefix_replacement() {
    let mut editor = Editor::with_content("self.fo");
    editor.set_file_path("/tmp/a.rs".to_string());
    editor.set_mode(crate::mode::Mode::Insert);
    editor
        .buffer_mut()
        .set_cursor_char_col(0, crate::unicode::CharCol(7));

    // Server responded when the buffer was "self.fo": textEdit replaces
    // cols [5,7) ("fo") with "foo_bar()".
    let item = CompletionItem {
        label: "foo_bar".to_string(),
        insert_text: Some("foo_bar()".to_string()),
        text_edit: Some(lsp_types::CompletionTextEdit::Edit(lsp_types::TextEdit {
            range: Range::new(Position::new(0, 5), Position::new(0, 7)),
            new_text: "foo_bar()".to_string(),
        })),
        ..Default::default()
    };
    let response_version = editor.buffer().version();
    editor
        .completion_menu_mut()
        .show(vec![item.clone()], 5, "fo".to_string());
    editor
        .completion_menu_mut()
        .set_items_buffer_version(response_version);

    // User types one more char before accepting: buffer is "self.foo",
    // cursor at col 8. The stale range [5,7) no longer covers the
    // typed prefix — applying it verbatim used to produce
    // "self.foo_bar()o" (orphaned trailing char).
    editor
        .buffer_mut()
        .insert_text_at(0, crate::unicode::CharCol(7), "o");
    editor
        .buffer_mut()
        .set_cursor_char_col(0, crate::unicode::CharCol(8));
    assert!(editor.buffer().version() > response_version);
    // Typing refilters the menu, as the insert-mode key handler does.
    editor.completion_menu_mut().filter("foo");

    editor.accept_completion();

    assert_eq!(
        editor.buffer().rope().to_string(),
        "self.foo_bar()\n",
        "stale textEdit range must not leave orphaned typed characters"
    );
}

/// OV-00474: accepting a method completion whose snippet leaves the cursor
/// inside `name(|)` brings the parameter popup up at once; a completion
/// that ends after its `)` does not.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn accepting_a_method_snippet_requests_signature_help() {
    let mut editor = Editor::with_content("list.ad");
    editor.set_file_path("/tmp/a.java".to_string());
    editor.set_mode(crate::mode::Mode::Insert);
    editor
        .buffer_mut()
        .set_cursor_char_col(0, crate::unicode::CharCol(7));
    let method = CompletionItem {
        label: "add(E e)".to_string(),
        insert_text: Some("add(${1:e})".to_string()),
        insert_text_format: Some(lsp_types::InsertTextFormat::SNIPPET),
        ..Default::default()
    };
    editor
        .completion_menu_mut()
        .show(vec![method], 5, "ad".to_string());
    editor.accept_completion();
    assert_eq!(editor.buffer().rope().to_string(), "list.add(e)\n");
    assert!(
        editor.lsp.intents.signature_help,
        "cursor is inside add(...)"
    );

    let mut editor = Editor::with_content("list.si");
    editor.set_file_path("/tmp/a.java".to_string());
    editor.set_mode(crate::mode::Mode::Insert);
    editor
        .buffer_mut()
        .set_cursor_char_col(0, crate::unicode::CharCol(7));
    let finished = CompletionItem {
        label: "size()".to_string(),
        insert_text: Some("size()".to_string()),
        ..Default::default()
    };
    editor
        .completion_menu_mut()
        .show(vec![finished], 5, "si".to_string());
    editor.accept_completion();
    assert!(
        !editor.lsp.intents.signature_help,
        "the cursor is after `size()`, not inside a call"
    );
}
