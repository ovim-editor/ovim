mod helpers;
use helpers::EditorTest;
use crossterm::event::{KeyCode, KeyModifiers};

// ============================================================================
// Ctrl-A - Increment number under cursor
// ============================================================================

#[test]
fn test_ctrl_a_increment_decimal() {
    let mut test = EditorTest::new("count: 42");

    test.keys("w")        // Move to number
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "count: 43\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_ctrl_a_increment_from_any_digit() {
    let mut test = EditorTest::new("number 123 end");

    test.keys("www")      // Move somewhere in the number
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "number 124 end\n");
    test.assert_cursor(0, 9);
}

#[test]
fn test_ctrl_a_negative_number() {
    let mut test = EditorTest::new("temp: -5");

    test.keys("w")
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "temp: -4\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_ctrl_a_zero() {
    let mut test = EditorTest::new("value: 0");

    test.keys("w")
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "value: 1\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_ctrl_a_large_number() {
    let mut test = EditorTest::new("big: 999999");

    test.keys("w")
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "big: 1000000\n");
    test.assert_cursor(0, 11);
}

#[test]
fn test_ctrl_a_with_count() {
    let mut test = EditorTest::new("val: 10");

    test.keys("w")
        .keys("5")        // Count of 5
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "val: 15\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_ctrl_a_hex_number() {
    let mut test = EditorTest::new("color: 0xff");

    test.keys("w")
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    // Should increment hex: 0xff -> 0x100
    assert_eq!(test.buffer_content(), "color: 0x100\n");
    test.assert_cursor(0, 11);
}

#[test]
fn test_ctrl_a_octal_number() {
    let mut test = EditorTest::new("perms: 0644");

    test.keys("w")
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "perms: 0645\n");
    test.assert_cursor(0, 10);
}

#[test]
fn test_ctrl_a_binary_number() {
    let mut test = EditorTest::new("bits: 0b1010");

    test.keys("w")
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "bits: 0b1011\n");
    test.assert_cursor(0, 11);
}

#[test]
fn test_ctrl_a_no_number_on_line() {
    let mut test = EditorTest::new("no numbers here");

    test.keys("w")
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    // Should not change anything
    assert_eq!(test.buffer_content(), "no numbers here\n");
    test.assert_cursor(0, 3);
}

#[test]
fn test_ctrl_a_search_forward() {
    let mut test = EditorTest::new("text 123 more");

    // Cursor at beginning - should find first number
    test.press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "text 124 more\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_ctrl_a_negative_to_positive() {
    let mut test = EditorTest::new("value: -1");

    test.keys("w")
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "value: 0\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_ctrl_a_multiple_numbers_on_line() {
    let mut test = EditorTest::new("x: 10, y: 20");

    test.keys("w")        // Move to first number
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "x: 11, y: 20\n");
    test.assert_cursor(0, 4);
}

// ============================================================================
// Ctrl-X - Decrement number under cursor
// ============================================================================

#[test]
fn test_ctrl_x_decrement_decimal() {
    let mut test = EditorTest::new("count: 42");

    test.keys("w")
        .press_with(KeyCode::Char('x'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "count: 41\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_ctrl_x_to_zero() {
    let mut test = EditorTest::new("value: 1");

    test.keys("w")
        .press_with(KeyCode::Char('x'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "value: 0\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_ctrl_x_to_negative() {
    let mut test = EditorTest::new("value: 0");

    test.keys("w")
        .press_with(KeyCode::Char('x'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "value: -1\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_ctrl_x_negative_number() {
    let mut test = EditorTest::new("temp: -5");

    test.keys("w")
        .press_with(KeyCode::Char('x'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "temp: -6\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_ctrl_x_with_count() {
    let mut test = EditorTest::new("val: 20");

    test.keys("w")
        .keys("7")        // Count of 7
        .press_with(KeyCode::Char('x'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "val: 13\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_ctrl_x_hex_number() {
    let mut test = EditorTest::new("color: 0x10");

    test.keys("w")
        .press_with(KeyCode::Char('x'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "color: 0xf\n");
    test.assert_cursor(0, 9);
}

#[test]
fn test_ctrl_x_octal_number() {
    let mut test = EditorTest::new("perms: 0755");

    test.keys("w")
        .press_with(KeyCode::Char('x'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "perms: 0754\n");
    test.assert_cursor(0, 10);
}

#[test]
fn test_ctrl_x_binary_number() {
    let mut test = EditorTest::new("bits: 0b1010");

    test.keys("w")
        .press_with(KeyCode::Char('x'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "bits: 0b1001\n");
    test.assert_cursor(0, 11);
}

// ============================================================================
// Ctrl-A/X with undo/redo
// ============================================================================

#[test]
fn test_ctrl_a_undo() {
    let mut test = EditorTest::new("value: 10");

    test.keys("w")
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL)
        .press('u');      // Undo

    assert_eq!(test.buffer_content(), "value: 10\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_ctrl_x_undo() {
    let mut test = EditorTest::new("value: 10");

    test.keys("w")
        .press_with(KeyCode::Char('x'), KeyModifiers::CONTROL)
        .press('u');

    assert_eq!(test.buffer_content(), "value: 10\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_ctrl_a_redo() {
    let mut test = EditorTest::new("value: 10");

    test.keys("w")
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL)
        .press('u')
        .press_with(KeyCode::Char('r'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "value: 11\n");
    test.assert_cursor(0, 7);
}

// ============================================================================
// Ctrl-A/X with dot repeat
// ============================================================================

#[test]
fn test_ctrl_a_dot_repeat() {
    let mut test = EditorTest::new("a: 1\nb: 2\nc: 3");

    test.keys("w")
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL) // Increment to 2
        .press('j')
        .press('.');      // Repeat on next line

    assert_eq!(test.buffer_content(), "a: 2\nb: 3\nc: 3\n");
    test.assert_cursor(1, 3);
}

#[test]
fn test_ctrl_x_dot_repeat() {
    let mut test = EditorTest::new("a: 5\nb: 4\nc: 3");

    test.keys("w")
        .press_with(KeyCode::Char('x'), KeyModifiers::CONTROL) // Decrement to 4
        .press('j')
        .press('.');      // Repeat

    assert_eq!(test.buffer_content(), "a: 4\nb: 3\nc: 3\n");
    test.assert_cursor(1, 3);
}

#[test]
fn test_ctrl_a_with_count_dot_repeat() {
    let mut test = EditorTest::new("a: 10\nb: 10\nc: 10");

    test.keys("w")
        .keys("5")
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL) // +5 to 15
        .press('j')
        .press('.');      // Repeat (should add 5 again)

    assert_eq!(test.buffer_content(), "a: 15\nb: 15\nc: 10\n");
    test.assert_cursor(1, 4);
}

// ============================================================================
// g Ctrl-A - Sequential increment in visual mode
// ============================================================================

#[test]
fn test_g_ctrl_a_sequential_increment() {
    let mut test = EditorTest::new("item 1\nitem 1\nitem 1");

    test.keys("w")        // Move to first number
        .press('V')       // Visual line mode
        .keys("jj")       // Select 3 lines
        .press('g')
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    // Should increment sequentially: 1, 2, 3
    assert_eq!(test.buffer_content(), "item 1\nitem 2\nitem 3\n");
    test.assert_cursor(0, 5);
}

#[test]
fn test_g_ctrl_a_with_start_value() {
    let mut test = EditorTest::new("step 0\nstep 0\nstep 0");

    test.keys("w")
        .press('V')
        .keys("jj")
        .press('g')
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "step 0\nstep 1\nstep 2\n");
    test.assert_cursor(0, 5);
}

#[test]
fn test_g_ctrl_a_visual_block() {
    let mut test = EditorTest::new("1. item\n1. item\n1. item");

    test.press_with(KeyCode::Char('v'), KeyModifiers::CONTROL)
        .keys("jj")       // Select column
        .press('g')
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    // Should increment the column of numbers sequentially
    assert_eq!(test.buffer_content(), "1. item\n2. item\n3. item\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// g Ctrl-X - Sequential decrement in visual mode
// ============================================================================

#[test]
fn test_g_ctrl_x_sequential_decrement() {
    let mut test = EditorTest::new("item 5\nitem 5\nitem 5");

    test.keys("w")
        .press('V')
        .keys("jj")
        .press('g')
        .press_with(KeyCode::Char('x'), KeyModifiers::CONTROL);

    // Should decrement sequentially: 5, 4, 3
    assert_eq!(test.buffer_content(), "item 5\nitem 4\nitem 3\n");
    test.assert_cursor(0, 5);
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_ctrl_a_at_line_end() {
    let mut test = EditorTest::new("value: 99");

    test.keys("$")        // End of line
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "value: 100\n");
    test.assert_cursor(0, 9);
}

#[test]
fn test_ctrl_a_before_number() {
    let mut test = EditorTest::new("prefix123suffix");

    test.keys("w")        // On 'p' of prefix
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    // Should search forward and find 123
    assert_eq!(test.buffer_content(), "prefix124suffix\n");
    test.assert_cursor(0, 8);
}

#[test]
fn test_ctrl_a_signed_number() {
    let mut test = EditorTest::new("delta: +5");

    test.keys("w")
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "delta: +6\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_ctrl_x_underflow() {
    let mut test = EditorTest::new("val: 0");

    test.keys("w")
        .keys("99")       // Large count
        .press_with(KeyCode::Char('x'), KeyModifiers::CONTROL);

    assert_eq!(test.buffer_content(), "val: -99\n");
    test.assert_cursor(0, 5);
}

#[test]
fn test_ctrl_a_float_number() {
    let mut test = EditorTest::new("pi: 3.14");

    test.keys("w")
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    // Behavior: only increment integer part or whole number?
    // Document actual behavior
    assert_eq!(test.buffer_content(), "pi: 4.14\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_ctrl_a_scientific_notation() {
    let mut test = EditorTest::new("val: 1e10");

    test.keys("w")
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    // How should scientific notation be handled?
    assert_eq!(test.buffer_content(), "val: 2e10\n");
    test.assert_cursor(0, 5);
}

#[test]
fn test_ctrl_a_empty_line() {
    let mut test = EditorTest::new("\n");

    test.press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    // Should not crash or change anything
    assert_eq!(test.buffer_content(), "\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_ctrl_a_letters_only() {
    let mut test = EditorTest::new("abc");

    test.press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    // Some versions of vim increment letters (a->b)
    // Document actual behavior
    assert_eq!(test.buffer_content(), "abc\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_ctrl_a_number_with_leading_zeros() {
    let mut test = EditorTest::new("id: 007");

    test.keys("w")
        .press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);

    // Might be treated as octal (007 -> 010) or decimal (007 -> 008)
    assert_eq!(test.buffer_content(), "id: 010\n");
    test.assert_cursor(0, 6);
}
