#![cfg(feature = "lua")]

use ovim::editor::{Editor, InputHandler};
use ovim::mode::Mode;
use ovim_core::{KeyCode, KeyEvent, Modifiers};

#[test]
fn test_lua_basic_execution() {
    let mut editor = Editor::new();

    // Enable Lua support
    editor.enable_lua().expect("Failed to enable Lua");

    // Execute basic Lua code
    let result = editor
        .execute_lua("return 2 + 2")
        .expect("Failed to execute Lua");
    assert_eq!(result, "4");

    // Test string return
    let result = editor
        .execute_lua("return 'hello'")
        .expect("Failed to execute Lua");
    assert_eq!(result, "hello");
}

#[test]
fn test_vim_fn_line() {
    let mut editor = Editor::with_content("line 1\nline 2\nline 3");
    editor.enable_lua().expect("Failed to enable Lua");

    // Cursor should be at line 0 (1-indexed for Lua)
    let result = editor
        .execute_lua("return vim.fn.line('.')")
        .expect("Failed to execute");
    assert_eq!(result, "1");
}

#[test]
fn test_vim_fn_col() {
    let mut editor = Editor::with_content("hello world");
    editor.enable_lua().expect("Failed to enable Lua");

    // Cursor should be at column 0 (1-indexed for Lua)
    let result = editor
        .execute_lua("return vim.fn.col('.')")
        .expect("Failed to execute");
    assert_eq!(result, "1");
}

#[test]
fn test_vim_api_get_current_line() {
    let mut editor = Editor::with_content("hello world\nline 2");
    editor.enable_lua().expect("Failed to enable Lua");

    // Get current line
    let result = editor
        .execute_lua("return vim.api.nvim_get_current_line()")
        .expect("Failed to execute");
    assert_eq!(result, "hello world");
}

#[test]
fn test_vim_cmd_queues_command() {
    let mut editor = Editor::with_content("test");
    editor.enable_lua().expect("Failed to enable Lua");

    // Queue a command (it won't execute immediately)
    editor
        .execute_lua("vim.cmd('nohl')")
        .expect("Failed to execute Lua");

    // Process the queued commands
    editor
        .process_lua_commands()
        .expect("Failed to process commands");

    // The command should have been executed (nohl clears search highlight)
    // This is a simple smoke test - we're just verifying no errors
}

#[test]
fn test_multiple_lua_calls() {
    let mut editor = Editor::with_content("line 1\nline 2\nline 3");
    editor.enable_lua().expect("Failed to enable Lua");

    // Multiple Lua calls
    let r1 = editor
        .execute_lua("return vim.fn.line('.')")
        .expect("Failed");
    let r2 = editor
        .execute_lua("return vim.fn.line('$')")
        .expect("Failed");

    assert_eq!(r1, "1"); // Current line
    assert_eq!(r2, "3"); // Last line
}

#[test]
fn test_lua_table_creation() {
    let mut editor = Editor::new();
    editor.enable_lua().expect("Failed to enable Lua");

    // Test that Lua can create tables
    let result = editor
        .execute_lua("local t = {1, 2, 3}; return t[1]")
        .expect("Failed");
    assert_eq!(result, "1");
}

#[test]
fn test_vim_namespace_exists() {
    let mut editor = Editor::new();
    editor.enable_lua().expect("Failed to enable Lua");

    // Test that vim namespace exists
    let result = editor.execute_lua("return vim ~= nil").expect("Failed");
    assert_eq!(result, "true");

    // Test vim.api exists
    let result = editor.execute_lua("return vim.api ~= nil").expect("Failed");
    assert_eq!(result, "true");

    // Test vim.fn exists
    let result = editor.execute_lua("return vim.fn ~= nil").expect("Failed");
    assert_eq!(result, "true");

    // Test vim.cmd exists
    let result = editor.execute_lua("return vim.cmd ~= nil").expect("Failed");
    assert_eq!(result, "true");
}

#[test]
fn test_vim_keymap_set_exists() {
    let mut editor = Editor::new();
    editor.enable_lua().expect("Failed to enable Lua");

    let result = editor
        .execute_lua("return vim.keymap ~= nil and vim.keymap.set ~= nil")
        .expect("Failed");
    assert_eq!(result, "true");
}

#[test]
fn test_vim_keymap_set_normal_mode_mapping() {
    let mut editor = Editor::with_content("abc");
    editor.enable_lua().expect("Failed to enable Lua");

    editor
        .execute_lua("vim.keymap.set('n', 'Q', 'x')")
        .expect("Failed to execute Lua");
    editor
        .process_lua_commands()
        .expect("Failed to process commands");

    InputHandler::handle_key_event(
        &mut editor,
        KeyEvent::new(KeyCode::Char('Q'), Modifiers::NONE),
    )
    .expect("Failed to handle key");

    let line = editor.buffer().line_text(0).unwrap_or_default();
    assert_eq!(line.trim_end_matches('\n'), "bc");
}

#[test]
fn test_vim_keymap_set_insert_mode_mapping() {
    let mut editor = Editor::with_content("abc");
    editor.enable_lua().expect("Failed to enable Lua");

    editor
        .execute_lua("vim.keymap.set('i', 'jk', '<Esc>')")
        .expect("Failed to execute Lua");
    editor
        .process_lua_commands()
        .expect("Failed to process commands");

    InputHandler::handle_key_event(
        &mut editor,
        KeyEvent::new(KeyCode::Char('i'), Modifiers::NONE),
    )
    .expect("Failed to enter insert mode");
    InputHandler::handle_key_event(
        &mut editor,
        KeyEvent::new(KeyCode::Char('j'), Modifiers::NONE),
    )
    .expect("Failed to handle first mapped key");
    InputHandler::handle_key_event(
        &mut editor,
        KeyEvent::new(KeyCode::Char('k'), Modifiers::NONE),
    )
    .expect("Failed to handle second mapped key");

    assert_eq!(editor.mode(), Mode::Normal);
    let line = editor.buffer().line_text(0).unwrap_or_default();
    assert_eq!(line.trim_end_matches('\n'), "abc");
}
