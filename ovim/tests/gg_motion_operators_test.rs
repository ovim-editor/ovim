mod helpers;
use helpers::EditorTest;
use ovim::mode::Mode;

// ============================================================================
// dgg - Delete from current line to first line
// ============================================================================

#[test]
fn test_dgg_from_middle_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.set_cursor(2, 0) // Move to line 3
        .keys("dgg");

    // Should delete lines 1-3, leaving only line 4
    assert_eq!(test.buffer_content(), "line 4\n");
    // Cursor should be at first non-blank of remaining line
    test.assert_cursor(0, 0);
}

#[test]
fn test_dgg_from_last_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.set_cursor(3, 0) // Move to line 4
        .keys("dgg");

    // Should delete all 4 lines
    assert_eq!(test.buffer_content(), "\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dgg_from_first_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("dgg"); // On first line, should delete just first line

    assert_eq!(test.buffer_content(), "line 2\nline 3\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dgg_with_count() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4\nline 5");

    test.set_cursor(4, 0) // Move to line 5
        .keys("d2gg"); // Delete from line 5 to line 2

    // Should delete lines 2-5, leaving only line 1
    assert_eq!(test.buffer_content(), "line 1\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dgg_with_indented_content() {
    let mut test = EditorTest::new("fn main() {\n    let x = 1;\n    let y = 2;\n}");

    test.set_cursor(2, 0) // Move to line 3 (let y = 2;)
        .keys("dgg");

    // Should delete first 3 lines, leaving only closing brace
    assert_eq!(test.buffer_content(), "}\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dgg_cursor_position_with_indentation() {
    let mut test = EditorTest::new("line 1\n    indented line 2\nline 3\nline 4");

    test.set_cursor(2, 0) // Move to line 3
        .keys("dgg");

    // After deletion, cursor should be at first non-blank
    assert_eq!(test.buffer_content(), "line 4\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dgg_single_line_file() {
    let mut test = EditorTest::new("only line");

    test.keys("dgg");

    // Should delete the only line, leaving empty buffer
    assert_eq!(test.buffer_content(), "\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// ygg - Yank from current line to first line
// ============================================================================

#[test]
fn test_ygg_from_middle_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.set_cursor(2, 0) // Move to line 3
        .keys("ygg");
    test.set_cursor(3, 0) // Move to line 4
        .press('p'); // Paste

    // Should yank lines 1-3 and paste them after line 4
    assert_eq!(
        test.buffer_content(),
        "line 1\nline 2\nline 3\nline 4\nline 1\nline 2\nline 3\n"
    );
}

#[test]
fn test_ygg_from_last_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.set_cursor(2, 0) // Move to line 3
        .keys("ygg")
        .press('p'); // Paste

    // Should yank all 3 lines and paste them after line 3
    assert_eq!(
        test.buffer_content(),
        "line 1\nline 2\nline 3\nline 1\nline 2\nline 3\n"
    );
}

#[test]
fn test_ygg_from_first_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("ygg") // On first line
        .press('p'); // Paste

    // Should yank just line 1 and paste after it
    assert_eq!(test.buffer_content(), "line 1\nline 1\nline 2\nline 3\n");
}

#[test]
fn test_ygg_with_count() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4\nline 5");

    test.set_cursor(4, 0) // Move to line 5
        .keys("y2gg") // Yank from line 5 to line 2
        .press('p'); // Paste

    // Should yank lines 2-5 and paste after line 5
    assert_eq!(
        test.buffer_content(),
        "line 1\nline 2\nline 3\nline 4\nline 5\nline 2\nline 3\nline 4\nline 5\n"
    );
}

#[test]
fn test_ygg_cursor_stays_at_position() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.set_cursor(2, 5) // Move to line 3, col 5
        .keys("ygg");

    // Cursor should stay at original position after yank
    test.assert_cursor(2, 5);
    // Buffer should be unchanged
    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
}

#[test]
fn test_ygg_single_line_file() {
    let mut test = EditorTest::new("only line");

    test.keys("ygg").press('p'); // Paste

    // Should yank and paste the single line
    assert_eq!(test.buffer_content(), "only line\nonly line\n");
}

// ============================================================================
// cgg - Change from current line to first line
// ============================================================================

#[test]
fn test_cgg_from_middle_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.set_cursor(2, 0) // Move to line 3
        .keys("cgg")
        .type_text("new text");

    // Should delete lines 1-3, enter insert mode, and allow typing
    test.press_esc();
    assert_eq!(test.buffer_content(), "new text\nline 4\n");
}

#[test]
fn test_cgg_from_last_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.set_cursor(2, 0) // Move to line 3
        .keys("cgg")
        .type_text("replacement");

    test.press_esc();
    // Should delete all lines and allow replacement
    assert_eq!(test.buffer_content(), "replacement\n");
}

#[test]
fn test_cgg_enters_insert_mode() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.set_cursor(1, 0) // Move to line 2
        .keys("cgg");

    // Should be in insert mode
    test.assert_mode(Mode::Insert);
}

#[test]
fn test_cgg_with_count() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4\nline 5");

    test.set_cursor(4, 0) // Move to line 5
        .keys("c3gg") // Change from line 5 to line 3
        .type_text("new");

    test.press_esc();
    assert_eq!(test.buffer_content(), "line 1\nline 2\nnew\n");
}

// ============================================================================
// Edge cases and integration tests
// ============================================================================

#[test]
fn test_dgg_undo() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.set_cursor(2, 0) // Move to line 3
        .keys("dgg")
        .press('u'); // Undo

    // Should restore all deleted lines
    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\nline 4\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_ygg_with_register() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.set_cursor(1, 0) // Move to line 2
        .keys("\"aygg"); // Yank to register 'a'
    test.set_cursor(2, 0) // Move to line 3
        .keys("\"ap"); // Paste from register 'a'

    // Should paste lines 1-2 after line 3
    assert_eq!(
        test.buffer_content(),
        "line 1\nline 2\nline 3\nline 1\nline 2\n"
    );
}

#[test]
fn test_dgg_with_empty_lines() {
    let mut test = EditorTest::new("line 1\n\n\nline 4");

    test.set_cursor(3, 0) // Move to line 4
        .keys("dgg");

    // Should delete all lines including empty ones
    assert_eq!(test.buffer_content(), "\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_multiple_dgg_operations() {
    let mut test = EditorTest::new("a\nb\nc\nd\ne\nf");

    test.set_cursor(2, 0) // Move to line 3 (c)
        .keys("dgg"); // Delete a, b, c

    assert_eq!(test.buffer_content(), "d\ne\nf\n");

    test.set_cursor(1, 0) // Move to line 2 (e)
        .keys("dgg"); // Delete d, e

    assert_eq!(test.buffer_content(), "f\n");
}

#[test]
fn test_dgg_with_cursor_in_middle_of_line() {
    let mut test = EditorTest::new("hello world\nfoo bar\nbaz qux");

    test.set_cursor(2, 4) // Move to line 3, col 4 (middle of "baz qux")
        .keys("dgg");

    // Should delete all 3 lines regardless of cursor column
    assert_eq!(test.buffer_content(), "\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_ygg_then_dgg() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.set_cursor(2, 0) // Move to line 3
        .keys("ygg") // Yank lines 1-3
        .keys("dgg") // Delete lines 1-3
        .press('p'); // Paste

    // Should have line 4, then pasted lines 1-3
    assert_eq!(test.buffer_content(), "line 4\nline 1\nline 2\nline 3\n");
}

// ============================================================================
// Comparison with dG and yG (for symmetry testing)
// ============================================================================

#[test]
fn test_dG_still_works() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.set_cursor(1, 0) // Move to line 2
        .keys("dG"); // Delete from line 2 to end

    assert_eq!(test.buffer_content(), "line 1\n");
}

#[test]
fn test_yG_still_works() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.set_cursor(0, 0) // Move to line 1
        .keys("yG") // Yank from line 1 to end
        .press('p'); // Paste

    assert_eq!(
        test.buffer_content(),
        "line 1\nline 1\nline 2\nline 3\nline 2\nline 3\n"
    );
}

#[test]
fn test_d2G_with_count() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.set_cursor(0, 0) // Move to line 1
        .keys("d2G"); // Delete from line 1 to line 2

    assert_eq!(test.buffer_content(), "line 3\nline 4\n");
}
