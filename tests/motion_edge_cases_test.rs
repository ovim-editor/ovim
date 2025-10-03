mod helpers;
use helpers::EditorTest;
use insta::assert_snapshot;

// ============================================================================
// Motion edge cases - Buffer boundaries
// ============================================================================

#[test]
fn test_j_at_last_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("G")        // Go to last line
        .press('j')       // Try to move down (should stay)
        .press('j');      // Try again

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_k_at_first_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('k')       // Try to move up (should stay)
        .press('k');      // Try again

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_l_at_end_of_line() {
    let mut test = EditorTest::new("hello");

    test.keys("$")        // Go to last char
        .press('l')       // Try to move right (should stay)
        .press('l');      // Try again

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_h_at_start_of_line() {
    let mut test = EditorTest::new("hello world");

    test.press('h')       // Try to move left (should stay)
        .press('h');      // Try again

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_w_at_last_word() {
    let mut test = EditorTest::new("hello world");

    test.keys("w")        // Move to "world"
        .press('w')       // Try to move forward (should stay at end)
        .press('w');      // Try again

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_b_at_first_word() {
    let mut test = EditorTest::new("hello world");

    test.press('b')       // Try to move backward (should stay)
        .press('b');      // Try again

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_e_at_last_char() {
    let mut test = EditorTest::new("hello");

    test.keys("$")        // Last char
        .press('e')       // Try to move to end of word
        .press('e');      // Try again

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_gg_on_first_line() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.keys("gg");      // Already on first line

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_G_on_last_line() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.keys("G")        // Go to last
        .press('G');      // Already there

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Motion edge cases - Empty lines
// ============================================================================

#[test]
fn test_w_on_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.press('j')       // Move to empty line
        .press('w');      // Should move to next line with content

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_b_from_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.keys("jj")       // Line 2 (world)
        .press('b');      // Should move back past empty line

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_e_on_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.press('j')       // Empty line
        .press('e');      // Should move to next word end

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dollar_on_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.press('j')       // Empty line
        .keys("$");       // Should stay at column 0

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_zero_on_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.press('j')       // Empty line
        .keys("0");       // Should stay at column 0

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_caret_on_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.press('j')       // Empty line
        .press('^');      // Should stay at column 0

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_multiple_consecutive_empty_lines() {
    let mut test = EditorTest::new("hello\n\n\n\nworld");

    test.press('w')       // Should skip all empty lines
        .press('w');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Motion edge cases - Whitespace and special characters
// ============================================================================

#[test]
fn test_w_only_whitespace() {
    let mut test = EditorTest::new("hello     world");

    test.press('w');      // Should skip all whitespace

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_w_tabs() {
    let mut test = EditorTest::new("hello\t\tworld");

    test.press('w');      // Should handle tabs

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_w_mixed_whitespace() {
    let mut test = EditorTest::new("hello \t \t world");

    test.press('w');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_w_punctuation() {
    let mut test = EditorTest::new("hello...world");

    test.press('w');      // Should stop at punctuation

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_w_vs_W_punctuation() {
    // Test w (stops at punctuation)
    let mut test_w = EditorTest::new("hello.world test");
    test_w.press('w');

    // Test W (treats punctuation as part of WORD)
    let mut test_W = EditorTest::new("hello.world test");
    test_W.press('W');

    assert_snapshot!("w_motion", test_w.snapshot_state());
    assert_snapshot!("W_motion", test_W.snapshot_state());
}

#[test]
fn test_e_vs_E_punctuation() {
    let mut test = EditorTest::new("hello.world test");

    test.press('e');      // Should stop at end of "hello"

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_b_vs_B_punctuation() {
    let mut test = EditorTest::new("hello.world test");

    test.keys("$")        // End of line
        .press('b');      // Back to "test"

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Motion edge cases - Line wrapping and column preservation
// ============================================================================

#[test]
fn test_j_preserves_column() {
    let mut test = EditorTest::new("hello world test\nshort\nhello again test");

    test.keys("$")        // End of first line
        .press('j')       // Down to short line (column clamped)
        .press('j');      // Down to long line (column restored)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_k_preserves_column() {
    let mut test = EditorTest::new("hello world test\nshort\nhello again test");

    test.keys("Gj$")      // Last line, end
        .press('k')       // Up to short (clamped)
        .press('k');      // Up to long (restored)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_j_to_shorter_line() {
    let mut test = EditorTest::new("hello world\nhi\ntest");

    test.keys("$")        // End of first line (col 10)
        .press('j');      // Down to "hi" (should clamp to col 1)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_j_from_short_to_long() {
    let mut test = EditorTest::new("hi\nhello world");

    test.keys("$")        // End of "hi"
        .press('j');      // Down to longer line

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Motion edge cases - Large counts
// ============================================================================

#[test]
fn test_j_count_exceeds_buffer() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("999j");    // Try to move down 999 lines

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_k_count_exceeds_buffer() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("G")        // Last line
        .keys("999k");    // Try to move up 999 lines

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_w_count_exceeds_words() {
    let mut test = EditorTest::new("one two three");

    test.keys("99w");     // Try to move forward 99 words

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_b_count_exceeds_words() {
    let mut test = EditorTest::new("one two three");

    test.keys("$")
        .keys("99b");     // Try to move back 99 words

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_l_count_exceeds_line() {
    let mut test = EditorTest::new("hello");

    test.keys("99l");     // Try to move right 99 chars

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_h_count_exceeds_line() {
    let mut test = EditorTest::new("hello world");

    test.keys("$")
        .keys("99h");     // Try to move left 99 chars

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Motion edge cases - Counts with specific values
// ============================================================================

#[test]
fn test_10j() {
    let mut test = EditorTest::new("line 0\nline 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\nline 11");

    test.keys("10j");     // Move down exactly 10 lines

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_5w() {
    let mut test = EditorTest::new("one two three four five six seven");

    test.keys("5w");      // Move forward 5 words

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_3e() {
    let mut test = EditorTest::new("one two three four five");

    test.keys("3e");      // Move to end of 3rd word

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_4b() {
    let mut test = EditorTest::new("one two three four five six");

    test.keys("$")        // End
        .keys("4b");      // Back 4 words

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Motion edge cases - Zero count (special case)
// ============================================================================

#[test]
fn test_0_is_motion_not_count() {
    let mut test = EditorTest::new("hello world");

    test.keys("w")        // Move to "world"
        .keys("0");       // Should go to beginning of line, not count

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Motion edge cases - Percentage motion
// ============================================================================

#[test]
fn test_50_percent() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10");

    test.keys("50%");     // Go to 50% of file

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_100_percent() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("100%");    // Go to 100% (last line)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_1_percent() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("G")        // Last line
        .keys("1%");      // Go to 1% (first line)

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Motion edge cases - Line number jumps
// ============================================================================

#[test]
fn test_line_number_jump() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4\nline 5");

    test.keys("3G");      // Go to line 3

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_line_number_exceeds_buffer() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("999G");    // Try to go to line 999

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_1G() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("G")        // Last line
        .keys("1G");      // Go to line 1

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Motion edge cases - Sentence and paragraph motions
// ============================================================================

#[test]
fn test_closing_paren_motion_no_match() {
    let mut test = EditorTest::new("hello world");

    test.press('%');      // No matching paren

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_closing_paren_nested() {
    let mut test = EditorTest::new("outer(inner(deep))");

    test.press('%');      // Should match outer parens

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_closing_paren_from_middle() {
    let mut test = EditorTest::new("func(arg1, arg2)");

    test.keys("f,")       // Move to comma
        .press('%');      // Should find closing paren

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Motion edge cases - Special characters in motions
// ============================================================================

#[test]
fn test_f_space() {
    let mut test = EditorTest::new("hello world test");

    test.press('f')
        .press(' ');      // Find space

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_f_tab() {
    let mut test = EditorTest::new("hello\tworld");

    test.press('f')
        .press('\t');     // Find tab (if supported)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_F_last_char() {
    let mut test = EditorTest::new("hello world");

    test.keys("$")        // Last char
        .press('F')
        .press('h');      // Find 'h' backward

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_t_last_char() {
    let mut test = EditorTest::new("hello world");

    test.press('t')
        .press('d');      // Till 'd' (last char)

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Motion edge cases - Single character lines
// ============================================================================

#[test]
fn test_w_on_single_char() {
    let mut test = EditorTest::new("a");

    test.press('w');      // Should not move or go to next line

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_e_on_single_char() {
    let mut test = EditorTest::new("a");

    test.press('e');      // Already at end

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dollar_on_single_char() {
    let mut test = EditorTest::new("x");

    test.keys("$");       // Should be at the char

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Motion edge cases - Very long lines
// ============================================================================

#[test]
fn test_w_very_long_line() {
    let mut test = EditorTest::new("word ".repeat(100).trim());

    test.keys("50w");     // Move forward 50 words

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dollar_very_long_line() {
    let mut test = EditorTest::new(&"x".repeat(1000));

    test.keys("$");       // Go to end of very long line

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Motion edge cases - Empty buffer
// ============================================================================

#[test]
fn test_motions_on_empty_buffer() {
    let mut test = EditorTest::new("");

    test.press('j')       // Down
        .press('k')       // Up
        .press('w')       // Word forward
        .press('b')       // Word back
        .keys("$")        // End
        .keys("0");       // Beginning

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_gg_on_empty_buffer() {
    let mut test = EditorTest::new("");

    test.keys("gg");

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_G_on_empty_buffer() {
    let mut test = EditorTest::new("");

    test.keys("G");

    assert_snapshot!(test.snapshot_state());
}
