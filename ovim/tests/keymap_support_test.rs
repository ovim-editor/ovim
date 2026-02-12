mod helpers;

use helpers::EditorTest;
use ovim::editor::{InputHandler, LspAction};

#[test]
fn test_normal_mode_mapping_executes_rhs() {
    let mut test = EditorTest::new("abc\n");

    InputHandler::execute_command_string(&mut test.editor, "nnoremap jk x").unwrap();
    test.keys("jk");

    assert_eq!(test.buffer_content(), "bc\n");
}

#[test]
fn test_mapping_prefix_fallback_replays_original_keys() {
    let mut test = EditorTest::new("aaa\nbcd\n");

    InputHandler::execute_command_string(&mut test.editor, "nnoremap jk x").unwrap();
    test.keys("jl");

    test.assert_cursor(1, 1);
    assert_eq!(test.buffer_content(), "aaa\nbcd\n");
}

#[test]
fn test_recursive_mapping_remaps_rhs() {
    let mut test = EditorTest::new("abc\n");

    InputHandler::execute_command_string(&mut test.editor, "nmap a b").unwrap();
    InputHandler::execute_command_string(&mut test.editor, "nmap b x").unwrap();

    test.keys("a");

    assert_eq!(test.buffer_content(), "bc\n");
}

#[test]
fn test_noremap_does_not_remap_rhs() {
    let mut test = EditorTest::new("abc\n");

    InputHandler::execute_command_string(&mut test.editor, "nnoremap a b").unwrap();
    InputHandler::execute_command_string(&mut test.editor, "nmap b x").unwrap();

    test.keys("a");

    test.assert_cursor(0, 0);
    assert_eq!(test.buffer_content(), "abc\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_leader_save_mapping_writes_file() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    let mut test = EditorTest::new("changed\n");
    test.set_file_path(path.clone());

    InputHandler::execute_command_string(&mut test.editor, "nnoremap <leader>w :w<CR>").unwrap();
    test.keys("<Space>w");

    let saved = std::fs::read_to_string(path).unwrap();
    assert_eq!(saved, "changed\n");
}

#[test]
fn test_leader_format_mapping_queues_format_action() {
    let mut test = EditorTest::new("fn main() {}\n");

    InputHandler::execute_command_string(&mut test.editor, "nnoremap <leader>f gq").unwrap();
    test.keys("<Space>f");

    assert_eq!(
        test.editor.pending_lsp_action(),
        Some(&LspAction::FormatDocument)
    );
}
