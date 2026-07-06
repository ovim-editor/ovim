//! Regression tests for bug-hunt findings:
//!  - gu/gU/g~ with the inclusive `e` motion must transform the last char
//!  - Ctrl-A/Ctrl-X on the leading '0' of a based literal must not truncate it
//!  - Ctrl-D/Ctrl-U preserve the goal column across shorter lines

#![allow(non_snake_case)]

mod helpers;
use helpers::EditorTest;
use ovim_core::{KeyCode, Modifiers};

// ---- case + inclusive `e` motion ----------------------------------------

#[test]
fn test_gUe_uppercases_whole_word() {
    let mut test = EditorTest::new("hello world");
    test.keys("gUe");
    assert_eq!(test.buffer_content(), "HELLO world\n", "gUe must uppercase the final char too");
}

#[test]
fn test_gue_lowercases_whole_word() {
    let mut test = EditorTest::new("HELLO world");
    test.keys("gue");
    assert_eq!(test.buffer_content(), "hello world\n");
}

#[test]
fn test_gtilde_e_toggles_whole_word() {
    let mut test = EditorTest::new("abc def");
    test.keys("g~e");
    assert_eq!(test.buffer_content(), "ABC def\n");
}

#[test]
fn test_gUw_still_exclusive() {
    // gUw is exclusive: uppercases up to (not including) the next word start.
    let mut test = EditorTest::new("hello world");
    test.keys("gUw");
    assert_eq!(test.buffer_content(), "HELLO world\n");
}

// ---- Ctrl-A / Ctrl-X on based literals ----------------------------------

fn ctrl(test: &mut EditorTest, c: char) {
    test.press_with(KeyCode::Char(c), Modifiers::CONTROL);
}

#[test]
fn test_ctrl_a_on_leading_zero_of_hex() {
    let mut test = EditorTest::new("0xff");
    // cursor on leading '0'
    ctrl(&mut test, 'a');
    assert_eq!(test.buffer_content(), "0x100\n", "Ctrl-A on 0xff should give 0x100, not mangle to 1");
}

#[test]
fn test_ctrl_a_on_hex_digit() {
    let mut test = EditorTest::new("0xff");
    test.keys("$"); // cursor on last 'f'
    ctrl(&mut test, 'a');
    assert_eq!(test.buffer_content(), "0x100\n");
}

#[test]
fn test_ctrl_a_on_leading_zero_of_binary() {
    let mut test = EditorTest::new("0b101");
    ctrl(&mut test, 'a');
    assert_eq!(test.buffer_content(), "0b110\n");
}

#[test]
fn test_ctrl_x_on_leading_zero_of_hex() {
    let mut test = EditorTest::new("0x10");
    ctrl(&mut test, 'x');
    assert_eq!(test.buffer_content(), "0xf\n");
}

#[test]
fn test_ctrl_a_plain_decimal_unaffected() {
    let mut test = EditorTest::new("42");
    ctrl(&mut test, 'a');
    assert_eq!(test.buffer_content(), "43\n");
}

// ---- Ctrl-D / Ctrl-U goal column ----------------------------------------

#[test]
fn test_ctrl_d_preserves_goal_column() {
    // Long line, short line, long line. Cursor at col 8 on line 0.
    // Ctrl-D onto the short middle region then back should restore column 8.
    let mut lines = String::new();
    for i in 0..30 {
        if i == 10 {
            lines.push_str("ab\n"); // short line
        } else {
            lines.push_str("abcdefghij\n"); // 10-char lines
        }
    }
    let mut test = EditorTest::new(&lines);
    test.keys("8l"); // cursor col 8 on line 0, goal col 8
    assert_eq!(test.cursor(), (0, 8));
    // Half-page scroll a few times; ensure that once we land on a long line the
    // column is restored to the goal even after crossing the short line.
    ctrl(&mut test, 'd');
    // After landing, if on a long line the col should be 8 (goal preserved)
    let (line, col) = test.cursor();
    if test.line_text(line).map(|l| l.len()).unwrap_or(0) >= 9 {
        assert_eq!(col, 8, "goal column should be preserved on long lines after Ctrl-D");
    }
}
