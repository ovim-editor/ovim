mod helpers;

use helpers::EditorTest;

/// Test entering command mode with :
#[test]
fn test_enter_command_mode() {
    let mut test = EditorTest::new("test\n");

    test.press(':');

    test.assert_mode(ovim::mode::Mode::Command);
}

/// Test :q command
#[test]
fn test_command_quit() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.type_text("q");
    test.press_enter();

    // Editor should be marked to quit
    assert!(test.editor.should_quit());
}

/// Test :w command
#[test]
fn test_command_write() {
    let mut test = EditorTest::new("content\n");
    test.set_file_path("/tmp/test_write.txt".to_string());

    test.press(':');
    test.type_text("w");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test :wq command
#[test]
fn test_command_write_quit() {
    let mut test = EditorTest::new("content\n");
    test.set_file_path("/tmp/test_wq.txt".to_string());

    test.press(':');
    test.type_text("wq");
    test.press_enter();

    assert!(test.editor.should_quit());
}

/// Test :q! command (force quit)
#[test]
fn test_command_force_quit() {
    let mut test = EditorTest::new("test\n");

    // Make a change
    test.press('i');
    test.type_text("change");
    test.press_esc();

    test.press(':');
    test.type_text("q!");
    test.press_enter();

    assert!(test.editor.should_quit());
}

/// Test :e command (edit file)
#[test]
fn test_command_edit_file() {
    // Create the file first
    std::fs::write("/tmp/newfile.txt", "new content\n").unwrap();

    let mut test = EditorTest::new("old\n");

    test.press(':');
    test.type_text("e /tmp/newfile.txt");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);

    // Clean up
    std::fs::remove_file("/tmp/newfile.txt").ok();
}

/// Test :s command (substitute)
#[test]
fn test_command_substitute() {
    let mut test = EditorTest::new("hello world\n");

    test.press(':');
    test.type_text("s/hello/goodbye");
    test.press_enter();

    assert!(test.buffer_content().contains("goodbye"));
}

/// Test :s with g flag (global on line)
#[test]
fn test_command_substitute_global() {
    let mut test = EditorTest::new("foo foo foo\n");

    test.press(':');
    test.type_text("s/foo/bar/g");
    test.press_enter();

    assert_eq!(test.buffer_content(), "bar bar bar\n");
}

/// Test :%s command (substitute all lines)
#[test]
fn test_command_substitute_all_lines() {
    let mut test = EditorTest::new("hello\nworld\nhello\n");

    test.press(':');
    test.type_text("%s/hello/hi");
    test.press_enter();

    let content = test.buffer_content();
    assert!(content.contains("hi"));
    assert!(!content.contains("hello") || content.matches("hello").count() == 1);
}

/// Test :d command (delete line)
#[test]
fn test_command_delete() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    test.press(':');
    test.type_text("2d");
    test.press_enter();

    let content = test.buffer_content();
    assert!(!content.contains("line2"));
}

/// Test :y command (yank)
#[test]
fn test_command_yank() {
    let mut test = EditorTest::new("yank me\nother\n");

    test.press(':');
    test.type_text("y");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test :set command
#[test]
fn test_command_set() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.type_text("set number");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test escaping from command mode
#[test]
fn test_escape_command_mode() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.type_text("incomplete command");
    test.press_esc();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test backspace in command mode
#[test]
fn test_backspace_in_command_mode() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.type_text("wrong");
    // Backspace 5 times to delete "wrong"
    test.press_backspace();
    test.press_backspace();
    test.press_backspace();
    test.press_backspace();
    test.press_backspace();
    test.type_text("q");
    test.press_enter();

    assert!(test.editor.should_quit());
}

/// Test :! command (shell command)
#[test]
fn test_command_shell() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.type_text("!echo hello");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test :help command
#[test]
fn test_command_help() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.type_text("help");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test :version command
#[test]
fn test_command_version() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.type_text("version");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test line number navigation (:42)
#[test]
fn test_command_line_number() {
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\n");

    test.press(':');
    test.type_text("3");
    test.press_enter();

    test.assert_cursor(2, 0);
}

/// Test :$ (go to last line)
#[test]
fn test_command_dollar_last_line() {
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\n");

    test.press(':');
    test.type_text("$");
    test.press_enter();

    // Should be on last line
    assert!(test.cursor().0 >= 3);
}

/// Test range delete (:1,3d)
#[test]
fn test_command_range_delete() {
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\n");

    test.press(':');
    test.type_text("1,3d");
    test.press_enter();

    let content = test.buffer_content();
    assert!(!content.contains("l1"));
    assert!(!content.contains("l2"));
    assert!(!content.contains("l3"));
    assert!(content.contains("l4"));
}

/// Test range yank (:1,2y)
#[test]
fn test_command_range_yank() {
    let mut test = EditorTest::new("l1\nl2\nl3\n");

    test.press(':');
    test.type_text("1,2y");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test :split command
#[test]
fn test_command_split() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.type_text("split");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test :vsplit command
#[test]
fn test_command_vsplit() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.type_text("vsplit");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test :tabnew command
#[test]
fn test_command_tabnew() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.type_text("tabnew");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test :source command
#[test]
fn test_command_source() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.type_text("source /tmp/config.vim");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test empty command (just pressing Enter)
#[test]
fn test_command_empty() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test command with leading/trailing spaces
#[test]
fn test_command_with_spaces() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.type_text("  q  ");
    test.press_enter();

    assert!(test.editor.should_quit());
}

/// Test invalid command
#[test]
fn test_command_invalid() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.type_text("invalidcommand");
    test.press_enter();

    // Should return to normal mode
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test :noh command (no highlight)
#[test]
fn test_command_noh() {
    let mut test = EditorTest::new("test\n");

    test.press('/');
    test.type_text("test");
    test.press_enter();

    test.press(':');
    test.type_text("noh");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test :reg command (show registers)
#[test]
fn test_command_registers() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.type_text("reg");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test :marks command
#[test]
fn test_command_marks() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.type_text("marks");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}
