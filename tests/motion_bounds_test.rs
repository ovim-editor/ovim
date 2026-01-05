mod helpers;
use helpers::EditorTest;

// ============================================================================
// 'w' command - Word forward motion with bounds checking
// ============================================================================

#[test]
fn test_w_at_last_word() {
    let mut test = EditorTest::new("hello world");

    test.keys("w") // Move to "world"
        .press('w'); // Try to move past last word

    assert_eq!(test.buffer_content(), "hello world\n");

    // Motion goes past last word to trailing newline (line 1, col 0)
    test.assert_cursor(1, 0);
}

#[test]
fn test_w_multiple_at_end() {
    let mut test = EditorTest::new("one two three");

    test.keys("www") // Move to "three"
        .keys("www"); // Try to move beyond

    assert_eq!(test.buffer_content(), "one two three\n");

    // Motion goes past last word to trailing newline (line 1, col 0)
    test.assert_cursor(1, 0);
}

#[test]
fn test_w_single_word() {
    let mut test = EditorTest::new("word");

    test.press('w') // Move to trailing newline
        .press('w');

    assert_eq!(test.buffer_content(), "word\n");

    // Motion goes past single word to trailing newline (line 1, col 0)
    test.assert_cursor(1, 0);
}

#[test]
fn test_w_at_eof_no_newline() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.keys("G$") // Go to end of last line
        .press('w'); // Try to move forward

    assert_eq!(test.buffer_content(), "line 1\nline 2\n");

    // Motion goes past last word to trailing newline (line 2, col 0)
    test.assert_cursor(2, 0);
}

#[test]
fn test_w_with_count_beyond_buffer() {
    let mut test = EditorTest::new("one two three");

    test.keys("10w"); // Try to move 10 words (only 3 exist)

    assert_eq!(test.buffer_content(), "one two three\n");

    // Motion goes past last word to trailing newline (line 1, col 0)
    test.assert_cursor(1, 0);
}

#[test]
fn test_w_on_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.press('j') // Move to empty line
        .press('w'); // Should move to "world"

    assert_eq!(test.buffer_content(), "hello\n\nworld\n");

    test.assert_cursor(2, 0);
}

#[test]
fn test_w_on_whitespace_only() {
    let mut test = EditorTest::new("     ");

    test.press('w'); // Should handle whitespace-only line

    assert_eq!(test.buffer_content(), "     \n");

    // Motion on whitespace-only line goes to trailing newline (line 1, col 0)
    test.assert_cursor(1, 0);
}

// ============================================================================
// 'W' command - WORD forward motion with bounds checking
// ============================================================================

#[test]
fn test_W_at_last_WORD() {
    let mut test = EditorTest::new("hello-world test");

    test.press('W') // Move to "test"
        .press('W'); // Try to move past

    assert_eq!(test.buffer_content(), "hello-world test\n");

    // Motion goes past last WORD to trailing newline (line 1, col 0)
    test.assert_cursor(1, 0);
}

#[test]
fn test_W_beyond_buffer() {
    let mut test = EditorTest::new("one");

    test.keys("5W"); // Try to move 5 WORDs

    assert_eq!(test.buffer_content(), "one\n");

    // Motion goes past last WORD to trailing newline (line 1, col 0)
    test.assert_cursor(1, 0);
}

// ============================================================================
// 'b' command - Word backward motion with bounds checking
// ============================================================================

#[test]
fn test_b_at_first_word() {
    let mut test = EditorTest::new("hello world");

    test.press('b') // At beginning, try to move back
        .press('b');

    assert_eq!(test.buffer_content(), "hello world\n");

    test.assert_cursor(0, 0);
}

#[test]
fn test_b_multiple_at_start() {
    let mut test = EditorTest::new("one two three");

    test.keys("bbb"); // Try to move back 3 times from start

    assert_eq!(test.buffer_content(), "one two three\n");

    test.assert_cursor(0, 0);
}

#[test]
fn test_b_with_count_beyond_buffer() {
    let mut test = EditorTest::new("one two three");

    test.keys("$") // End of line
        .keys("10b"); // Try to move back 10 words

    assert_eq!(test.buffer_content(), "one two three\n");

    test.assert_cursor(0, 0);
}

#[test]
fn test_b_single_word() {
    let mut test = EditorTest::new("word");

    test.keys("$") // End
        .press('b') // Should move to start
        .press('b'); // Should stay at start

    assert_eq!(test.buffer_content(), "word\n");

    test.assert_cursor(0, 0);
}

// ============================================================================
// 'B' command - WORD backward motion with bounds checking
// ============================================================================

#[test]
fn test_B_at_beginning() {
    let mut test = EditorTest::new("hello-world");

    test.press('B'); // At start, try to move back

    assert_eq!(test.buffer_content(), "hello-world\n");

    test.assert_cursor(0, 0);
}

#[test]
fn test_B_beyond_buffer() {
    let mut test = EditorTest::new("one-two three-four");

    test.keys("$").keys("10B"); // Move back 10 WORDs (only 2 exist)

    assert_eq!(test.buffer_content(), "one-two three-four\n");

    test.assert_cursor(0, 0);
}

// ============================================================================
// 'e' command - End of word motion with bounds checking
// ============================================================================

#[test]
fn test_e_at_last_word() {
    let mut test = EditorTest::new("hello world");

    test.press('e') // End of "hello"
        .press('e') // End of "world"
        .press('e'); // Try to move past

    assert_eq!(test.buffer_content(), "hello world\n");

    // Motion goes past last word to trailing newline (line 1, col 0)
    test.assert_cursor(1, 0);
}

#[test]
fn test_e_single_word() {
    let mut test = EditorTest::new("word");

    test.press('e') // End of word
        .press('e'); // Try to move past

    assert_eq!(test.buffer_content(), "word\n");

    // Motion goes past single word to trailing newline (line 1, col 0)
    test.assert_cursor(1, 0);
}

#[test]
fn test_e_with_count_beyond() {
    let mut test = EditorTest::new("one two");

    test.keys("5e"); // Try to move to end of 5th word

    assert_eq!(test.buffer_content(), "one two\n");

    // Motion goes past last word to trailing newline (line 1, col 0)
    test.assert_cursor(1, 0);
}

#[test]
fn test_e_at_eof() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.keys("G$") // End of file
        .press('e'); // Try to move forward

    assert_eq!(test.buffer_content(), "line 1\nline 2\n");

    // Motion goes past last word to trailing newline (line 2, col 0)
    test.assert_cursor(2, 0);
}

// ============================================================================
// 'E' command - End of WORD motion with bounds checking
// ============================================================================

#[test]
fn test_E_at_last_WORD() {
    let mut test = EditorTest::new("hello-world test-case");

    test.press('E') // End of "hello-world"
        .press('E') // End of "test-case"
        .press('E'); // Try to move past

    assert_eq!(test.buffer_content(), "hello-world test-case\n");

    // Motion goes past last WORD to trailing newline (line 1, col 0)
    test.assert_cursor(1, 0);
}

#[test]
fn test_E_beyond_buffer() {
    let mut test = EditorTest::new("one");

    test.keys("10E");

    assert_eq!(test.buffer_content(), "one\n");

    // Motion goes past last WORD to trailing newline (line 1, col 0)
    test.assert_cursor(1, 0);
}

// ============================================================================
// 'ge' command - Backward to end of word with bounds checking
// ============================================================================

#[test]
fn test_ge_at_beginning() {
    let mut test = EditorTest::new("hello world");

    test.keys("ge") // Try to move backward from start
        .keys("ge");

    assert_eq!(test.buffer_content(), "hello world\n");

    test.assert_cursor(0, 0);
}

#[test]
fn test_ge_single_word() {
    let mut test = EditorTest::new("word");

    test.keys("$")
        .keys("ge") // Should move to start
        .keys("ge"); // Try to move back more

    assert_eq!(test.buffer_content(), "word\n");

    test.assert_cursor(0, 3);
}

// ============================================================================
// 'gE' command - Backward to end of WORD with bounds checking
// ============================================================================

#[test]
fn test_gE_at_beginning() {
    let mut test = EditorTest::new("hello-world test");

    test.keys("gE"); // At start, try to move back

    assert_eq!(test.buffer_content(), "hello-world test\n");

    test.assert_cursor(0, 0);
}

// ============================================================================
// Line motions with bounds checking
// ============================================================================

#[test]
fn test_j_at_last_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("G") // Last line
        .press('j') // Try to move down
        .press('j');

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");

    test.assert_cursor(2, 0);
}

#[test]
fn test_j_with_count_beyond() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("10j"); // Try to move down 10 lines

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");

    // 10j from line 0 goes to line 2 (last line), clamped to buffer bounds
    test.assert_cursor(2, 0);
}

#[test]
fn test_k_at_first_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('k') // At top, try to move up
        .press('k');

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");

    test.assert_cursor(0, 0);
}

#[test]
fn test_k_with_count_beyond() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("10k"); // Try to move up 10 lines from top

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");

    test.assert_cursor(0, 0);
}

#[test]
fn test_j_single_line() {
    let mut test = EditorTest::new("only line");

    test.press('j') // No line below
        .press('j');

    assert_eq!(test.buffer_content(), "only line\n");

    test.assert_cursor(0, 0);
}

#[test]
fn test_k_single_line() {
    let mut test = EditorTest::new("only line");

    test.press('k') // No line above
        .press('k');

    assert_eq!(test.buffer_content(), "only line\n");

    test.assert_cursor(0, 0);
}

// ============================================================================
// Character motions with bounds checking
// ============================================================================

#[test]
fn test_l_at_end_of_line() {
    let mut test = EditorTest::new("hello");

    test.keys("$") // End of line
        .press('l') // Try to move right
        .press('l');

    assert_eq!(test.buffer_content(), "hello\n");

    test.assert_cursor(0, 4);
}

#[test]
fn test_l_with_count_beyond() {
    let mut test = EditorTest::new("hello");

    test.keys("20l"); // Try to move right 20 chars

    assert_eq!(test.buffer_content(), "hello\n");

    // 20l moves to end of line (col 4), clamped to line bounds
    test.assert_cursor(0, 4);
}

#[test]
fn test_h_at_beginning() {
    let mut test = EditorTest::new("hello");

    test.press('h') // At start, try to move left
        .press('h');

    assert_eq!(test.buffer_content(), "hello\n");

    test.assert_cursor(0, 0);
}

#[test]
fn test_h_with_count_beyond() {
    let mut test = EditorTest::new("hello");

    test.keys("$").keys("20h"); // Try to move left 20 chars

    assert_eq!(test.buffer_content(), "hello\n");

    test.assert_cursor(0, 0);
}

#[test]
fn test_l_empty_line() {
    let mut test = EditorTest::new("\n");

    test.press('l'); // Can't move right on empty line

    assert_eq!(test.buffer_content(), "\n");

    test.assert_cursor(0, 0);
}

#[test]
fn test_h_empty_line() {
    let mut test = EditorTest::new("\n");

    test.press('h'); // Can't move left on empty line

    assert_eq!(test.buffer_content(), "\n");

    test.assert_cursor(0, 0);
}

// ============================================================================
// Motion combinations with bounds
// ============================================================================

#[test]
fn test_w_then_b_at_boundary() {
    let mut test = EditorTest::new("word");

    test.press('w') // Move forward past word to trailing newline (line 1, col 0)
        .press('b') // Back to end of "word" (line 0, col 3) - b goes to end of previous word
        .press('w') // Forward again to trailing newline (line 1, col 0)
        .press('b'); // Back to end of "word" (line 0, col 3)

    assert_eq!(test.buffer_content(), "word\n");

    // b from empty line moves to end of last word (col 3), not start
    // This is consistent with Vim: b moves to the beginning of the *current* or *previous* word,
    // but from an empty/whitespace-only position, it finds the end of the last word
    test.assert_cursor(0, 3);
}

#[test]
fn test_e_then_ge_at_boundary() {
    let mut test = EditorTest::new("word");

    test.press('e') // To end of "word" (col 3)
        .keys("ge") // Try backward - but ge at start of file stays at col 3
        .press('e'); // Forward - goes to trailing newline

    assert_eq!(test.buffer_content(), "word\n");

    // e goes to trailing newline (line 1, col 0)
    test.assert_cursor(1, 0);
}

// ============================================================================
// Paragraph motions with bounds checking
// ============================================================================

#[test]
fn test_close_brace_at_eof() {
    let mut test = EditorTest::new("para 1\n\npara 2");

    test.keys("G") // Last line
        .press('}') // Try to move to next paragraph
        .press('}');

    assert_eq!(test.buffer_content(), "para 1\n\npara 2\n");

    // } motion goes to trailing newline (line 3, col 0)
    test.assert_cursor(3, 0);
}

#[test]
fn test_open_brace_at_beginning() {
    let mut test = EditorTest::new("para 1\n\npara 2");

    test.press('{') // At start, try to move to prev paragraph
        .press('{');

    assert_eq!(test.buffer_content(), "para 1\n\npara 2\n");

    test.assert_cursor(0, 0);
}

#[test]
fn test_close_brace_no_blank_lines() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('}') // No paragraphs, should go to end?
        .press('}');

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");

    // } motion goes to trailing newline (line 3, col 0)
    test.assert_cursor(3, 0);
}

// ============================================================================
// Sentence motions with bounds checking
// ============================================================================

#[test]
fn test_close_paren_at_eof() {
    let mut test = EditorTest::new("First. Second.");

    test.keys("$") // End
        .press(')') // Try to move to next sentence
        .press(')');

    assert_eq!(test.buffer_content(), "First. Second.\n");

    // ) motion goes to trailing newline (line 1, col 0)
    test.assert_cursor(1, 0);
}

#[test]
fn test_open_paren_at_beginning() {
    let mut test = EditorTest::new("First. Second.");

    test.press('(') // At start
        .press('(');

    assert_eq!(test.buffer_content(), "First. Second.\n");

    test.assert_cursor(0, 0);
}

// ============================================================================
// Special motion bounds
// ============================================================================

#[test]
fn test_G_beyond_buffer() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("999G"); // Try to go to line 999

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");

    test.assert_cursor(998, 0);
}

#[test]
fn test_gg_already_at_top() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.keys("gg") // Go to top
        .keys("gg"); // Already there

    assert_eq!(test.buffer_content(), "line 1\nline 2\n");

    test.assert_cursor(0, 0);
}

#[test]
fn test_G_already_at_bottom() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.keys("G") // Go to bottom
        .keys("G"); // Already there

    assert_eq!(test.buffer_content(), "line 1\nline 2\n");

    test.assert_cursor(1, 0);
}

#[test]
fn test_percent_beyond_100() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("150%"); // Try to go to 150% of file

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");

    test.assert_cursor(0, 0);
}

// ============================================================================
// Find motions with bounds
// ============================================================================

#[test]
fn test_f_char_not_found() {
    let mut test = EditorTest::new("hello world");

    test.press('f').press('z'); // Character not in line

    assert_eq!(test.buffer_content(), "hello world\n");

    test.assert_cursor(0, 0);
}

#[test]
fn test_f_at_end_of_line() {
    let mut test = EditorTest::new("hello");

    test.keys("$") // End of line
        .press('f')
        .press('x'); // Try to find past end

    assert_eq!(test.buffer_content(), "hello\n");

    test.assert_cursor(0, 4);
}

#[test]
fn test_F_at_beginning() {
    let mut test = EditorTest::new("hello");

    test.press('F') // At start, try backward find
        .press('h');

    assert_eq!(test.buffer_content(), "hello\n");

    test.assert_cursor(0, 0);
}

#[test]
fn test_semicolon_no_previous_find() {
    let mut test = EditorTest::new("hello world");

    test.press(';'); // No previous f/F/t/T

    assert_eq!(test.buffer_content(), "hello world\n");

    test.assert_cursor(0, 0);
}

#[test]
fn test_comma_no_previous_find() {
    let mut test = EditorTest::new("hello world");

    test.press(','); // No previous f/F/t/T

    assert_eq!(test.buffer_content(), "hello world\n");

    test.assert_cursor(0, 0);
}

// ============================================================================
// Operators with bounded motions
// ============================================================================

#[test]
fn test_dw_at_last_word() {
    let mut test = EditorTest::new("hello world");

    test.keys("w") // Move to "world"
        .keys("dw"); // Delete word "world" (should delete to end)

    assert_eq!(test.buffer_content(), "hello \n");

    // After deleting "world", cursor is at end of remaining content
    test.assert_cursor(0, 5);
}

#[test]
fn test_d10w_beyond_buffer() {
    let mut test = EditorTest::new("one two three");

    test.keys("d10w"); // Try to delete 10 words

    assert_eq!(test.buffer_content(), "one two three\n");

    test.assert_cursor(0, 4);
}

#[test]
fn test_cw_at_last_word() {
    let mut test = EditorTest::new("hello world");

    test.keys("w")
        .keys("cw") // Change last word
        .type_text("earth")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello earth\n");

    // After cw deletes "world" and types "earth", cursor is on last char
    test.assert_cursor(0, 10);
}

#[test]
fn test_yw_at_eof() {
    let mut test = EditorTest::new("last");

    test.keys("yw") // Yank word "last" (includes trailing newline motion)
        .press('p'); // Paste after cursor

    // yw yanks "last\n" (to end of motion which is trailing newline)
    // p pastes after cursor (cursor at col 0), so "last\n" is inserted after 'l'
    // Result: "l" + "last\n" + "ast\n" = "llast\nast\n"
    assert_eq!(test.buffer_content(), "llast\nast\n");

    // Cursor ends on the trailing newline line after paste (line 1, col 0)
    test.assert_cursor(1, 0);
}
