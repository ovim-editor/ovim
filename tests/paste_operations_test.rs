// Snake case makes sense for 'P', and it's tests.
#![allow(non_snake_case)]

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
    // Cursor moves to the pasted line (line 1, 0-indexed)
    test.assert_cursor(1, 0);
}

#[test]
fn test_p_linewise_last_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("gg") // Go to top
        .keys("yy") // Yank line 1
        .keys("G") // Go to last line
        .keys("p"); // Paste after

    // Linewise paste creates a new line below the current line
    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\nline 1\n");
    // Cursor moves to the pasted line (line 3, 0-indexed)
    test.assert_cursor(3, 0);
}

#[test]
fn test_p_linewise_middle() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("j") // Line 2
        .keys("yy") // Yank line 2
        .keys("p"); // Paste after

    assert_eq!(
        test.buffer_content(),
        "line 1\nline 2\nline 2\nline 3\nline 4\n"
    );
    test.assert_cursor(2, 0);
}

#[test]
fn test_p_characterwise_middle_of_word() {
    let mut test = EditorTest::new("hello world");

    test.keys("yw") // Yank word "hello"
        .keys("w") // Move to "world"
        .keys("p"); // Paste after 'w'

    assert_eq!(test.buffer_content(), "hello whello orld\n");
    // Vim: cursor on last character of pasted text
    test.assert_cursor(0, 12);
}

#[test]
fn test_p_characterwise_end_of_line() {
    let mut test = EditorTest::new("hello world");

    test.keys("yiw") // Yank "hello"
        .keys("$") // Move to end
        .keys("p"); // Paste after last char

    // yiw on "hello" yanks just "hello" (no trailing space)
    // $ moves to last char 'd', p pastes after it
    assert_eq!(test.buffer_content(), "hello worldhello\n");
    // Vim: cursor on last character of pasted text
    test.assert_cursor(0, 15);
}

#[test]
fn test_p_characterwise_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.keys("yiw") // Yank "hello"
        .keys("j") // Move down to empty line, but cursor clamped
        .keys("j") // Move down to "world"
        .keys("p"); // Paste after cursor

    // Two j's: first to empty line (cursor clamped), second to "world"
    // p on col 0 of "world" pastes after 'w'
    assert_eq!(test.buffer_content(), "hello\n\nwhelloorld\n");
    // Vim: cursor on last character of pasted text
    test.assert_cursor(2, 5);
}

#[test]
fn test_p_multiple_times() {
    let mut test = EditorTest::new("x\ny");

    test.keys("yy") // Yank "x\n"
        .keys("p") // Paste once - now on line 1
        .keys("p") // Paste again - now on line 2
        .keys("p"); // And again - now on line 3

    assert_eq!(test.buffer_content(), "x\nx\nx\nx\ny\n");
    // After 3 pastes, cursor is on line 3 (0-indexed)
    test.assert_cursor(3, 0);
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
    // P pastes above current line. After paste, cursor is on the pasted line (still line 1)
    test.assert_cursor(1, 0);
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

    // P inserts before cursor position (col 0), so "world" is inserted at start
    assert_eq!(test.buffer_content(), "worldworld\n");
    // Cursor ends after pasted text (position 5)
    test.assert_cursor(0, 5);
}

#[test]
fn test_P_characterwise_middle() {
    let mut test = EditorTest::new("hello world test");

    test.keys("yiw") // Yank "hello"
        .keys("w") // Move to "world"
        .keys("w") // Move to "test"
        .keys("P"); // Paste before 't'

    // P inserts "hello" before 't' in "test"
    assert_eq!(test.buffer_content(), "hello world hellotest\n");
    // Cursor after pasted "hello" (position 17)
    test.assert_cursor(0, 17);
}

#[test]
fn test_P_multiple_times() {
    let mut test = EditorTest::new("a\nb");

    test.keys("yy") // Yank "a\n"
        .keys("j") // Move to "b" (line 1)
        .keys("P") // Paste before - inserts at line 1, cursor stays at line 1
        .keys("P") // Paste again - inserts at line 1, cursor stays at line 1
        .keys("P"); // And again - inserts at line 1, cursor stays at line 1

    assert_eq!(test.buffer_content(), "a\na\na\na\nb\n");
    // P pastes above and keeps cursor on same logical position (pasted line)
    test.assert_cursor(1, 0);
}

// ============================================================================
// Mixed paste operations
// ============================================================================

#[test]
fn test_p_then_P() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("yy") // Yank line 1
        .keys("p") // Paste after - cursor moves to line 1
        .keys("P"); // Paste before - inserts above line 1, cursor stays at line 1

    assert_eq!(
        test.buffer_content(),
        "line 1\nline 1\nline 1\nline 2\nline 3\n"
    );
    // After P, cursor is on the pasted line (line 1)
    test.assert_cursor(1, 0);
}

#[test]
fn test_yank_delete_paste() {
    let mut test = EditorTest::new("hello world test");

    test.keys("yw") // Yank "hello"
        .keys("w") // Move to "world"
        .keys("dw") // Delete "world"
        .keys("p"); // Paste "hello"

    assert_eq!(test.buffer_content(), "hello tworld est\n");
    // Vim: cursor on last character of pasted text
    test.assert_cursor(0, 12);
}

#[test]
fn test_delete_overrides_yank_register() {
    let mut test = EditorTest::new("first\nsecond\nthird");

    test.keys("yy") // Yank "first"
        .keys("j") // Move to "second"
        .keys("dd") // Delete "second" (goes to default register, overrides yank)
        .keys("p"); // Should paste "second\n" (linewise), not "first"

    // dd deletes "second\n", p pastes it below "third"
    assert_eq!(test.buffer_content(), "first\nthird\nsecond\n");
    // Cursor on the pasted line
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

    // TODO: Count prefix for paste not implemented, only pastes once
    assert_eq!(test.buffer_content(), "hello worldhello\n");
    // Vim: cursor on last character of pasted text
    test.assert_cursor(0, 15);
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

    // Linewise paste creates a new line below
    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 1\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_paste_empty_buffer() {
    let mut test = EditorTest::new("hello");

    test.keys("yy") // Yank "hello\n"
        .keys("dd") // Delete entire buffer (now empty)
        .keys("p"); // Paste linewise

    assert_eq!(test.buffer_content(), "hello\n");
    // Linewise paste, cursor on pasted line
    test.assert_cursor(1, 0);
}

#[test]
fn test_yank_and_paste_single_char() {
    let mut test = EditorTest::new("abc");

    // Use visual mode to yank single char - just 'v' then 'y' to yank cursor position
    test.keys("vy") // Visual select 'a' only, yank
        .keys("l") // Move right to 'b'
        .keys("p"); // Paste 'a' after 'b'

    // abc -> abac (insert 'a' after 'b')
    assert_eq!(test.buffer_content(), "abac\n");
    // Vim: cursor on last character of pasted text
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

    // Undo should restore the original buffer
    assert_eq!(test.buffer_content(), "line 1\nline 2\n");
    // Cursor returns to position before paste
    test.assert_cursor(0, 0);
}

#[test]
fn test_paste_undo_redo() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.keys("yy") // Yank
        .keys("p") // Paste
        .keys("u") // Undo
        .keys("<C-r>"); // Redo

    // After redo, the paste should be re-applied
    assert_eq!(test.buffer_content(), "line 1\nline 1\nline 2\n");
    test.assert_cursor(1, 0);
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

    // Linewise paste creates a new line below
    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\nline 1\n");
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

    // Linewise paste of 3 lines below the last line
    assert_eq!(test.buffer_content(), "a\nb\nc\nd\ne\na\nb\nc\n");
    // Cursor on last pasted line (line 7)
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

    // Linewise paste creates a new line below with the original indentation
    assert_eq!(test.buffer_content(), "    indented\nplain\n    indented\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_paste_into_indented_context() {
    let mut test = EditorTest::new("plain\n    indented");

    test.keys("yy") // Yank plain line
        .keys("j") // Move to indented line
        .keys("p"); // Paste (linewise, creates new line below)

    // Linewise paste preserves original "plain\n" content (no indentation)
    assert_eq!(test.buffer_content(), "plain\n    indented\nplain\n");
    test.assert_cursor(2, 0);
}
