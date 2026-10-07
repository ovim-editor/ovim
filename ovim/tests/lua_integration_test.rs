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

#[test]
fn pullbase_lua_options_set_and_clear_through_editor_commands() {
    let mut editor = Editor::new();
    editor.enable_lua().unwrap();
    for namespace in ["ovim", "vim"] {
        editor
            .execute_lua(&format!("{namespace}.opt.pullbase = 'release/stable'"))
            .unwrap();
        editor.process_lua_commands().unwrap();
        assert_eq!(editor.options.pullbase.as_deref(), Some("release/stable"));
        for invalid in ["true", "42", "'main..feature'", "'main\\nquit'"] {
            assert!(editor
                .execute_lua(&format!("{namespace}.opt.pullbase = {invalid}"))
                .is_err());
        }
        editor
            .execute_lua(&format!("{namespace}.opt.pullbase = nil"))
            .unwrap();
        editor.process_lua_commands().unwrap();
        assert_eq!(editor.options.pullbase, None);
    }
    assert_eq!(
        editor
            .execute_lua("return type(ovim.languages.register)")
            .unwrap(),
        "function"
    );
}

#[test]
fn pullbase_lua_path_overrides_support_spaces_and_clear_individually() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("project with spaces");
    std::fs::create_dir(&path).unwrap();
    let canonical = path.canonicalize().unwrap();
    let path_literal = format!("{:?}", path.to_str().unwrap());
    let mut editor = Editor::new();
    editor.enable_lua().unwrap();
    editor.execute_lua("ovim.opt.pullbase = 'main'").unwrap();
    for namespace in ["ovim", "vim"] {
        editor
            .execute_lua(&format!(
                "{namespace}.opt.pullbase = {{ path = {path_literal}, branch = 'develop' }}"
            ))
            .unwrap();
        editor.process_lua_commands().unwrap();
        assert_eq!(
            editor
                .options
                .pullbase_paths
                .get(&canonical)
                .map(String::as_str),
            Some("develop")
        );
        assert_eq!(editor.options.pullbase.as_deref(), Some("main"));
        for fields in [
            "branch = true",
            "branch = 'main..feature'",
            "brnach = 'develop'",
        ] {
            assert!(editor
                .execute_lua(&format!(
                    "{namespace}.opt.pullbase = {{ path = {path_literal}, {fields} }}"
                ))
                .is_err());
        }
        editor
            .execute_lua(&format!(
                "{namespace}.opt.pullbase = {{ path = {path_literal}, branch = nil }}"
            ))
            .unwrap();
        editor.process_lua_commands().unwrap();
        assert!(editor.options.pullbase_paths.is_empty());
        assert_eq!(editor.options.pullbase.as_deref(), Some("main"));
    }
}

#[test]
fn idle_lua_ticks_do_not_copy_the_buffer() {
    // 20 MB of text: copying it into a String every tick (what the bridge
    // used to do) costs several milliseconds per tick, so 200 idle ticks
    // would take seconds. Handing the bridge a rope handle is O(1).
    let line = format!("{}\n", "x".repeat(79));
    let mut editor = Editor::with_content(&line.repeat(250_000));
    editor.enable_lua().expect("Failed to enable Lua");

    let started = std::time::Instant::now();
    for _ in 0..200 {
        editor.process_lua_commands().unwrap();
    }
    let elapsed = started.elapsed();

    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "200 idle Lua ticks took {elapsed:?} on a 20 MB buffer"
    );
}

#[test]
fn lua_reads_the_current_line_and_line_count_after_edits() {
    let mut editor = Editor::with_content("one\ntwo\nthree\n");
    editor.enable_lua().expect("Failed to enable Lua");

    assert_eq!(editor.execute_lua("return vim.fn.line('$')").unwrap(), "3");

    // Edit the second line, tick, and read it back through Lua.
    for ch in "jccchanged".chars() {
        InputHandler::handle_key_event(
            &mut editor,
            KeyEvent::new(KeyCode::Char(ch), Modifiers::NONE),
        )
        .unwrap();
    }
    InputHandler::handle_key_event(&mut editor, KeyEvent::new(KeyCode::Esc, Modifiers::NONE))
        .unwrap();
    editor.process_lua_commands().unwrap();

    assert_eq!(
        editor
            .execute_lua("return vim.api.nvim_get_current_line()")
            .unwrap(),
        "changed"
    );
    assert_eq!(editor.execute_lua("return vim.fn.line('$')").unwrap(), "3");
}
