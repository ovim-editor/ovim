mod helpers;
use helpers::EditorTest;

// ============================================================================
// Dot (.) command - Repeat last change
// ============================================================================

#[test]
fn test_dot_repeat_delete_char() {
    let mut test = EditorTest::new("hello");

    test.press('x') // Delete 'h'
        .press('.'); // Repeat (delete 'e')

    assert_eq!(test.buffer_content(), "llo\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dot_repeat_delete_word() {
    let mut test = EditorTest::new("one two three four");

    test.keys("dw") // Delete "one "
        .press('.'); // Repeat (delete "two ")

    assert_eq!(test.buffer_content(), "three four\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dot_repeat_delete_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("dd") // Delete line 1
        .press('.'); // Repeat (delete line 2)

    assert_eq!(test.buffer_content(), "line 3\nline 4\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dot_repeat_insert() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('i')
        .type_text("PREFIX:")
        .press_esc()
        .press('j') // Next line
        .press('.'); // Repeat insert

    assert_eq!(test.buffer_content(), "PREFIX:line 1\nline PREFIX:2\n");
    test.assert_cursor(1, 11);
}

#[test]
fn test_dot_repeat_append() {
    let mut test = EditorTest::new("hello\nworld");

    test.press('a')
        .type_text("!")
        .press_esc()
        .press('j') // Next line
        .press('.'); // Repeat append

    assert_eq!(test.buffer_content(), "h!ello\nw!orld\n");
    test.assert_cursor(1, 2);
}

#[test]
fn test_dot_repeat_change_word() {
    let mut test = EditorTest::new("one two three");

    test.keys("ciw") // Change "one" to "X"
        .type_text("X")
        .press_esc()
        .keys("w") // Move to "two"
        .press('.'); // Repeat: change "two" to "X"

    // ciw changes only the word, not surrounding whitespace
    assert_eq!(test.buffer_content(), "X X three\n");
    test.assert_cursor(0, 2);
}

#[test]
fn test_dot_repeat_substitute() {
    let mut test = EditorTest::new("hello world");

    test.press('s') // Substitute 'h'
        .type_text("H")
        .press_esc()
        .keys("w") // Move to 'w'
        .press('.'); // Repeat substitute

    assert_eq!(test.buffer_content(), "Hello Hworld\n");
    test.assert_cursor(0, 7);
}

// ============================================================================
// Dot with counts
// ============================================================================

#[test]
fn test_dot_with_count() {
    let mut test = EditorTest::new("abcdefgh");

    test.press('x') // Delete one char
        .keys("3."); // Repeat 3 times

    assert_eq!(test.buffer_content(), "cdefgh\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_original_count_vs_repeat_count() {
    let mut test = EditorTest::new("one two three four five six");

    test.keys("2dw") // Delete 2 words
        .press('.'); // Repeat (should delete 2 more words)

    assert_eq!(test.buffer_content(), "ur five six\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dot_override_original_count() {
    let mut test = EditorTest::new("one two three four five six");

    test.keys("2dw") // Delete 2 words
        .keys("3."); // Repeat with different count

    assert_eq!(test.buffer_content(), "ur five six\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dot_repeat_counted_insert() {
    let mut test = EditorTest::new("line");

    test.press('i').type_text("X").press_esc().keys("3."); // Repeat 3 times

    assert_eq!(test.buffer_content(), "XXline\n");
    test.assert_cursor(0, 1);
}

// ============================================================================
// Dot with various operators
// ============================================================================

#[test]
fn test_dot_repeat_yank_then_change() {
    let mut test = EditorTest::new("one two three");

    test.keys("yiw") // Yank doesn't count as change
        .keys("ciw") // Change word "one" to "X"
        .type_text("X")
        .press_esc()
        .keys("w") // Move to "two"
        .press('.'); // Should repeat change: ciw "X" on "two"

    // ciw preserves whitespace - "one" becomes "X", "two" becomes "X"
    assert_eq!(test.buffer_content(), "X X three\n");
    test.assert_cursor(0, 2);
}

#[test]
fn test_dot_repeat_d_dollar() {
    let mut test = EditorTest::new("hello world\ntest case");

    test.keys("d$") // Delete to end of line
        .press('j') // Next line
        .press('.'); // Repeat

    assert_eq!(test.buffer_content(), "\n\n");
    test.assert_cursor(1, 0);
}

#[test]
fn test_dot_repeat_c_dollar() {
    let mut test = EditorTest::new("hello world\ntest case");

    test.keys("c$") // Change to end
        .type_text("NEW")
        .press_esc()
        .press('j')
        .press('.'); // Repeat

    assert_eq!(test.buffer_content(), "NEW\nteNEWst case\n");
    test.assert_cursor(1, 4);
}

#[test]
fn test_dot_repeat_upper_case_X() {
    let mut test = EditorTest::new("hello");

    test.keys("$") // End (cursor on 'o')
        .press('X') // Delete char before cursor ('l'), leaving "helo", cursor on 'o'
        .press('.'); // Repeat (delete char before cursor 'l'), leaving "heo", cursor on 'e'

    assert_eq!(test.buffer_content(), "heo\n");
    test.assert_cursor(0, 1);
}

#[test]
fn test_dot_repeat_J_join() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('J') // Join line 1 and 2
        .press('.'); // Repeat

    assert_eq!(test.buffer_content(), "line 1 line 2 line 3\nline 4\n");
    test.assert_cursor(0, 13);
}

// ============================================================================
// Dot with text objects
// ============================================================================

#[test]
fn test_dot_repeat_diw() {
    let mut test = EditorTest::new("one two three four");

    // Note: iw does NOT include trailing whitespace (that's aw)
    test.keys("diw") // Delete "one" → " two three four" (cursor at 0)
        .keys("w") // Move to next word "two" (cursor at 1)
        .press('.'); // Repeat diw: delete "two" → "  three four"

    // Semantic repeat: re-evaluates inner word at cursor, deletes entire word
    assert_eq!(test.buffer_content(), "  three four\n");
    test.assert_cursor(0, 1);
}

#[test]
fn test_dot_repeat_daw() {
    let mut test = EditorTest::new("one two three four");

    test.keys("daw").press('.'); // Repeat on next word

    assert_eq!(test.buffer_content(), "three four\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dot_repeat_ci_quote() {
    let mut test = EditorTest::new(r#""hello" and "world""#);

    test.keys("f\"") // Find first quote (opening quote of "hello")
        .keys("ci\"") // Change inside quotes
        .type_text("X")
        .press_esc()
        .keys("f\"") // Find next quote (closing quote of "X", not opening of "world"!)
        .press('.'); // Repeat (finds same "X" pair, replaces X with X)

    // In Vim: f" from inside "X" finds the closing quote of "X", so ci" operates on "X" again
    assert_eq!(test.buffer_content(), "\"X\" and \"world\"\n");
    test.assert_cursor(0, 1); // On the X after ci" repeat
}

#[test]
fn test_dot_repeat_di_paren() {
    let mut test = EditorTest::new("func(arg1) and func(arg2)");

    test.keys("f(")
        .keys("di(") // Delete inside parens
        .keys("f(") // Next parens
        .press('.'); // Repeat

    assert_eq!(test.buffer_content(), "func() and func()\n");
    test.assert_cursor(0, 16); // On closing paren after deletion
}

// ============================================================================
// Dot with visual mode operations
// ============================================================================

#[test]
fn test_dot_after_visual_delete() {
    let mut test = EditorTest::new("hello world test");

    test.press('v')
        .keys("e") // Select word
        .press('d') // Delete
        .press('w') // Move to next word
        .press('.'); // Repeat (should work?)

    assert_eq!(test.buffer_content(), "  test\n");
    test.assert_cursor(0, 1);
}

#[test]
fn test_dot_after_visual_change() {
    let mut test = EditorTest::new("one two three");

    test.press('v')
        .keys("e") // Select "one"
        .press('c') // Change
        .type_text("X")
        .press_esc()
        .press('w')
        .press('.'); // Repeat

    assert_eq!(test.buffer_content(), "X Xtwo three\n");
    test.assert_cursor(0, 3);
}

#[test]
fn test_dot_after_visual_line_delete() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('V') // Visual line
        .press('d') // Delete
        .press('.'); // Repeat

    assert_eq!(test.buffer_content(), "line 3\nline 4\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Dot with motion variations
// ============================================================================

#[test]
fn test_dot_repeat_dw_different_positions() {
    let mut test = EditorTest::new("one two three four five");

    test.keys("dw") // Delete "one "
        .keys("w") // Move to "three"
        .press('.'); // Delete "three "

    assert_eq!(test.buffer_content(), "two e four five\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_dot_repeat_cw_at_different_word_lengths() {
    let mut test = EditorTest::new("short longerword");

    test.keys("cw") // Change "short"
        .type_text("X")
        .press_esc()
        .keys("w") // Move to "longerword"
        .press('.'); // Repeat (should change "longerword")

    assert_eq!(test.buffer_content(), "X X\n");
    test.assert_cursor(0, 2);
}

// ============================================================================
// Dot repeat edge cases
// ============================================================================

#[test]
fn test_dot_without_previous_change() {
    let mut test = EditorTest::new("hello");

    test.press('.'); // No previous change

    assert_eq!(test.buffer_content(), "hello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dot_after_movement_only() {
    let mut test = EditorTest::new("hello world");

    test.keys("w") // Just move
        .press('.'); // No change to repeat

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_dot_across_lines() {
    let mut test = EditorTest::new("hello\nworld\ntest");

    test.press('x') // Delete char
        .press('j') // Next line
        .press('.') // Repeat
        .press('j') // Next line
        .press('.'); // Repeat again

    assert_eq!(test.buffer_content(), "ello\norld\nest\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_dot_at_end_of_line() {
    let mut test = EditorTest::new("hello");

    test.keys("$") // End
        .press('x') // Delete last char
        .press('.'); // Repeat (nothing to delete)

    assert_eq!(test.buffer_content(), "hel\n");
    test.assert_cursor(0, 3);
}

#[test]
fn test_dot_after_failed_operation() {
    let mut test = EditorTest::new("x");

    test.press('x') // Delete 'x'
        .press('.'); // Try to repeat (nothing to delete)

    assert_eq!(test.buffer_content(), "\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Dot with insert mode variations
// ============================================================================

#[test]
fn test_dot_repeat_o_command() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('o') // Open line below
        .type_text("new")
        .press_esc()
        .press('j') // Move down
        .press('.'); // Repeat

    assert_eq!(test.buffer_content(), "line 1\nnew\nline 2\nnew\n");
    test.assert_cursor(3, 2);
}

#[test]
fn test_dot_repeat_O_command() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('O') // Open line above
        .type_text("new")
        .press_esc()
        .press('j')
        .press('.'); // Repeat

    assert_eq!(test.buffer_content(), "new\nnew\nline 1\nline 2\n");
    test.assert_cursor(1, 2);
}

#[test]
fn test_dot_repeat_A_command() {
    let mut test = EditorTest::new("hello\nworld");

    test.press('A') // Append at end
        .type_text("!")
        .press_esc()
        .press('j')
        .press('.'); // Repeat

    assert_eq!(test.buffer_content(), "hello!\nworl!d\n");
    test.assert_cursor(1, 5);
}

#[test]
fn test_dot_repeat_I_command() {
    let mut test = EditorTest::new("hello\nworld");

    test.press('I') // Insert at beginning
        .type_text("START:")
        .press_esc()
        .press('j')
        .press('.'); // Repeat

    // I repeats at first non-blank of the line, not at cursor position
    assert_eq!(test.buffer_content(), "START:hello\nSTART:world\n");
    test.assert_cursor(1, 5);
}

// ============================================================================
// Dot with replace mode
// ============================================================================

#[test]
fn test_dot_repeat_r_command() {
    let mut test = EditorTest::new("hello world");

    test.press('r') // Replace char (h -> X)
        .press('X')
        .press('l') // Move right
        .press('.'); // Repeat (e -> X)

    assert_eq!(test.buffer_content(), "XXllo world\n");
    test.assert_cursor(0, 1);
}

#[test]
fn test_dot_repeat_R_command() {
    let mut test = EditorTest::new("hello world");

    test.press('R') // Replace mode (replaces "he" -> "HI")
        .type_text("HI")
        .press_esc()
        .press('w') // Move to "world"
        .press('.'); // Repeat (replaces "wo" -> "HI")

    assert_eq!(test.buffer_content(), "HIllo HIrld\n");
    test.assert_cursor(0, 7);
}

// ============================================================================
// Complex dot repeat scenarios
// ============================================================================

#[test]
fn test_dot_repeat_multiple_times() {
    let mut test = EditorTest::new("abcdefghijkl");

    test.press('x') // Delete 'a'
        .press('.') // Delete 'b'
        .press('.') // Delete 'c'
        .press('.') // Delete 'd'
        .press('.'); // Delete 'e'

    assert_eq!(test.buffer_content(), "fghijkl\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dot_changes_after_different_operation() {
    let mut test = EditorTest::new("one two three four");

    test.press('x') // Delete char
        .keys("dw") // Delete word (new change)
        .press('.'); // Should repeat dw, not x

    assert_eq!(test.buffer_content(), " three four\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dot_with_undo() {
    let mut test = EditorTest::new("hello");

    test.press('x') // Delete
        .press('u') // Undo
        .press('.'); // Repeat (should work)

    assert_eq!(test.buffer_content(), "ello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dot_after_undo_redo() {
    let mut test = EditorTest::new("hello");

    test.press('x') // Delete 'h'
        .press('u') // Undo
        .press_with(
            ovim_core::KeyCode::Char('r'),
            ovim_core::Modifiers::CONTROL,
        ) // Redo
        .press('.'); // Repeat

    assert_eq!(test.buffer_content(), "llo\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Dot repeat + undo interaction
// ============================================================================

#[test]
fn test_dot_repeat_X_undo_restores_correctly() {
    // Regression: $X.uu was turning "hello" into "helll" because the repeated
    // change had stale range/deleted_text from the original X, not the repeat.
    let mut test = EditorTest::new("hello");

    test.keys("$") // Cursor on 'o' (col 4)
        .press('X') // Delete 'l' before cursor → "helo"
        .press('.') // Repeat X → "heo"
        .press('u') // Undo repeat → "helo"
        .press('u'); // Undo original X → "hello"

    assert_eq!(test.buffer_content(), "hello\n");
    test.assert_cursor(0, 4);
}

// ============================================================================
// Dot with search motions
// ============================================================================

#[test]
fn test_dot_with_search_motion() {
    let mut test = EditorTest::new("hello world hello test");

    test.keys("d/world") // Delete to "world"
        .press_enter()
        .press('.'); // Repeat (delete to next match?)

    assert_eq!(test.buffer_content(), "hello world hello test\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_dot_with_f_motion() {
    let mut test = EditorTest::new("a b c d e f");

    // dfc deletes from cursor to 'c' inclusive
    // Starting at position 0, finds 'c' at position 4, deletes "a b c" (positions 0-4)
    // Result: " d e f" (starting with space, which was after 'c')
    test.keys("dfc"); // Delete to 'c'
    assert_eq!(test.buffer_content(), " d e f\n");
    test.assert_cursor(0, 0);

    // . tries to repeat dfc, but there's no 'c' to find, so nothing happens
    test.press('.');
    assert_eq!(test.buffer_content(), " d e f\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Dot preserving count
// ============================================================================

#[test]
fn test_dot_preserves_original_count() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4\nline 5");

    test.keys("2dd") // Delete 2 lines
        .press('.'); // Should delete 2 more lines

    assert_eq!(test.buffer_content(), "line 5\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dot_with_multichar_insert() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('i')
        .type_text("LONG TEXT ")
        .press_esc()
        .press('j')
        .press('.'); // Should insert same text

    assert_eq!(
        test.buffer_content(),
        "LONG TEXT line 1\nline LONG TEXT 2\n"
    );
    test.assert_cursor(1, 14);
}

#[test]
fn test_dot_repeat_complex_change() {
    let mut test = EditorTest::new("one two\nthree four\nfive six");

    test.keys("ciw") // Change "one" to "REPLACED"
        .type_text("REPLACED")
        .press_esc()
        .press('j') // Next line (cursor ends up on "four")
        .press('w') // Move to next word (wraps to "five" on line 2)
        .press('.'); // Repeat change: ciw "REPLACED" on "five"

    // ciw preserves surrounding whitespace
    assert_eq!(
        test.buffer_content(),
        "REPLACED two\nthree four\nREPLACED six\n"
    );
    test.assert_cursor(2, 7);
}

// ============================================================================
// Regression: dot-repeat o/O must create a new line, not corrupt current line
// ============================================================================

#[test]
fn test_dot_repeat_o_on_indented_line() {
    // Regression: dot-repeat of 'o' was inserting indent at end of current
    // line instead of creating a new line below.
    let mut test = EditorTest::new("    hello\n    world");

    // o on indented line, type text, Esc
    test.keys("o");
    test.type_text("new");
    test.press_esc();

    // Move to "world" line and repeat
    test.keys("j"); // now on "    world"
    test.keys(".");

    // Should have: "    hello", "    new", "    world", "    new"
    assert_eq!(test.line(0).unwrap(), "    hello\n");
    assert_eq!(test.line(1).unwrap(), "    new\n");
    assert_eq!(test.line(2).unwrap(), "    world\n");
    assert_eq!(test.line(3).unwrap(), "    new\n");
    assert_eq!(test.line_count(), 4);
}

#[test]
fn test_dot_repeat_O_on_indented_line() {
    // Regression: same issue for O (open above)
    let mut test = EditorTest::new("    hello\n    world");

    // O on first line, type text, Esc
    test.keys("O");
    test.type_text("above");
    test.press_esc();

    // Move to "world" line and repeat
    test.keys("jj"); // skip past hello to world
    test.keys(".");

    // Should insert "    above" above "    world"
    assert_eq!(test.line_count(), 4);
    assert!(
        test.buffer_content().contains("    above\n    hello"),
        "First O should be above hello"
    );
    assert!(
        test.buffer_content().contains("    above\n    world"),
        "Repeated O should be above world"
    );
}

#[test]
fn test_dot_repeat_o_esc_no_text() {
    // Regression: o<Esc> with non-default entry_mode must preserve entry_mode
    // for dot-repeat even when only 1 or 2 sub-changes exist.
    let mut test = EditorTest::new("line 1\nline 2");

    // o then immediately Esc (creates empty line)
    test.keys("o");
    test.press_esc();

    // Move down and repeat
    test.keys("j");
    test.keys(".");

    // Both lines should have gotten new empty lines below them
    assert_eq!(test.line_count(), 4);
    assert_eq!(test.line(0).unwrap(), "line 1\n");
    assert_eq!(test.line(1).unwrap(), "\n");
    assert_eq!(test.line(2).unwrap(), "line 2\n");
    assert_eq!(test.line(3).unwrap(), "\n");
}

#[test]
fn test_dot_repeat_o_uses_current_line_indent() {
    // Dot-repeat of 'o' should use the CURRENT line's indent, not the
    // indent from the original 'o' command.
    let mut test = EditorTest::new("    four_spaces\n\t\ttwo_tabs");

    // o on 4-space indented line, type text, Esc
    test.keys("o");
    test.type_text("x");
    test.press_esc();

    // Move to two-tabs line and repeat
    test.keys("j"); // on two_tabs line
    test.keys(".");

    // The repeated line should use two_tabs indent, not four_spaces
    let content = test.buffer_content();
    assert!(
        content.contains("\t\tx\n"),
        "Repeated o should use current line's tab indent, got: {:?}",
        content
    );
}
