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

// Sticky `$` (curswant = MAXCOL) entering a block must behave as a to-EOL block
// instead of leaving the cursor column at usize::MAX.
// nvim --clean: "abc\ndef\nghi", `$<C-v>jy` yanks "c\nf" blockwise (width 1); `d`/`x`
// delete the last column of both lines; `U` uppercases it; `rX` replaces it; `cX<Esc>`
// changes it on both lines (-> "abX\ndeX").

#[test]
fn test_dollar_then_block_yank_does_not_panic() {
    let mut test = EditorTest::new("abc\ndef\nghi\n");
    test.keys("$");
    cblock(&mut test);
    test.keys("jy");
    assert_eq!(test.get_register_content('"').as_deref(), Some("c\nf"));
    assert_eq!(test.buffer_content(), "abc\ndef\nghi\n");
}

#[test]
fn test_dollar_then_block_delete_and_x() {
    for key in ["d", "x"] {
        let mut test = EditorTest::new("abc\ndef\nghi\n");
        test.keys("$");
        cblock(&mut test);
        test.keys(&format!("j{key}"));
        assert_eq!(test.buffer_content(), "ab\nde\nghi\n", "key {key}");
    }
}

#[test]
fn test_dollar_then_block_upper_and_replace() {
    let mut test = EditorTest::new("abc\ndef\nghi\n");
    test.keys("$");
    cblock(&mut test);
    test.keys("jU");
    assert_eq!(test.buffer_content(), "abC\ndeF\nghi\n");

    let mut test = EditorTest::new("abc\ndef\nghi\n");
    test.keys("$");
    cblock(&mut test);
    test.keys("jrX");
    assert_eq!(test.buffer_content(), "abX\ndeX\nghi\n");
}

#[test]
fn test_dollar_then_block_change_hits_both_lines() {
    let mut test = EditorTest::new("abc\ndef\nghi\n");
    test.keys("$");
    cblock(&mut test);
    test.keys("jcX<Esc>");
    assert_eq!(test.buffer_content(), "abX\ndeX\nghi\n");
}

#[test]
fn test_dollar_then_block_over_short_line_deletes_only_existing_columns() {
    // nvim --clean: "abc\nde\nghij", `$<C-v>jjd` -> "ab\nde\ngh" (to-EOL block starting at
    // the last column of line 1; the short middle line has no such column).
    let mut test = EditorTest::new("abc\nde\nghij\n");
    test.keys("$");
    cblock(&mut test);
    test.keys("jjd");
    assert_eq!(test.buffer_content(), "ab\nde\ngh\n");
}

#[test]
fn test_block_dollar_typed_inside_block_matches_sticky_dollar() {
    // `<C-v>$jd` (explicit `$` inside the block) from column 1.
    // nvim --clean: "abc\ndef\nghi", `l<C-v>$jd` -> "a\nd\nghi".
    let mut test = EditorTest::new("abc\ndef\nghi\n");
    test.keys("l");
    cblock(&mut test);
    test.keys("$jd");
    assert_eq!(test.buffer_content(), "a\nd\nghi\n");
}

#[test]
fn test_dollar_block_left_edge_is_the_anchor_column() {
    // nvim --clean: "hello\nworld\ntest", `$<C-v>jjd` -> "hell\nworl\ntest": the short
    // last line (no column 4) must not pull the left edge to its own last column.
    let mut test = EditorTest::new("hello\nworld\ntest\n");
    test.keys("$");
    cblock(&mut test);
    test.keys("jjd");
    assert_eq!(test.buffer_content(), "hell\nworl\ntest\n");

    // nvim --clean: `lll<C-v>jj$d` -> "hel\nwor\ntes".
    let mut test = EditorTest::new("hello\nworld\ntest\n");
    test.keys("lll");
    cblock(&mut test);
    test.keys("jj$d");
    assert_eq!(test.buffer_content(), "hel\nwor\ntes\n");
}
