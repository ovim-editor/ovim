mod helpers;
use helpers::EditorTest;
use ovim::mode::Mode;

// ============================================================================
// 'i' command - Insert before cursor
// ============================================================================

#[test]
fn test_i_basic() {
    let mut test = EditorTest::new("hello");

    test.press('i').type_text("start ").press_esc();

    assert_eq!(test.buffer_content(), "start hello\n");
    test.assert_cursor(0, 5);
}

#[test]
fn test_i_middle_of_line() {
    let mut test = EditorTest::new("hello world");

    test.keys("w") // Move to 'w' in "world"
        .press('i')
        .type_text("big ")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello big world\n");
    test.assert_cursor(0, 9);
}

#[test]
fn test_i_empty_line() {
    let mut test = EditorTest::new("\n");

    test.press('i').type_text("new text").press_esc();

    assert_eq!(test.buffer_content(), "new text\n");
    test.assert_cursor(0, 7);
}

// ============================================================================
// 'I' command - Insert at beginning of line
// ============================================================================

#[test]
fn test_I_basic() {
    let mut test = EditorTest::new("hello");

    test.press('$') // Move to end
        .press('I') // Should go to beginning
        .type_text("start ")
        .press_esc();

    assert_eq!(test.buffer_content(), "start hello\n");
    test.assert_cursor(0, 5);
}

#[test]
fn test_I_with_indentation() {
    let mut test = EditorTest::new("    indented line");

    test.press('$') // Move to end
        .press('I') // Should go to first non-blank (column 4, before 'i')
        .type_text("prefix ")
        .press_esc();

    // I goes to first non-blank, so "prefix " is inserted at column 4
    assert_eq!(test.buffer_content(), "    prefix indented line\n");
    test.assert_cursor(0, 10); // After "    prefix" (cursor moved back 1 from 11)
}

#[test]
fn test_I_whitespace_only_line() {
    let mut test = EditorTest::new("    \n");

    test.press('I') // On whitespace-only line, I goes to column 0
        .type_text("text")
        .press_esc();

    // The trailing whitespace is preserved
    assert_eq!(test.buffer_content(), "text    \n");
    test.assert_cursor(0, 3);
}

// ============================================================================
// 'a' command - Append after cursor
// ============================================================================

#[test]
fn test_a_basic() {
    let mut test = EditorTest::new("hello");

    test.press('a') // Append after 'h'
        .type_text("X")
        .press_esc();

    assert_eq!(test.buffer_content(), "hXello\n");
    test.assert_cursor(0, 1);
}

#[test]
fn test_a_end_of_line() {
    let mut test = EditorTest::new("hello");

    test.press('$') // Move to end (last char)
        .press('a')
        .type_text(" world")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 10);
}

#[test]
fn test_a_empty_line() {
    let mut test = EditorTest::new("");

    test.press('a').type_text("text").press_esc();

    assert_eq!(test.buffer_content(), "text\n");
    // After Esc from insert mode, cursor lands on the last typed character.
    // "text" has 4 chars (cols 0-3), so cursor should be at col 3.
    test.assert_cursor(0, 3);
}

// ============================================================================
// 'A' command - Append at end of line
// ============================================================================

#[test]
fn test_A_basic() {
    let mut test = EditorTest::new("hello");

    test.press('A').type_text(" world").press_esc();

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 10);
}

#[test]
fn test_A_from_middle() {
    let mut test = EditorTest::new("hello world");

    test.keys("w") // Move to "world"
        .press('A') // Should jump to end
        .type_text("!")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello world!\n");
    test.assert_cursor(0, 11);
}

#[test]
fn test_A_empty_line() {
    let mut test = EditorTest::new("");

    test.press('A').type_text("text").press_esc();

    assert_eq!(test.buffer_content(), "text\n");
    test.assert_cursor(0, 3);
}

// ============================================================================
// 'o' command - Open line below
// ============================================================================

#[test]
fn test_o_basic() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('o').type_text("new line").press_esc();

    assert_eq!(test.buffer_content(), "line 1\nnew line\nline 2\n");
    test.assert_cursor(1, 7);
}

#[test]
fn test_o_last_line_no_newline() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('j') // Move to last line
        .press('o')
        .type_text("line 3")
        .press_esc();

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    test.assert_cursor(2, 5);
}

#[test]
fn test_o_with_spaces_indentation() {
    let mut test = EditorTest::new("start\n    indented\nend");

    test.press('j') // Move to indented line
        .press('o')
        .type_text("same indent")
        .press_esc();

    assert_eq!(
        test.buffer_content(),
        "start\n    indented\n    same indent\nend\n"
    );
    test.assert_cursor(2, 14);
}

#[test]
fn test_o_with_tabs_indentation() {
    let mut test = EditorTest::new("start\n\t\tindented\nend");

    test.press('j') // Move to indented line
        .press('o')
        .type_text("same indent")
        .press_esc();

    assert_eq!(
        test.buffer_content(),
        "start\n\t\tindented\n\t\tsame indent\nend\n"
    );
    test.assert_cursor(2, 12);
}

#[test]
fn test_o_empty_file() {
    let mut test = EditorTest::new("");

    test.press('o').type_text("first line").press_esc();

    assert_eq!(test.buffer_content(), "\nfirst line\n");
    test.assert_cursor(1, 9);
}

#[test]
fn test_o_single_line_no_newline() {
    let mut test = EditorTest::new("hello");

    test.press('o').type_text("world").press_esc();

    assert_eq!(test.buffer_content(), "hello\nworld\n");
    test.assert_cursor(1, 4);
}

#[test]
fn test_o_preserves_position_in_line() {
    let mut test = EditorTest::new("hello world\nnext line");

    test.keys("w") // Move to "world"
        .press('o')
        .press_esc();

    // After 'o', we should be on new line with proper indentation
    // Position in original line shouldn't matter
    assert_eq!(test.buffer_content(), "hello world\n\nnext line\n");
    test.assert_cursor(1, 0);
}

// ============================================================================
// 'O' command - Open line above
// ============================================================================

#[test]
fn test_O_basic() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('j') // Move to line 2
        .press('O')
        .type_text("inserted above")
        .press_esc();

    assert_eq!(test.buffer_content(), "line 1\ninserted above\nline 2\n");
    test.assert_cursor(1, 13);
}

#[test]
fn test_O_first_line() {
    let mut test = EditorTest::new("first line\nsecond line");

    test.press('O').type_text("new first").press_esc();

    assert_eq!(
        test.buffer_content(),
        "new first\nfirst line\nsecond line\n"
    );
    test.assert_cursor(0, 8);
}

#[test]
fn test_O_with_indentation() {
    let mut test = EditorTest::new("start\n    indented\nend");

    test.press('j') // Move to indented line
        .press('O')
        .type_text("same indent")
        .press_esc();

    assert_eq!(
        test.buffer_content(),
        "start\n    same indent\n    indented\nend\n"
    );
    test.assert_cursor(1, 14);
}

#[test]
fn test_O_empty_file() {
    let mut test = EditorTest::new("");

    test.press('O').type_text("first line").press_esc();

    assert_eq!(test.buffer_content(), "first line\n");
    test.assert_cursor(0, 9);
}

#[test]
fn test_O_single_line() {
    let mut test = EditorTest::new("only line");

    test.press('O').type_text("new first").press_esc();

    assert_eq!(test.buffer_content(), "new first\nonly line\n");
    test.assert_cursor(0, 8);
}

// ============================================================================
// Edge cases and combinations
// ============================================================================

#[test]
fn test_multiple_o_commands() {
    let mut test = EditorTest::new("start");

    test.press('o')
        .type_text("line 1")
        .press_esc()
        .press('o')
        .type_text("line 2")
        .press_esc();

    assert_eq!(test.buffer_content(), "start\nline 1\nline 2\n");
    test.assert_cursor(2, 5);
}

#[test]
fn test_o_then_O() {
    let mut test = EditorTest::new("middle");

    test.press('o')
        .type_text("below")
        .press_esc()
        .press('O')
        .type_text("above")
        .press_esc();

    assert_eq!(test.buffer_content(), "middle\nabove\nbelow\n");
    test.assert_cursor(1, 4);
}

#[test]
fn test_i_then_enter() {
    let mut test = EditorTest::new("hello world");

    test.keys("w") // Move to "world"
        .press('i')
        .type_text("big")
        .press_enter()
        .type_text("very ")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello big\nvery world\n");
    test.assert_cursor(1, 4);
}

#[test]
fn test_A_then_enter() {
    let mut test = EditorTest::new("line 1");

    test.press('A')
        .press_enter()
        .type_text("line 2")
        .press_esc();

    assert_eq!(test.buffer_content(), "line 1\nline 2\n");
    test.assert_cursor(1, 5);
}

// ============================================================================
// Undo/Redo with insert operations
// ============================================================================

#[test]
fn test_o_and_undo() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('o').type_text("inserted").press_esc().press('u'); // Undo

    // Undo should restore original content (no blank line)
    assert_eq!(test.buffer_content(), "line 1\nline 2\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_i_and_undo() {
    let mut test = EditorTest::new("hello world");

    test.keys("w")
        .press('i')
        .type_text("big ")
        .press_esc()
        .press('u'); // Undo

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_A_and_undo() {
    let mut test = EditorTest::new("hello");

    test.press('A').type_text(" world").press_esc().press('u'); // Undo

    assert_eq!(test.buffer_content(), "hello\n");
    test.assert_cursor(0, 0);
}
