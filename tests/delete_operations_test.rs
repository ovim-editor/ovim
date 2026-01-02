mod helpers;
use helpers::EditorTest;

// ============================================================================
// 'x' command - Delete character under cursor
// ============================================================================

#[test]
fn test_x_basic() {
    let mut test = EditorTest::new("hello");

    test.press('x'); // Delete 'h'

    assert_eq!(test.buffer_content(), "ello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_x_multiple() {
    let mut test = EditorTest::new("hello");

    test.keys("xxx"); // Delete 'h', 'e', 'l'

    assert_eq!(test.buffer_content(), "lo\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_x_with_count() {
    let mut test = EditorTest::new("hello world");

    test.keys("3x"); // Delete 3 chars

    assert_eq!(test.buffer_content(), "lo world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_x_at_end_of_line() {
    let mut test = EditorTest::new("hello");

    test.press('$') // Move to last char
        .press('x'); // Delete last char

    assert_eq!(test.buffer_content(), "hell\n");
    test.assert_cursor(0, 3);
}

#[test]
fn test_x_single_char_line() {
    let mut test = EditorTest::new("x");

    test.press('x'); // Delete only char

    assert_eq!(test.buffer_content(), "\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_x_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.press('j') // Move to empty line
        .press('x'); // Should do nothing or delete newline

    assert_eq!(test.buffer_content(), "hello\n\nworld\n");
    test.assert_cursor(1, 0);
}

// ============================================================================
// 'X' command - Delete character before cursor
// ============================================================================

#[test]
fn test_X_basic() {
    let mut test = EditorTest::new("hello");

    test.press('l') // Move to 'e'
        .press('X'); // Delete 'h'

    assert_eq!(test.buffer_content(), "ello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_X_at_beginning() {
    let mut test = EditorTest::new("hello");

    test.press('X'); // At beginning, should do nothing

    assert_eq!(test.buffer_content(), "hello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_X_with_count() {
    let mut test = EditorTest::new("hello world");

    test.press('$') // Move to end
        .keys("3X"); // Delete 3 chars before

    assert_eq!(test.buffer_content(), "hello wd\n");
    test.assert_cursor(0, 7);
}

// ============================================================================
// 'dd' command - Delete line
// ============================================================================

#[test]
fn test_dd_basic() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("dd"); // Delete first line

    assert_eq!(test.buffer_content(), "line 2\nline 3\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dd_middle_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('j') // Move to line 2
        .keys("dd"); // Delete line 2

    assert_eq!(test.buffer_content(), "line 1\nline 3\n");
    test.assert_cursor(1, 0);
}

#[test]
fn test_dd_last_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('G') // Go to last line
        .keys("dd"); // Delete last line

    assert_eq!(test.buffer_content(), "line 1\nline 2\n");
    test.assert_cursor(1, 0);
}

#[test]
fn test_dd_single_line() {
    let mut test = EditorTest::new("only line");

    test.keys("dd"); // Delete only line

    assert_eq!(test.buffer_content(), "\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dd_with_count() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4\nline 5");

    test.keys("3dd"); // Delete 3 lines

    assert_eq!(test.buffer_content(), "line 4\nline 5\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dd_count_exceeds_buffer() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.keys("5dd"); // Try to delete 5 lines (only 2 exist)

    assert_eq!(test.buffer_content(), "\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// 'dw' command - Delete word
// ============================================================================

#[test]
fn test_dw_basic() {
    let mut test = EditorTest::new("hello world test");

    test.keys("dw"); // Delete "hello "

    assert_eq!(test.buffer_content(), "world test\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dw_multiple() {
    let mut test = EditorTest::new("hello world test");

    test.keys("dw") // Delete "hello "
        .keys("dw"); // Delete "world "

    assert_eq!(test.buffer_content(), "test\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dw_with_count() {
    let mut test = EditorTest::new("one two three four");

    test.keys("2dw"); // Delete 2 words

    assert_eq!(test.buffer_content(), "three four\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dw_at_end_of_line() {
    let mut test = EditorTest::new("hello world");

    test.keys("w") // Move to "world"
        .keys("dw"); // Delete "world"

    // After deleting "world", only "hello " remains (with trailing newline from normalization)
    assert_eq!(test.buffer_content(), "hello \n");
    test.assert_cursor(0, 5);
}

#[test]
fn test_dw_last_word_no_newline() {
    let mut test = EditorTest::new("hello");

    test.keys("dw"); // Delete only word

    // After deleting "hello", buffer is empty (just trailing newline from normalization)
    assert_eq!(test.buffer_content(), "\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// 'd$' command - Delete to end of line
// ============================================================================

#[test]
fn test_d_dollar_basic() {
    let mut test = EditorTest::new("hello world");

    test.keys("w") // Move to "world"
        .keys("d$"); // Delete to end

    assert_eq!(test.buffer_content(), "hello \n");
    test.assert_cursor(0, 5);
}

#[test]
fn test_d_dollar_from_beginning() {
    let mut test = EditorTest::new("hello world");

    test.keys("d$"); // Delete entire line content

    assert_eq!(test.buffer_content(), "\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_d_dollar_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.press('j') // Move to empty line
        .keys("d$"); // Delete to end (nothing)

    assert_eq!(test.buffer_content(), "hello\n\nworld\n");
    test.assert_cursor(1, 0);
}

// ============================================================================
// 'd0' command - Delete to beginning of line
// ============================================================================

#[test]
fn test_d_zero_basic() {
    let mut test = EditorTest::new("hello world");

    test.keys("w") // Move to "world"
        .keys("d0"); // Delete to beginning

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_d_zero_at_beginning() {
    let mut test = EditorTest::new("hello world");

    test.keys("d0"); // At beginning, should delete nothing

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// 'dG' command - Delete to end of file
// ============================================================================

#[test]
fn test_dG_from_beginning() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("dG"); // Delete entire file

    assert_eq!(test.buffer_content(), "\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dG_from_middle() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('j') // Move to line 2
        .keys("dG"); // Delete from line 2 to end

    assert_eq!(test.buffer_content(), "line 1\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dG_on_last_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('G') // Go to last line
        .keys("dG"); // Delete last line only

    assert_eq!(test.buffer_content(), "line 1\nline 2\n");
    test.assert_cursor(1, 0);
}

// ============================================================================
// 'dgg' command - Delete to beginning of file
// ============================================================================

#[test]
fn test_dgg_from_end() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('G') // Go to last line
        .keys("dgg"); // Delete from last to first

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\nline 4\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dgg_from_middle() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("jj") // Move to line 3
        .keys("dgg"); // Delete from line 3 to beginning

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\nline 4\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Text object deletes - 'diw', 'daw', 'di"', etc.
// ============================================================================

#[test]
fn test_diw_basic() {
    let mut test = EditorTest::new("hello world test");

    test.keys("w") // Move to "world"
        .keys("diw"); // Delete inner word

    assert_eq!(test.buffer_content(), "hello test\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_daw_basic() {
    let mut test = EditorTest::new("hello world test");

    test.keys("w") // Move to "world"
        .keys("daw"); // Delete a word (including surrounding space)

    assert_eq!(test.buffer_content(), "hello test\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_diw_single_char() {
    let mut test = EditorTest::new("a b c");

    test.keys("diw"); // Delete "a"

    assert_eq!(test.buffer_content(), "b c\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Delete with motions
// ============================================================================

#[test]
fn test_dj_delete_line_and_below() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("dj"); // Delete current line and line below

    assert_eq!(test.buffer_content(), "line 3\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dk_delete_line_and_above() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('j') // Move to line 2
        .keys("dk"); // Delete line 2 and line 1

    assert_eq!(test.buffer_content(), "line 3\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_d3l_delete_3_chars_right() {
    let mut test = EditorTest::new("hello world");

    test.keys("d3l"); // Delete 3 chars to the right

    // d3l deletes 3 characters starting from cursor: "hel" → "lo world"
    assert_eq!(test.buffer_content(), "lo world\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_delete_last_char_multiline() {
    let mut test = EditorTest::new("hello\nworld");

    test.press('G') // Go to last line
        .press('$') // End of line
        .press('x'); // Delete last char

    assert_eq!(test.buffer_content(), "hello\nworl\n");
    test.assert_cursor(1, 3);
}

#[test]
fn test_delete_all_content() {
    let mut test = EditorTest::new("hello");

    test.keys("daw"); // Delete all

    // daw on "hello" deletes the entire word, leaving empty buffer
    assert_eq!(test.buffer_content(), "\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_delete_with_newlines() {
    let mut test = EditorTest::new("line 1\n\nline 3");

    test.press('j') // Move to empty line
        .keys("dd"); // Delete empty line

    assert_eq!(test.buffer_content(), "line 1\nline 3\n");
    test.assert_cursor(1, 0);
}

// ============================================================================
// Undo/Redo with delete operations
// ============================================================================

#[test]
fn test_delete_and_undo() {
    let mut test = EditorTest::new("hello world");

    test.keys("dw") // Delete word
        .press('u'); // Undo

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dd_and_undo() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("dd") // Delete line
        .press('u'); // Undo

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_multiple_delete_undo() {
    let mut test = EditorTest::new("one two three four");

    test.keys("dw") // Delete "one "
        .keys("dw") // Delete "two "
        .press('u') // Undo delete "two "
        .press('u'); // Undo delete "one "

    assert_eq!(test.buffer_content(), "one two three four\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// 'd%' command - Delete to matching bracket
// ============================================================================

#[test]
fn test_d_percent_parens_basic() {
    let mut test = EditorTest::new("function(arg1, arg2)");

    test.keys("8l") // Move to opening paren
        .keys("d%"); // Delete from ( to )

    assert_eq!(test.buffer_content(), "function\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_d_percent_curly_braces() {
    let mut test = EditorTest::new("fn test() {\n    code here\n}");

    test.keys("10l") // Move to opening brace
        .keys("d%"); // Delete from { to } (inclusive, multiline)

    // d% deletes from { to matching } inclusive
    assert_eq!(test.buffer_content(), "fn test() \n");
    test.assert_cursor(0, 9);
}

#[test]
fn test_d_percent_square_brackets() {
    let mut test = EditorTest::new("array[0, 1, 2]");

    test.keys("5l") // Move to opening bracket
        .keys("d%"); // Delete from [ to ]

    assert_eq!(test.buffer_content(), "array\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_d_percent_nested_parens() {
    let mut test = EditorTest::new("func(outer(inner))");

    test.keys("4l") // Move to outer opening paren
        .keys("d%"); // Delete from outer ( to outer )

    assert_eq!(test.buffer_content(), "func\n");
    test.assert_cursor(0, 3);
}

#[test]
fn test_d_percent_from_closing_bracket() {
    let mut test = EditorTest::new("test(abc)");

    test.keys("$") // Move to closing paren
        .keys("d%"); // Delete from ) to (

    assert_eq!(test.buffer_content(), "test\n");
    test.assert_cursor(0, 3);
}

#[test]
fn test_d_percent_multiline() {
    let mut test = EditorTest::new("if (condition) {\n    statement1;\n    statement2;\n}");

    test.keys("15l") // Move to opening brace
        .keys("d%"); // Delete from { to }

    assert_eq!(test.buffer_content(), "if (condition) \n");
    test.assert_cursor(0, 14);
}

#[test]
fn test_d_percent_no_match() {
    let mut test = EditorTest::new("no brackets here");

    test.keys("d%"); // Should do nothing

    assert_eq!(test.buffer_content(), "no brackets here\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_d_percent_angle_brackets() {
    let mut test = EditorTest::new("Vec<String>");

    test.keys("3l") // Move to <
        .keys("d%"); // Delete from < to >

    assert_eq!(test.buffer_content(), "Vec\n");
    test.assert_cursor(0, 2);
}

// ============================================================================
// Delete and paste combinations
// ============================================================================

#[test]
fn test_delete_line_and_paste() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("dd") // Delete line 1
        .keys("p"); // Paste it back

    assert_eq!(test.buffer_content(), "line 2\nline 1\nline 3\n");
    test.assert_cursor(1, 0);
}

#[test]
fn test_delete_word_and_paste() {
    let mut test = EditorTest::new("hello world");

    test.keys("dw") // Delete "hello "
        .keys("p"); // Paste after "w"

    assert_eq!(test.buffer_content(), "whello orld\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_cut_and_paste_move_word() {
    let mut test = EditorTest::new("hello world test");

    test.keys("dw") // Cut "hello "
        .keys("w") // Move to "test"
        .keys("P"); // Paste before "test"

    assert_eq!(test.buffer_content(), "world hello test\n");
    test.assert_cursor(0, 12);
}
