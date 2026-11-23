//! Edits that do not come from typing (an AI tool, an LSP result, an API
//! request) can land while Insert mode is recording what the user types.
//! Each one must become its own undo step, and the typing around it must
//! survive.
use super::ai_chat_tools::ToolDispatchOutcome;
use super::input::InputHandler;
use super::Editor;
use crate::ai::chat_types::{ChatOpts, ToolCallInfo};
use crate::ai::tools::ToolResult;
use crate::mode::Mode;
use crate::{KeyCode, KeyEvent, Modifiers};

fn keys(editor: &mut Editor, sequence: &str) {
    for ch in sequence.chars() {
        let event = KeyEvent::new(KeyCode::Char(ch), Modifiers::NONE);
        InputHandler::handle_key_event(editor, event).unwrap();
    }
}

fn escape(editor: &mut Editor) {
    let event = KeyEvent::new(KeyCode::Esc, Modifiers::NONE);
    InputHandler::handle_key_event(editor, event).unwrap();
}

fn text(editor: &Editor) -> String {
    editor.buffer().rope().to_string()
}

/// Opens `content` as a file with an editable AI chat on it, then returns
/// focus to the editor in Normal mode.
fn editor_with_editable_chat(dir: &tempfile::TempDir, content: &str) -> Editor {
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, content).unwrap();
    let mut editor = Editor::default();
    editor.open_file(&file).unwrap();
    editor
        .open_ai_chat(ChatOpts {
            name: "chat".to_string(),
            allow_edits: true,
            ..Default::default()
        })
        .unwrap();
    editor.set_mode(Mode::Normal);
    editor
}

fn agent_replaces_line(editor: &mut Editor, line: usize, new_text: &str) {
    let call = ToolCallInfo {
        id: "call_edit".to_string(),
        name: "edit_range".to_string(),
        arguments: serde_json::json!({
            "start_line": line,
            "end_line": line,
            "new_text": new_text,
            "expected_revision": editor.buffer().version(),
        }),
    };
    match editor.dispatch_tool_call_with_approval(&call, None) {
        ToolDispatchOutcome::Completed(ToolResult::Success(_)) => {}
        ToolDispatchOutcome::Completed(ToolResult::Error(error)) => panic!("{error}"),
        ToolDispatchOutcome::ApprovalRequired(request) => panic!("{}", request.message),
    }
}

#[test]
fn an_agent_edit_while_typing_is_its_own_undo_step() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut editor = editor_with_editable_chat(&dir, "one\ntwo\n");

        keys(&mut editor, "A x");
        agent_replaces_line(&mut editor, 2, "TWO\n");
        assert_eq!(editor.mode(), Mode::Insert);
        keys(&mut editor, " y");
        escape(&mut editor);
        assert_eq!(text(&editor), "one x y\nTWO\n");

        // Like `<C-g>u` in Vim: the edit splits the typing into two undo steps.
        keys(&mut editor, "u");
        assert_eq!(text(&editor), "one x\nTWO\n");
        keys(&mut editor, "u");
        assert_eq!(text(&editor), "one x\ntwo\n");
        keys(&mut editor, "u");
        assert_eq!(text(&editor), "one\ntwo\n");
    });
}

/// Replaces `line` (0-based) with `new_text`, as an LSP `TextEdit`.
fn replace_line(line: u32, old_len: u32, new_text: &str) -> lsp_types::TextEdit {
    lsp_types::TextEdit {
        range: lsp_types::Range {
            start: lsp_types::Position { line, character: 0 },
            end: lsp_types::Position {
                line,
                character: old_len,
            },
        },
        new_text: new_text.to_string(),
    }
}

#[test]
fn a_format_result_that_lands_in_insert_mode_is_its_own_undo_step() {
    let mut editor = Editor::with_content("one\ntwo\n");

    keys(&mut editor, "A");
    editor.apply_lsp_edits(vec![replace_line(1, 3, "TWO")]);
    assert_eq!(editor.mode(), Mode::Insert);
    keys(&mut editor, " x");
    escape(&mut editor);
    assert_eq!(text(&editor), "one x\nTWO\n");

    keys(&mut editor, "u");
    assert_eq!(text(&editor), "one\nTWO\n");
    keys(&mut editor, "u");
    assert_eq!(text(&editor), "one\ntwo\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_server_edit_while_typing_is_its_own_undo_step() {
    use std::str::FromStr;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Main.java");
    std::fs::write(&file, "one\ntwo\n").unwrap();
    let mut editor = Editor::default();
    editor.open_file(&file).unwrap();
    let uri = lsp_types::Uri::from_str(&format!(
        "file://{}",
        file.canonicalize().unwrap().to_string_lossy()
    ))
    .unwrap();
    #[allow(clippy::mutable_key_type)]
    let changes = std::collections::HashMap::from([(uri, vec![replace_line(1, 3, "TWO")])]);

    keys(&mut editor, "A x");
    let response = editor.apply_workspace_edit_reporting(lsp_types::WorkspaceEdit {
        changes: Some(changes),
        ..Default::default()
    });
    assert!(response.applied, "{response:?}");
    keys(&mut editor, " y");
    escape(&mut editor);
    assert_eq!(text(&editor), "one x y\nTWO\n");

    keys(&mut editor, "u");
    assert_eq!(text(&editor), "one x\nTWO\n");
    keys(&mut editor, "u");
    assert_eq!(text(&editor), "one x\ntwo\n");
    keys(&mut editor, "u");
    assert_eq!(text(&editor), "one\ntwo\n");
}

/// The GUI's Cmd-Z runs `:undo` while Insert mode stays on, like `<C-o>u`.
/// In `nvim --clean`, `itwo <C-o>:undo<CR>three <Esc>` on "one" leaves
/// "three one", and `u` then leaves "one".
#[test]
fn undo_from_the_gui_in_insert_mode_removes_the_typing_so_far() {
    let mut editor = Editor::with_content("one\n");

    keys(&mut editor, "itwo ");
    let result = crate::commands::execute_command_api(&mut editor, "undo");
    assert!(
        !matches!(result, crate::command_result::CommandResult::Error(_)),
        "{result:?}"
    );
    assert_eq!(text(&editor), "one\n");
    assert_eq!(editor.mode(), Mode::Insert);
    keys(&mut editor, "three ");
    escape(&mut editor);
    assert_eq!(text(&editor), "three one\n");

    keys(&mut editor, "u");
    assert_eq!(text(&editor), "one\n");
}

/// The GUI switches tabs on a click without leaving Insert mode.
#[test]
fn typing_across_a_tab_switch_gives_each_buffer_its_own_undo() {
    let mut editor = Editor::with_content("one\n");
    let _ = crate::commands::execute_command(&mut editor, "tabnew");
    keys(&mut editor, "ifirst");
    escape(&mut editor);
    keys(&mut editor, "0");
    editor.goto_tab(0);

    keys(&mut editor, "A x");
    editor.goto_tab(1);
    assert_eq!(editor.mode(), Mode::Insert);
    keys(&mut editor, "y ");
    escape(&mut editor);
    assert_eq!(text(&editor), "y first");
    keys(&mut editor, "u");
    assert_eq!(text(&editor), "first");

    editor.goto_tab(0);
    assert_eq!(text(&editor), "one x\n");
    keys(&mut editor, "u");
    assert_eq!(text(&editor), "one\n");
}
