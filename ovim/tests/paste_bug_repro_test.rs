mod helpers;
use helpers::EditorTest;

// ============================================================================
// Regression tests for paste off-by-one bugs
// Fixed: linewise paste on last line (no trailing newline) and
//        characterwise paste on empty lines
// ============================================================================

/// Linewise paste (yy p) on single-line buffer without trailing newline
#[test]
fn test_linewise_paste_single_line_no_trailing_newline() {
    let mut test = EditorTest::new("");

    test.keys("i```<Esc>");
    assert_eq!(test.buffer_content(), "```\n");

    test.keys("yy");
    test.keys("p");

    // Should create two lines, not concatenate on one line
    assert_eq!(test.buffer_content(), "```\n```\n");
    test.assert_cursor(1, 0);
}

/// Linewise paste from dd register after inserting text
#[test]
fn test_linewise_paste_after_insert_and_dd() {
    let mut test = EditorTest::new("AB");

    test.keys("dd"); // Delete "AB", register = "AB\n" (Line)
    test.keys("i```<Esc>"); // Insert backticks, buffer = "```"
    test.keys("p"); // Paste linewise

    assert_eq!(test.buffer_content(), "```\nAB\n");
}

/// Linewise paste on empty buffer (yy dd p)
#[test]
fn test_linewise_paste_empty_buffer() {
    let mut test = EditorTest::new("hello");

    test.keys("yy"); // Yank "hello\n"
    test.keys("dd"); // Delete entire buffer (now empty)
    test.keys("p"); // Paste linewise

    assert_eq!(test.buffer_content(), "hello\n");
}

/// Characterwise paste on empty line
#[test]
fn test_characterwise_paste_on_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.keys("yiw"); // Yank "hello"
    test.keys("j"); // Move to empty line
    test.keys("p"); // Paste on empty line

    // Should insert on the empty line, not on the next line
    assert_eq!(test.buffer_content(), "hello\nhello\nworld\n");
}

/// Characterwise paste at end of line (should still work)
#[test]
fn test_characterwise_paste_at_end_of_line() {
    let mut test = EditorTest::new("abc");

    test.keys("yiw"); // Yank "abc"
    test.keys("$"); // Move to end (col 2)
    test.keys("p"); // Paste after last char

    assert_eq!(test.buffer_content(), "abcabc\n");
}

/// Characterwise paste at col 0
#[test]
fn test_characterwise_paste_at_beginning() {
    let mut test = EditorTest::new("abc");

    test.keys("yiw");
    test.keys("0");
    test.keys("p");

    assert_eq!(test.buffer_content(), "aabcbc\n");
}

/// Paste then undo should fully restore buffer
#[test]
fn test_paste_undo_no_residue() {
    let mut test = EditorTest::new("hello world");

    test.keys("yiw");
    test.keys("$");
    test.keys("p");
    test.keys("u");

    assert_eq!(test.buffer_content(), "hello world\n");
}

/// Linewise paste at end of file (file has trailing newline)
#[test]
fn test_linewise_paste_at_end_with_trailing_newline() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("yy"); // Yank line 1
    test.keys("G"); // Go to last line
    test.keys("p"); // Paste after

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\nline 1\n");
    test.assert_cursor(3, 0);
}
