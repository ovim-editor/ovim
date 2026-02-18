//! Integration tests for shell command features
//!
//! Tests for:
//! - :!cmd - run shell command and display output
//! - :.!cmd - replace current line with command output
//! - :%!cmd - pipe buffer through command
//! - :r !cmd - insert command output
//! - :w !cmd - write buffer to command stdin
//! - % and # expansion in shell commands

mod helpers;

use helpers::EditorTest;
use ovim::editor::InputHandler;

#[test]
fn test_shell_command_echo() {
    let mut test = EditorTest::new("hello world\n");

    // Execute :!echo test
    InputHandler::execute_command_string(&mut test.editor, "!echo test").unwrap();

    // Status should contain "test"
    let status = test.editor.lsp_status();
    assert!(
        status.contains("test"),
        "Status should contain 'test', got: {}",
        status
    );
}

#[test]
fn test_filter_current_line() {
    let mut test = EditorTest::new("hello world\nfoo bar\nbaz qux\n");
    test.editor.buffer_mut().cursor_mut().set_position(1, 0); // Middle line

    // Execute :.!tr 'a-z' 'A-Z' to uppercase current line
    InputHandler::execute_command_string(&mut test.editor, ".!tr 'a-z' 'A-Z'").unwrap();

    // Line 1 should be uppercased
    let line = test.editor.buffer().line(1).unwrap();
    assert_eq!(
        line.trim(),
        "FOO BAR",
        "Line should be uppercased, got: {}",
        line
    );
}

#[test]
fn test_filter_entire_buffer() {
    let mut test = EditorTest::new("cherry\napple\nbanana\n");

    // Execute :%!sort to sort all lines
    InputHandler::execute_command_string(&mut test.editor, "%!sort").unwrap();

    // Buffer should be sorted
    let content = test.editor.buffer().rope().to_string();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines[0], "apple", "First line should be apple");
    assert_eq!(lines[1], "banana", "Second line should be banana");
    assert_eq!(lines[2], "cherry", "Third line should be cherry");
}

#[test]
fn test_filter_entire_buffer_undo_redo_macro_flow() {
    editor_flow_test! {
        content "cherry\napple\nbanana\n";
        step ":%!sort<Enter>" => |test| {
            assert_eq!(test.buffer_content(), "apple\nbanana\ncherry\n");
        }
        step "u" => |test| {
            assert_eq!(test.buffer_content(), "cherry\napple\nbanana\n");
        }
        step "<C-r>" => |test| {
            assert_eq!(test.buffer_content(), "apple\nbanana\ncherry\n");
        }
    }
}

#[test]
fn test_read_shell_command() {
    let mut test = EditorTest::new("first line\nsecond line\n");
    test.editor.buffer_mut().cursor_mut().set_position(0, 0); // First line

    // Execute :r !echo "inserted"
    InputHandler::execute_command_string(&mut test.editor, "r !echo inserted").unwrap();

    // "inserted" should be somewhere in the buffer (after current line)
    let content = test.editor.buffer().rope().to_string();
    assert!(
        content.contains("inserted"),
        "Buffer should contain 'inserted', got: {}",
        content
    );
}

#[test]
#[ignore = "requires tokio runtime for buffer operations"]
fn test_write_to_shell_command() {
    let mut test = EditorTest::new("hello world\n");

    // Execute :w !cat (should succeed and show line count)
    InputHandler::execute_command_string(&mut test.editor, "w !cat").unwrap();

    // Status should mention lines written
    let status = test.editor.lsp_status();
    assert!(
        status.contains("written") || status.contains("line"),
        "Status should mention lines written, got: {}",
        status
    );
}

#[test]
fn test_percent_expansion_in_shell() {
    let mut test = EditorTest::new("content\n");

    // Set file path
    test.editor
        .buffer_mut()
        .set_file_path("test_file.rs".to_string());

    // Execute :!echo % (should expand to filename)
    InputHandler::execute_command_string(&mut test.editor, "!echo %").unwrap();

    // Status should contain the filename
    let status = test.editor.lsp_status();
    assert!(
        status.contains("test_file.rs"),
        "Status should contain filename, got: {}",
        status
    );
}

#[test]
fn test_percent_tail_modifier() {
    let mut test = EditorTest::new("content\n");

    // Set file path with directory
    test.editor
        .buffer_mut()
        .set_file_path("src/main.rs".to_string());

    // Test :t (tail/basename)
    InputHandler::execute_command_string(&mut test.editor, "!echo %:t").unwrap();
    let status = test.editor.lsp_status();
    assert!(
        status.contains("main.rs"),
        "Tail should be main.rs, got: {}",
        status
    );
}

#[test]
fn test_percent_head_modifier() {
    let mut test = EditorTest::new("content\n");

    // Set file path with directory
    test.editor
        .buffer_mut()
        .set_file_path("src/main.rs".to_string());

    // Test :h (head/directory)
    InputHandler::execute_command_string(&mut test.editor, "!echo %:h").unwrap();
    let status = test.editor.lsp_status();
    assert!(
        status.contains("src"),
        "Head should be src, got: {}",
        status
    );
}

#[test]
fn test_percent_root_modifier() {
    let mut test = EditorTest::new("content\n");

    // Set file path with directory
    test.editor
        .buffer_mut()
        .set_file_path("src/main.rs".to_string());

    // Test :r (root/no extension)
    InputHandler::execute_command_string(&mut test.editor, "!echo %:r").unwrap();
    let status = test.editor.lsp_status();
    assert!(
        status.contains("src/main"),
        "Root should be src/main, got: {}",
        status
    );
}

#[test]
fn test_percent_extension_modifier() {
    let mut test = EditorTest::new("content\n");

    // Set file path with directory
    test.editor
        .buffer_mut()
        .set_file_path("src/main.rs".to_string());

    // Test :e (extension)
    InputHandler::execute_command_string(&mut test.editor, "!echo %:e").unwrap();
    let status = test.editor.lsp_status();
    assert!(
        status.contains("rs"),
        "Extension should be rs, got: {}",
        status
    );
}

#[test]
fn test_escaped_percent() {
    let mut test = EditorTest::new("content\n");

    // Set file path
    test.editor
        .buffer_mut()
        .set_file_path("test.rs".to_string());

    // Execute :!echo \% (should show literal %)
    InputHandler::execute_command_string(&mut test.editor, r"!echo \%").unwrap();

    // Status should contain literal %
    let status = test.editor.lsp_status();
    assert!(
        status.contains("%"),
        "Status should contain literal %, got: {}",
        status
    );
    assert!(
        !status.contains("test.rs"),
        "Status should NOT contain filename, got: {}",
        status
    );
}

#[test]
fn test_chained_modifiers() {
    let mut test = EditorTest::new("content\n");

    // Set file path with nested directories
    test.editor
        .buffer_mut()
        .set_file_path("src/editor/main.rs".to_string());

    // Test :t:r (tail then root = "main")
    InputHandler::execute_command_string(&mut test.editor, "!echo %:t:r").unwrap();
    let status = test.editor.lsp_status();
    assert!(
        status.contains("main"),
        "Tail+root should be main, got: {}",
        status
    );
    assert!(
        !status.contains(".rs"),
        "Should not contain .rs, got: {}",
        status
    );
}

#[test]
#[ignore = "requires tokio runtime for file operations"]
fn test_edit_force_reload() {
    // Create a temp file
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join("ovim_shell_test_reload.txt");
    std::fs::write(&temp_file, "original content\n").unwrap();

    let mut test = EditorTest::new("placeholder\n");

    // Load the file
    test.editor.load_file(temp_file.to_str().unwrap()).unwrap();

    // Verify content
    let content = test.editor.buffer().rope().to_string();
    assert!(content.contains("original"), "Should have original content");

    // Modify the buffer
    test.editor.buffer_mut().insert_text_at(0, 0, "MODIFIED ");
    assert!(test.editor.buffer().is_modified());

    // Execute :e! to force reload
    InputHandler::execute_command_string(&mut test.editor, "e!").unwrap();

    // Buffer should be back to original
    let content = test.editor.buffer().rope().to_string();
    assert!(
        content.contains("original content"),
        "Buffer should contain original content, got: {}",
        content
    );
    assert!(
        !content.contains("MODIFIED"),
        "Buffer should not contain MODIFIED, got: {}",
        content
    );

    // Clean up
    std::fs::remove_file(temp_file).ok();
}
