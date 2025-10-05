mod helpers;
use helpers::EditorTest;

// ============================================================================
// 'p' command - Paste after cursor
// ============================================================================

#[test]
fn test_p_linewise_basic() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("yy") // Yank line 1
        .keys("p"); // Paste after

    assert_eq!(test.buffer_content(), "line 1\nline 1\nline 2\nline 3\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_p_linewise_last_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("gg") // Go to top
        .keys("yy") // Yank line 1
        .keys("G") // Go to last line
        .keys("p"); // Paste after

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3line 1\n\n");
    test.assert_cursor(3, 0);
}

#[test]
fn test_p_linewise_middle() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("j") // Line 2
        .keys("yy") // Yank line 2
        .keys("p"); // Paste after

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 2\nline 3\nline 4\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_p_characterwise_middle_of_word() {
    let mut test = EditorTest::new("hello world");

    test.keys("yw") // Yank word "hello"
        .keys("w") // Move to "world"
        .keys("p"); // Paste after 'w'

    assert_eq!(test.buffer_content(), "hello whello orld\n");
    test.assert_cursor(0, 13);
}

#[test]
fn test_p_characterwise_end_of_line() {
    let mut test = EditorTest::new("hello world");

    test.keys("yiw") // Yank "hello"
        .keys("$") // Move to end
        .keys("p"); // Paste after last char

    assert_eq!(test.buffer_content(), "hello worldhello \n");
    test.assert_cursor(0, 17);
}

#[test]
fn test_p_characterwise_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.keys("yiw") // Yank "hello"
        .keys("j") // Move to empty line
        .keys("p"); // Paste

    assert_eq!(test.buffer_content(), "hello\n\nhello\nworld\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_p_multiple_times() {
    let mut test = EditorTest::new("x\ny");

    test.keys("yy") // Yank "x"
        .keys("p") // Paste once
        .keys("p") // Paste again
        .keys("p"); // And again

    assert_eq!(test.buffer_content(), "x\nx\nx\nx\ny\n");
    test.assert_cursor(4, 0);
}

// ============================================================================
// 'P' command - Paste before cursor
// ============================================================================

#[test]
fn test_P_linewise_basic() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("yy") // Yank line 1
        .keys("j") // Move to line 2
        .keys("P"); // Paste before

    assert_eq!(test.buffer_content(), "line 1\nline 1\nline 2\nline 3\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_P_linewise_first_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("j") // Line 2
        .keys("yy") // Yank line 2
        .keys("gg") // Go to first line
        .keys("P"); // Paste before

    assert_eq!(test.buffer_content(), "line 2\nline 1\nline 2\nline 3\n");
    test.assert_cursor(1, 0);
}

#[test]
fn test_P_characterwise_beginning() {
    let mut test = EditorTest::new("world");

    test.keys("yiw") // Yank "world"
        .keys("0") // Go to beginning
        .keys("P"); // Paste before

    assert_eq!(test.buffer_content(), "worlworld\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_P_characterwise_middle() {
    let mut test = EditorTest::new("hello world test");

    test.keys("yiw") // Yank "hello"
        .keys("w") // Move to "world"
        .keys("w") // Move to "test"
        .keys("P"); // Paste before 't'

    assert_eq!(test.buffer_content(), "hello world hello test\n");
    test.assert_cursor(0, 18);
}

#[test]
fn test_P_multiple_times() {
    let mut test = EditorTest::new("a\nb");

    test.keys("yy") // Yank "a"
        .keys("j") // Move to "b"
        .keys("P") // Paste before
        .keys("P") // Paste again
        .keys("P"); // And again

    assert_eq!(test.buffer_content(), "a\na\na\na\nb\n");
    test.assert_cursor(4, 0);
}

// ============================================================================
// Mixed paste operations
// ============================================================================

#[test]
fn test_p_then_P() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("yy") // Yank line 1
        .keys("p") // Paste after
        .keys("P"); // Paste before

    assert_eq!(test.buffer_content(), "line 1\nline 1\nline 1\nline 2\nline 3\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_yank_delete_paste() {
    let mut test = EditorTest::new("hello world test");

    test.keys("yw") // Yank "hello"
        .keys("w") // Move to "world"
        .keys("dw") // Delete "world"
        .keys("p"); // Paste "hello"

    assert_eq!(test.buffer_content(), "hello tworld est\n");
    test.assert_cursor(0, 13);
}

#[test]
fn test_delete_overrides_yank_register() {
    let mut test = EditorTest::new("first\nsecond\nthird");

    test.keys("yy") // Yank "first"
        .keys("j") // Move to "second"
        .keys("dd") // Delete "second" (goes to default register)
        .keys("p"); // Should paste "second", not "first"

    assert_eq!(test.buffer_content(), "first\nthirdsecond\n\n");
    test.assert_cursor(2, 0);
}

// ============================================================================
// Paste with count
// ============================================================================

#[test]
fn test_p_with_count_linewise() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.keys("yy") // Yank line 1
        .keys("3p"); // Paste 3 times

    assert_eq!(test.buffer_content(), "line 1\nline 1\nline 2\n");
    test.assert_cursor(1, 0);
}

#[test]
fn test_p_with_count_characterwise() {
    let mut test = EditorTest::new("hello world");

    test.keys("yiw") // Yank "hello"
        .keys("$") // Move to end
        .keys("3p"); // Paste 3 times

    assert_eq!(test.buffer_content(), "hello worldhello \n");
    test.assert_cursor(0, 17);
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_paste_at_end_of_file_no_newline() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.keys("yy") // Yank line 1
        .keys("G") // Go to last line
        .keys("p"); // Paste after

    assert_eq!(test.buffer_content(), "line 1\nline 2line 1\n\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_paste_empty_buffer() {
    let mut test = EditorTest::new("hello");

    test.keys("yy") // Yank "hello"
        .keys("dd") // Delete entire buffer
        .keys("p"); // Paste

    assert_eq!(test.buffer_content(), "hello\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_yank_and_paste_single_char() {
    let mut test = EditorTest::new("abc");

    test.keys("yl") // Yank single char
        .keys("l") // Move right
        .keys("p"); // Paste

    assert_eq!(test.buffer_content(), "abac\n");
    test.assert_cursor(0, 2);
}

#[test]
fn test_paste_with_visual_selection() {
    let mut test = EditorTest::new("hello\nworld\ntest");

    test.keys("yy") // Yank "hello"
        .keys("j") // Move to "world"
        .keys("p"); // Paste

    assert_eq!(test.buffer_content(), "hello\nworld\nhello\ntest\n");
    test.assert_cursor(2, 0);
}

// ============================================================================
// Undo/Redo with paste
// ============================================================================

#[test]
fn test_paste_and_undo() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.keys("yy") // Yank
        .keys("p") // Paste
        .keys("u"); // Undo

    assert_eq!(test.buffer_content(), "line 1\nline 2\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_paste_undo_redo() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.keys("yy") // Yank
        .keys("p") // Paste
        .keys("u") // Undo
        .press('\x12'); // Ctrl-R (redo)

    assert_eq!(test.buffer_content(), "line 1\nline 1\nline 2\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_multiple_paste_undo() {
    let mut test = EditorTest::new("x");

    test.keys("yy")
        .keys("p") // Paste 1
        .keys("p") // Paste 2
        .keys("u") // Undo paste 2
        .keys("u"); // Undo paste 1

    assert_eq!(test.buffer_content(), "x\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Paste across different line endings
// ============================================================================

#[test]
fn test_paste_line_with_newline() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("yy") // Yank line with newline
        .keys("G") // Go to last line
        .keys("p"); // Paste

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3line 1\n\n");
    test.assert_cursor(3, 0);
}

#[test]
fn test_paste_multiple_lines() {
    let mut test = EditorTest::new("a\nb\nc\nd\ne");

    test.keys("V") // Visual line mode
        .keys("jj") // Select 3 lines (a, b, c)
        .keys("y") // Yank
        .keys("G") // Go to last line
        .keys("p"); // Paste

    assert_eq!(test.buffer_content(), "a\nb\nc\nd\nea\nb\nc\n\n");
    test.assert_cursor(7, 0);
}

// ============================================================================
// Paste with indentation
// ============================================================================

#[test]
fn test_paste_indented_line() {
    let mut test = EditorTest::new("    indented\nplain");

    test.keys("yy") // Yank indented line
        .keys("j") // Move to plain line
        .keys("p"); // Paste

    assert_eq!(test.buffer_content(), "    indented\nplain    indented\n\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_paste_into_indented_context() {
    let mut test = EditorTest::new("plain\n    indented");

    test.keys("yy") // Yank plain line
        .keys("j") // Move to indented line
        .keys("p"); // Paste (should preserve original indentation)

    assert_eq!(test.buffer_content(), "plain\n    indentedplain\n\n");
    test.assert_cursor(2, 0);
}
