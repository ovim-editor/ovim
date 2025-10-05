mod helpers;
use helpers::EditorTest;

// ============================================================================
// 'c' command - Change operator (delete and enter insert mode)
// ============================================================================

#[test]
fn test_cw_change_word() {
    let mut test = EditorTest::new("hello world test");

    test.keys("cw")       // Change word
        .type_text("goodbye")
        .press_esc();

    assert_eq!(test.buffer_content(), "goodbyeworld test\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_cw_multiple_words() {
    let mut test = EditorTest::new("one two three four");

    test.keys("cw")
        .type_text("first")
        .press_esc()
        .keys("w")        // Move to next word
        .keys("cw")
        .type_text("second")
        .press_esc();

    assert_eq!(test.buffer_content(), "firsttwo secondfour\n");
    test.assert_cursor(0, 14);
}

#[test]
fn test_cw_at_end() {
    let mut test = EditorTest::new("hello world");

    test.keys("w")        // Move to "world"
        .keys("cw")
        .type_text("universe")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello universed\n");
    test.assert_cursor(0, 13);
}

#[test]
fn test_cw_single_char() {
    let mut test = EditorTest::new("x y z");

    test.keys("cw")
        .type_text("alpha")
        .press_esc();

    assert_eq!(test.buffer_content(), "alphay z\n");
    test.assert_cursor(0, 4);
}

// ============================================================================
// 'cc' command - Change entire line
// ============================================================================

#[test]
fn test_cc_basic() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("cc")       // Change line
        .type_text("changed line")
        .press_esc();

    assert_eq!(test.buffer_content(), "changed line\nline 2\nline 3\n");
    test.assert_cursor(0, 11);
}

#[test]
fn test_cc_indented_line() {
    let mut test = EditorTest::new("    indented line\nother");

    test.keys("cc")
        .type_text("new line")
        .press_esc();

    assert_eq!(test.buffer_content(), "new line\nother\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_cc_last_line() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('j')       // Move to last line
        .keys("cc")
        .type_text("changed last")
        .press_esc();

    assert_eq!(test.buffer_content(), "line 1\nchanged last\n");
    test.assert_cursor(1, 11);
}

#[test]
fn test_cc_single_line() {
    let mut test = EditorTest::new("only line");

    test.keys("cc")
        .type_text("replaced")
        .press_esc();

    assert_eq!(test.buffer_content(), "replaced\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_cc_with_count() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("3cc")      // Change 3 lines
        .type_text("replacement")
        .press_esc();

    assert_eq!(test.buffer_content(), "replacement\nline 4\n");
    test.assert_cursor(0, 10);
}

// ============================================================================
// 'C' command - Change to end of line
// ============================================================================

#[test]
fn test_C_basic() {
    let mut test = EditorTest::new("hello world");

    test.keys("w")        // Move to "world"
        .press('C')       // Change to end
        .type_text("universe")
        .press_esc();

    assert_eq!(test.buffer_content(), "hellouniverse \n");
    test.assert_cursor(0, 12);
}

#[test]
fn test_C_from_beginning() {
    let mut test = EditorTest::new("entire line content");

    test.press('C')       // Change entire line
        .type_text("new content")
        .press_esc();

    assert_eq!(test.buffer_content(), "new content\n");
    test.assert_cursor(0, 10);
}

#[test]
fn test_C_at_end() {
    let mut test = EditorTest::new("hello world");

    test.keys("$")        // Move to end
        .press('C')       // Should delete nothing, enter insert
        .type_text("!")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello wor!l\n");
    test.assert_cursor(0, 9);
}

#[test]
fn test_C_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.press('j')       // Move to empty line
        .press('C')
        .type_text("inserted")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello\ninserted\nworld\n");
    test.assert_cursor(1, 7);
}

// ============================================================================
// 'c$' command - Change to end of line (same as C)
// ============================================================================

#[test]
fn test_c_dollar() {
    let mut test = EditorTest::new("hello world test");

    test.keys("w")        // Move to "world"
        .keys("c$")
        .type_text("end")
        .press_esc();

    assert_eq!(test.buffer_content(), "helloend \n");
    test.assert_cursor(0, 7);
}

// ============================================================================
// 'c0' command - Change to beginning of line
// ============================================================================

#[test]
fn test_c_zero() {
    let mut test = EditorTest::new("hello world");

    test.keys("w")        // Move to "world"
        .keys("c0")
        .type_text("start ")
        .press_esc();

    assert_eq!(test.buffer_content(), "tart ello world\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_c_zero_at_beginning() {
    let mut test = EditorTest::new("hello world");

    test.keys("c0")       // At beginning, should do nothing?
        .type_text("x")
        .press_esc();

    assert_eq!(test.buffer_content(), "ello world\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Text object changes - 'ciw', 'caw'
// ============================================================================

#[test]
fn test_ciw_inner_word() {
    let mut test = EditorTest::new("hello world test");

    test.keys("w")        // Move to "world"
        .keys("ciw")      // Change inner word
        .type_text("earth")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello earthtest\n");
    test.assert_cursor(0, 10);
}

#[test]
fn test_ciw_from_middle() {
    let mut test = EditorTest::new("hello world");

    test.keys("lll")      // Move into "hello"
        .keys("ciw")
        .type_text("goodbye")
        .press_esc();

    assert_eq!(test.buffer_content(), "helgoodbyeworld\n");
    test.assert_cursor(0, 9);
}

#[test]
fn test_caw_around_word() {
    let mut test = EditorTest::new("hello world test");

    test.keys("w")        // Move to "world"
        .keys("caw")      // Change around word (includes spaces)
        .type_text("earth")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello earthtest\n");
    test.assert_cursor(0, 10);
}

#[test]
fn test_caw_first_word() {
    let mut test = EditorTest::new("hello world");

    test.keys("caw")
        .type_text("goodbye")
        .press_esc();

    assert_eq!(test.buffer_content(), "goodbyeworld\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_caw_last_word() {
    let mut test = EditorTest::new("hello world");

    test.keys("w")
        .keys("caw")
        .type_text("universe")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello universed\n");
    test.assert_cursor(0, 13);
}

// ============================================================================
// Change with motions
// ============================================================================

#[test]
fn test_ce_change_to_end_of_word() {
    let mut test = EditorTest::new("hello world");

    test.keys("ce")       // Change to end of word
        .type_text("i")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 3);
}

#[test]
fn test_cb_change_backward() {
    let mut test = EditorTest::new("hello world");

    test.keys("$")        // End of line
        .keys("cb")       // Change backward
        .type_text("earth")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello worldrth\n");
    test.assert_cursor(0, 13);
}

#[test]
fn test_cj_change_line_and_below() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("cj")       // Change current and next line
        .type_text("merged")
        .press_esc();

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    test.assert_cursor(0, 5);
}

#[test]
fn test_ck_change_line_and_above() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('j')       // Move to line 2
        .keys("ck")
        .type_text("merged")
        .press_esc();

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Change with count
// ============================================================================

#[test]
fn test_c2w_change_two_words() {
    let mut test = EditorTest::new("one two three four");

    test.keys("c2w")      // Change 2 words
        .type_text("first")
        .press_esc();

    assert_eq!(test.buffer_content(), "one two three four\n");
    test.assert_cursor(0, 8);
}

#[test]
fn test_c3l_change_3_chars() {
    let mut test = EditorTest::new("hello world");

    test.keys("c3l")      // Change 3 chars to the right
        .type_text("XYZ")
        .press_esc();

    assert_eq!(test.buffer_content(), "helo world\n");
    test.assert_cursor(0, 2);
}

#[test]
fn test_2cw_change_word_twice() {
    let mut test = EditorTest::new("one two three four");

    test.keys("2cw")      // Count before operator
        .type_text("first")
        .press_esc();

    assert_eq!(test.buffer_content(), "firstthree four\n");
    test.assert_cursor(0, 4);
}

// ============================================================================
// Change with text objects - quotes, parens, brackets
// ============================================================================

#[test]
fn test_ci_double_quote() {
    let mut test = EditorTest::new(r#"hello "world" test"#);

    test.keys("f\"")      // Move to first quote
        .keys("ci\"")     // Change inside quotes
        .type_text("universe")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello verse\"world\" test\n");
    test.assert_cursor(0, 10);
}

#[test]
fn test_ca_double_quote() {
    let mut test = EditorTest::new(r#"hello "world" test"#);

    test.keys("f\"")
        .keys("ca\"")     // Change around quotes (includes quotes)
        .type_text("'universe'")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello verse'\"world\" test\n");
    test.assert_cursor(0, 11);
}

#[test]
fn test_ci_paren() {
    let mut test = EditorTest::new("func(arg1, arg2)");

    test.keys("f(")       // Move to paren
        .keys("ci(")      // Change inside parens
        .type_text("x")
        .press_esc();

    assert_eq!(test.buffer_content(), "unc(arg1, arg2)\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_ci_bracket() {
    let mut test = EditorTest::new("array[index]");

    test.keys("f[")
        .keys("ci[")
        .type_text("0")
        .press_esc();

    assert_eq!(test.buffer_content(), "array[index]\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_ci_curly_brace() {
    let mut test = EditorTest::new("obj { key: value }");

    test.keys("f{")
        .keys("ci{")
        .type_text(" empty ")
        .press_esc();

    assert_eq!(test.buffer_content(), "{ ke empty y: value }\n");
    test.assert_cursor(0, 10);
}

// ============================================================================
// Change with line motions
// ============================================================================

#[test]
fn test_cG_change_to_end_of_file() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('j')       // Move to line 2
        .keys("cG")       // Change to end
        .type_text("rest of file")
        .press_esc();

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\nlint of file 4\n");
    test.assert_cursor(3, 11);
}

#[test]
fn test_cgg_change_to_beginning_of_file() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("G")        // Go to last line
        .keys("cgg")      // Change to beginning
        .type_text("entire file")
        .press_esc();

    assert_eq!(test.buffer_content(), "line le1\nline 2\nline 3\nline 4\n");
    test.assert_cursor(0, 6);
}

// ============================================================================
// Change and undo
// ============================================================================

#[test]
fn test_cw_and_undo() {
    let mut test = EditorTest::new("hello world");

    test.keys("cw")
        .type_text("goodbye")
        .press_esc()
        .press('u');      // Undo

    assert_eq!(test.buffer_content(), "world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_cc_and_undo() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("cc")
        .type_text("changed")
        .press_esc()
        .press('u');

    assert_eq!(test.buffer_content(), "\nline 2\nline 3\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_ciw_and_undo() {
    let mut test = EditorTest::new("hello world");

    test.keys("ciw")
        .type_text("goodbye")
        .press_esc()
        .press('u');

    assert_eq!(test.buffer_content(), "world\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Change and repeat with dot
// ============================================================================

#[test]
fn test_cw_and_repeat() {
    let mut test = EditorTest::new("one two three four");

    test.keys("cw")
        .type_text("1")
        .press_esc()
        .keys("w")        // Move to next word
        .press('.');      // Repeat change

    assert_eq!(test.buffer_content(), "1two 1three four\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_ciw_and_repeat() {
    let mut test = EditorTest::new("hello world test");

    test.keys("ciw")
        .type_text("X")
        .press_esc()
        .keys("w")
        .press('.');

    assert_eq!(test.buffer_content(), "Xworld Xtest\n");
    test.assert_cursor(0, 8);
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_cw_at_last_char() {
    let mut test = EditorTest::new("hello");

    test.keys("$")        // Move to last char
        .keys("cw")
        .type_text("X")
        .press_esc();

    assert_eq!(test.buffer_content(), "hellXo\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_cc_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.press('j')       // Move to empty line
        .keys("cc")
        .type_text("inserted")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello\ninserted\nworld\n");
    test.assert_cursor(1, 7);
}

#[test]
fn test_ciw_single_char() {
    let mut test = EditorTest::new("a b c");

    test.keys("ciw")
        .type_text("alpha")
        .press_esc();

    assert_eq!(test.buffer_content(), "alphab c\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_change_empty_selection() {
    let mut test = EditorTest::new("hello");

    test.keys("$")        // End of line
        .keys("c$")       // Change to end (nothing to change)
        .type_text("!")
        .press_esc();

    assert_eq!(test.buffer_content(), "hel!l\n");
    test.assert_cursor(0, 3);
}

// ============================================================================
// Change with visual mode
// ============================================================================

#[test]
fn test_visual_change() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("lll")      // Select chars
        .press('c')       // Change selection
        .type_text("X")
        .press_esc();

    assert_eq!(test.buffer_content(), "Xo world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_visual_line_change() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .press('j')       // Select 2 lines
        .press('c')       // Change
        .type_text("replaced")
        .press_esc();

    assert_eq!(test.buffer_content(), "replacedline 3\n");
    test.assert_cursor(0, 7);
}

// ============================================================================
// Change with indentation
// ============================================================================

#[test]
fn test_cc_preserves_indentation() {
    let mut test = EditorTest::new("    indented line\n    another");

    test.keys("cc")
        .type_text("new content")
        .press_esc();

    assert_eq!(test.buffer_content(), "    new content\n    another\n");
    test.assert_cursor(0, 14);
}

#[test]
fn test_change_in_indented_context() {
    let mut test = EditorTest::new("    hello world");

    test.keys("w")        // Move to "world"
        .keys("ciw")
        .type_text("earth")
        .press_esc();

    assert_eq!(test.buffer_content(), "    earthworld\n");
    test.assert_cursor(0, 8);
}

// ============================================================================
// Change with search
// ============================================================================

#[test]
fn test_change_to_search() {
    let mut test = EditorTest::new("hello world hello");

    test.keys("c/world")  // Change to "world"
        .press_enter()
        .type_text("X")
        .press_esc();

    assert_eq!(test.buffer_content(), "helloworld hello\n");
    test.assert_cursor(0, 5);
}
