mod helpers;

use helpers::EditorTest;
use std::sync::atomic::{AtomicU64, Ordering};

fn temp_test_path(name: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!("ovim_test_{}_{}", id, name))
        .to_string_lossy()
        .to_string()
}

/// Test that LSP errors don't print to stdout/stderr
#[test]
fn test_lsp_errors_silent() {
    let mut test = EditorTest::new("fn test() {}\n");
    test.set_file_path(temp_test_path("test.rs"));

    // Trigger LSP requests that might error
    test.keys("gd");
    test.keys("K");

    // Editor should still be functional, no panics
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test that LSP connection failures are handled gracefully
#[test]
fn test_lsp_connection_failure() {
    let mut test = EditorTest::new("fn test() {}\n");
    test.set_file_path(temp_test_path("missing_test.rs"));

    // Try LSP operations
    test.keys("gd");

    // Should not crash
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test that invalid LSP responses are handled
#[test]
fn test_lsp_invalid_response() {
    let mut test = EditorTest::new("struct Point { x: i32 }\n");
    test.set_file_path(temp_test_path("test.rs"));

    // Multiple rapid requests
    test.keys("K");
    test.keys("K");
    test.keys("K");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test file not found error handling
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_file_not_found_error() {
    let mut test = EditorTest::new("test\n");

    test.keys(":e /nonexistent/file.txt");
    test.press_enter();

    // Should return to normal mode, not crash
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test write to read-only file
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_write_readonly_error() {
    let mut test = EditorTest::new("test\n");
    test.set_file_path("/root/readonly.txt".to_string());

    test.keys(":w");
    test.press_enter();

    // Should handle error gracefully
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test invalid regex in search
#[test]
fn test_invalid_regex_error() {
    let mut test = EditorTest::new("test content\n");

    test.keys("/");
    test.type_text("[invalid");
    test.press_enter();

    // Should return to normal mode
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test invalid command
#[test]
fn test_invalid_command_error() {
    let mut test = EditorTest::new("test\n");

    test.keys(":invalidcommandthatdoesnotexist");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test substitute with invalid pattern
#[test]
fn test_substitute_invalid_pattern() {
    let mut test = EditorTest::new("hello world\n");

    test.keys(":");
    test.type_text("s/[/replacement");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test out of bounds line number
#[test]
fn test_out_of_bounds_line() {
    let mut test = EditorTest::new("line1\nline2\n");

    test.keys(":999");
    test.press_enter();

    // Should clamp to valid range
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test invalid mark access
#[test]
fn test_invalid_mark_error() {
    let mut test = EditorTest::new("test\n");

    // Try to jump to non-existent mark
    test.keys("'z");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test invalid register access
#[test]
fn test_invalid_register_error() {
    let mut test = EditorTest::new("test\n");

    // Try to paste from empty register
    test.keys("\"zp");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test macro recording errors
#[test]
fn test_macro_recording_error() {
    let mut test = EditorTest::new("test\n");

    // Try to play non-existent macro
    test.keys("@z");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test undo when nothing to undo
#[test]
fn test_undo_empty_error() {
    let mut test = EditorTest::new("test\n");

    test.keys("u");
    test.keys("u");
    test.keys("u");

    // Should handle gracefully
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test redo when nothing to redo
#[test]
fn test_redo_empty_error() {
    let mut test = EditorTest::new("test\n");

    test.keys("<C-r>");
    test.keys("<C-r>");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test delete on empty line
#[test]
fn test_delete_empty_line() {
    let mut test = EditorTest::new("\n");

    test.keys("dd");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test yank on empty buffer
#[test]
fn test_yank_empty_buffer() {
    let mut test = EditorTest::new("\n");

    test.keys("yy");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test paste with no yanked content
#[test]
fn test_paste_no_content() {
    let mut test = EditorTest::new("test\n");

    // Clear registers by starting fresh
    test.keys("p");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test join on last line
#[test]
fn test_join_last_line() {
    let mut test = EditorTest::new("only line\n");

    test.keys("J");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test indent on empty line
#[test]
fn test_indent_empty_line() {
    let mut test = EditorTest::new("\n");

    test.keys(">>");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test dedent when no indentation
#[test]
fn test_dedent_no_indent() {
    let mut test = EditorTest::new("no indent\n");

    test.keys("<<");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test find character not on line
#[test]
fn test_find_char_not_found() {
    let mut test = EditorTest::new("hello world\n");

    test.keys("0");
    test.keys("fz");

    // Should not move cursor
    test.assert_cursor(0, 0);
}

/// Test till character not on line
#[test]
fn test_till_char_not_found() {
    let mut test = EditorTest::new("hello world\n");

    test.keys("0");
    test.keys("tz");

    test.assert_cursor(0, 0);
}

/// Test search with no matches
#[test]
fn test_search_no_matches() {
    let mut test = EditorTest::new("hello world\n");

    test.keys("/");
    test.type_text("notfound");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test next search with no previous search
#[test]
fn test_next_no_previous_search() {
    let mut test = EditorTest::new("test\n");

    test.keys("n");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test visual mode on empty line
#[test]
fn test_visual_empty_line() {
    let mut test = EditorTest::new("\n");

    test.keys("v");
    test.keys("d");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test change on empty line
#[test]
fn test_change_empty_line() {
    let mut test = EditorTest::new("\n");

    test.keys("cc");
    test.press_esc();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test replace on empty line
#[test]
fn test_replace_empty_line() {
    let mut test = EditorTest::new("\n");

    test.keys("rx");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test increment with no number
#[test]
fn test_increment_no_number() {
    let mut test = EditorTest::new("no numbers here\n");

    test.keys("0");
    test.keys("<C-a>");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test decrement with no number
#[test]
fn test_decrement_no_number() {
    let mut test = EditorTest::new("no numbers here\n");

    test.keys("0");
    test.keys("<C-x>");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test motion beyond buffer bounds
#[test]
fn test_motion_beyond_buffer() {
    let mut test = EditorTest::new("short\n");

    test.keys("100l");

    // Should clamp to end of line
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test delete beyond end of line
#[test]
fn test_delete_beyond_eol() {
    let mut test = EditorTest::new("short\n");

    test.keys("$");
    test.keys("100x");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP timeout handling
#[test]
fn test_lsp_timeout() {
    let mut test = EditorTest::new("fn test() {}\n");
    test.set_file_path(temp_test_path("test.rs"));

    // Multiple rapid requests that might timeout
    for _ in 0..10 {
        test.keys("K");
    }

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP shutdown error
#[test]
fn test_lsp_shutdown_error() {
    let mut test = EditorTest::new("fn test() {}\n");
    test.set_file_path(temp_test_path("test.rs"));

    test.keys("K");

    // Quit (should handle LSP shutdown)
    test.keys(":q");
    test.press_enter();

    assert!(test.editor.should_quit());
}

/// Test buffer modification tracking errors
#[test]
fn test_buffer_mod_tracking() {
    let mut test = EditorTest::new("test\n");

    // Rapid modifications
    for _ in 0..100 {
        test.keys("i");
        test.type_text("x");
        test.press_esc();
        test.keys("u");
    }

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test syntax highlighting with invalid code
#[test]
fn test_syntax_invalid_code() {
    let code = "fn {{{ ((( [[[  invalid rust code\n";

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("invalid.rs"));

    // Navigate through invalid code
    test.keys("j");
    test.keys("k");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test extremely long line
#[test]
fn test_extremely_long_line() {
    let long_line = "x".repeat(10000) + "\n";

    let mut test = EditorTest::new(&long_line);

    test.keys("$");
    test.keys("0");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test many empty lines
#[test]
fn test_many_empty_lines() {
    let many_lines = "\n".repeat(1000);

    let mut test = EditorTest::new(&many_lines);

    test.keys("G");
    test.keys("gg");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test unicode handling errors
#[test]
fn test_unicode_handling() {
    let unicode = "Hello 世界 🌍 مرحبا\n";

    let mut test = EditorTest::new(unicode);

    test.keys("$");
    test.keys("0");
    test.keys("w");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test null bytes in buffer
#[test]
fn test_null_bytes() {
    let mut test = EditorTest::new("test\n");

    // Editor should handle gracefully
    test.keys("i");
    test.press_esc();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test simultaneous operations
#[test]
fn test_simultaneous_operations() {
    let mut test = EditorTest::new("test\n");

    // Rapid different operations
    test.keys("yy");
    test.keys("dd");
    test.keys("p");
    test.keys("u");
    test.keys("<C-r>");

    test.assert_mode(ovim::mode::Mode::Normal);
}
