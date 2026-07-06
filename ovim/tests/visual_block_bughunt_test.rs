//! Regression tests for VisualBlock / visual-paste bugs found in the bug hunt.

#![allow(non_snake_case)]

mod helpers;
use helpers::EditorTest;
use ovim_core::{KeyCode, Modifiers};

fn cblock(test: &mut EditorTest) {
    test.press_with(KeyCode::Char('v'), Modifiers::CONTROL);
}

#[test]
fn test_visualblock_column_not_collapsed_over_short_line() {
    // Column 3 exists on the long lines but not the short middle line. Deleting a
    // 1-wide block whose path crosses the short line must delete only column 3 on
    // the long lines, not collapse to column 0.
    let mut test = EditorTest::new("abcXd\np\nabcYd\n");
    test.keys("lll"); // cursor at col 3 (X) on line 0
    cblock(&mut test);
    test.keys("jj"); // extend block down across the short "p" line
    test.press('x'); // delete the block
    assert_eq!(
        test.buffer_content(),
        "abcd\np\nabcd\n",
        "block delete must not collapse the column over the short line"
    );
}

#[test]
fn test_visualblock_append_fixed_column() {
    // <C-v>jjA! on a col-0 block appends at the block column (col 1) on each line,
    // NOT at end-of-line.
    let mut test = EditorTest::new("hello\nworld\ntest\n");
    cblock(&mut test);
    test.keys("jj");
    test.press('A').type_text("!").press_esc();
    assert_eq!(test.buffer_content(), "h!ello\nw!orld\nt!est\n");
}

#[test]
fn test_visualblock_dollar_append_still_eol() {
    // $<C-v>jjA! is a to-EOL block: appends at each line's own end.
    let mut test = EditorTest::new("hello\nworld\ntest\n");
    test.keys("$");
    cblock(&mut test);
    test.keys("jj");
    test.press('A').type_text("!").press_esc();
    assert_eq!(test.buffer_content(), "hello!\nworld!\ntest!\n");
}

#[test]
fn test_visual_paste_over_selection_at_col0() {
    // Yank "PQ", select "he" at col 0 on line 1, paste over it -> "PQllo".
    let mut test = EditorTest::new("PQ\nhello");
    test.keys("vly"); // visual select P,Q then yank -> register "PQ"
    test.keys("j0"); // line 1 col 0
    test.keys("vlp"); // select h,e then paste "PQ" over them
    assert_eq!(
        test.buffer_content(),
        "PQ\nPQllo\n",
        "paste over a col-0 selection must not be shifted by one char"
    );
}
