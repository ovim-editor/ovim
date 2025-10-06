mod helpers;

use helpers::EditorTest;

/// Test J (join lines)
#[test]
fn test_j_join_lines() {
    let mut test = EditorTest::new("line one\nline two\nline three\n");

    test.keys("J");

    assert!(test.buffer_content().contains("line one line two"));
}

/// Test J with count
#[test]
fn test_j_join_with_count() {
    let mut test = EditorTest::new("a\nb\nc\nd\n");

    test.keys("3J");

    // Should join 3 lines total (current + 2 below)
    assert!(test.buffer_content().contains("a b c"));
}

/// Test gJ (join without space)
#[test]
fn test_gj_join_no_space() {
    let mut test = EditorTest::new("hello\nworld\n");

    test.keys("gJ");

    assert!(test.buffer_content().contains("helloworld"));
}

/// Test dd (delete line)
#[test]
fn test_dd_delete_line() {
    let mut test = EditorTest::new("keep\ndelete\nkeep\n");

    test.keys("j");
    test.keys("dd");

    let content = test.buffer_content();
    assert!(content.contains("keep"));
    assert!(!content.contains("delete"));
}

/// Test dd with count
#[test]
fn test_dd_delete_multiple_lines() {
    let mut test = EditorTest::new("line1\nline2\nline3\nline4\n");

    test.keys("2dd");

    let content = test.buffer_content();
    assert!(!content.contains("line1"));
    assert!(!content.contains("line2"));
    assert!(content.contains("line3"));
}

/// Test D (delete to end of line)
#[test]
fn test_d_capital_delete_to_eol() {
    let mut test = EditorTest::new("hello world\n");

    test.keys("0");
    test.keys("6l"); // Position after "hello "
    test.keys("D");

    assert_eq!(test.buffer_content(), "hello \n");
}

/// Test C (change to end of line)
#[test]
fn test_c_capital_change_to_eol() {
    let mut test = EditorTest::new("hello world\n");

    test.keys("0");
    test.keys("6l");
    test.keys("C");
    test.type_text("there");
    test.press_esc();

    assert!(test.buffer_content().contains("hello there"));
}

/// Test yy (yank line)
#[test]
fn test_yy_yank_line() {
    let mut test = EditorTest::new("yank me\nother\n");

    test.keys("yy");
    test.keys("j");
    test.keys("p");

    let content = test.buffer_content();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "yank me");
    assert_eq!(lines[2], "yank me");
}

/// Test yy with count
#[test]
fn test_yy_yank_multiple_lines() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    test.keys("2yy");
    test.keys("G");
    test.keys("p");

    let content = test.buffer_content();
    assert!(content.matches("line1").count() == 2);
    assert!(content.matches("line2").count() == 2);
}

/// Test Y (yank line - same as yy)
#[test]
fn test_y_capital_yank_line() {
    let mut test = EditorTest::new("yank this\nother\n");

    test.keys("Y");
    test.keys("p");

    assert!(test.buffer_content().matches("yank this").count() == 2);
}

/// Test cc (change line)
#[test]
fn test_cc_change_line() {
    let mut test = EditorTest::new("    old content\n");

    test.keys("cc");
    test.type_text("new content");
    test.press_esc();

    assert!(test.buffer_content().contains("new content"));
    assert!(!test.buffer_content().contains("old"));
}

/// Test S (substitute line - same as cc)
#[test]
fn test_s_capital_substitute_line() {
    let mut test = EditorTest::new("    original\n");

    test.keys("S");
    test.type_text("replaced");
    test.press_esc();

    assert!(test.buffer_content().contains("replaced"));
}

/// Test >> (indent line)
#[test]
fn test_shift_right_indent() {
    let mut test = EditorTest::new("not indented\n");

    test.keys(">>");

    let content = test.buffer_content();
    assert!(content.starts_with("    ") || content.starts_with("\t"));
}

/// Test >> with count
#[test]
fn test_shift_right_with_count() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    test.keys("2>>");

    let content = test.buffer_content();
    let lines: Vec<&str> = content.lines().collect();
    // First two lines should be indented
    assert!(lines[0].starts_with("    ") || lines[0].starts_with("\t"));
    assert!(lines[1].starts_with("    ") || lines[1].starts_with("\t"));
}

/// Test << (dedent line)
#[test]
fn test_shift_left_dedent() {
    let mut test = EditorTest::new("    indented\n");

    test.keys("<<");

    assert!(!test.buffer_content().starts_with("    "));
}

/// Test == (auto-indent line)
#[test]
fn test_equal_auto_indent() {
    let mut test = EditorTest::new("no indent\n");

    test.keys("==");

    // Should remain in normal mode
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test o (open line below)
#[test]
fn test_o_open_below() {
    let mut test = EditorTest::new("line1\nline2\n");

    test.keys("o");
    test.type_text("new line");
    test.press_esc();

    let content = test.buffer_content();
    assert!(content.contains("line1\nnew line\nline2"));
}

/// Test O (open line above)
#[test]
fn test_o_capital_open_above() {
    let mut test = EditorTest::new("line1\nline2\n");

    test.keys("j");
    test.keys("O");
    test.type_text("inserted");
    test.press_esc();

    let content = test.buffer_content();
    assert!(content.contains("line1\ninserted\nline2"));
}

/// Test p (paste below)
#[test]
fn test_p_paste_below() {
    let mut test = EditorTest::new("line1\nline2\n");

    test.keys("yy");
    test.keys("p");

    assert!(test.buffer_content().matches("line1").count() == 2);
}

/// Test P (paste above)
#[test]
fn test_p_capital_paste_above() {
    let mut test = EditorTest::new("line1\nline2\n");

    test.keys("j");      // Move to line2 (line index 1)
    test.keys("yy");     // Yank line2
    test.keys("P");      // Paste before line2

    let content = test.buffer_content();
    let lines: Vec<&str> = content.lines().collect();
    // After P, line2 is pasted before the current line
    // Result: line1, line2 (pasted), line2 (original)
    assert_eq!(lines[0], "line1");
    assert_eq!(lines[1], "line2");
    assert_eq!(lines[2], "line2");
}

/// Test ~ (toggle case)
#[test]
fn test_tilde_toggle_case() {
    let mut test = EditorTest::new("HeLLo\n");

    test.keys("0");
    test.keys("~");
    test.keys("~");

    let content = test.buffer_content();
    assert!(content.starts_with("hE"));
}

/// Test ~ with count
#[test]
fn test_tilde_with_count() {
    let mut test = EditorTest::new("hello\n");

    test.keys("0");
    test.keys("5~");

    assert!(test.buffer_content().contains("HELLO"));
}

/// Test x (delete character)
#[test]
fn test_x_delete_char() {
    let mut test = EditorTest::new("hello\n");

    test.keys("0");
    test.keys("x");

    assert_eq!(test.buffer_content(), "ello\n");
}

/// Test x with count
#[test]
fn test_x_delete_multiple_chars() {
    let mut test = EditorTest::new("hello world\n");

    test.keys("0");
    test.keys("5x");

    assert!(test.buffer_content().starts_with(" world"));
}

/// Test X (delete before cursor)
#[test]
fn test_x_capital_delete_before() {
    let mut test = EditorTest::new("hello\n");

    test.keys("$");
    test.keys("X");

    // X deletes the character BEFORE the cursor
    // $ puts cursor on the last char ('o' at position 4)
    // X deletes the char before it (second 'l' at position 3)
    assert_eq!(test.buffer_content(), "helo\n");
}

/// Test r (replace character)
#[test]
fn test_r_replace_char() {
    let mut test = EditorTest::new("hello\n");

    test.keys("0");
    test.keys("rx");

    assert_eq!(test.buffer_content(), "xello\n");
}

/// Test r with count
#[test]
fn test_r_replace_multiple_chars() {
    let mut test = EditorTest::new("hello\n");

    test.keys("0");
    test.keys("3rX");

    assert!(test.buffer_content().starts_with("XXX"));
}

/// Test s (substitute character)
#[test]
fn test_s_substitute_char() {
    let mut test = EditorTest::new("hello\n");

    test.keys("0");
    test.keys("s");
    test.type_text("H");
    test.press_esc();

    assert!(test.buffer_content().starts_with("Hello"));
}

/// Test s with count
#[test]
fn test_s_substitute_multiple_chars() {
    let mut test = EditorTest::new("hello world\n");

    test.keys("0");
    test.keys("5s");
    test.type_text("goodbye");
    test.press_esc();

    assert!(test.buffer_content().contains("goodbye"));
}

/// Test gu (lowercase) motion
#[test]
fn test_gu_lowercase_motion() {
    let mut test = EditorTest::new("HELLO WORLD\n");

    test.keys("0");
    test.keys("guw");

    assert!(test.buffer_content().starts_with("hello"));
}

/// Test gU (uppercase) motion
#[test]
fn test_gu_capital_uppercase_motion() {
    let mut test = EditorTest::new("hello world\n");

    test.keys("0");
    test.keys("gUw");

    assert!(test.buffer_content().starts_with("HELLO"));
}

/// Test guu (lowercase line)
#[test]
fn test_guu_lowercase_line() {
    let mut test = EditorTest::new("HELLO WORLD\n");

    test.keys("guu");

    assert_eq!(test.buffer_content(), "hello world\n");
}

/// Test gUU (uppercase line)
#[test]
fn test_guu_capital_uppercase_line() {
    let mut test = EditorTest::new("hello world\n");

    test.keys("gUU");

    assert_eq!(test.buffer_content(), "HELLO WORLD\n");
}

/// Test g~~ (toggle case line)
#[test]
fn test_g_tilde_tilde_toggle_line() {
    let mut test = EditorTest::new("HeLLo WoRLd\n");

    test.keys("g~~");

    assert!(test.buffer_content().contains("hEllO wOrlD"));
}
