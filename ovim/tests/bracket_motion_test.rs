//! Regression tests for the enclosing-bracket motions `[{`, `]}`, `[(`, `])`.
//!
//! These used to build the absolute char offset of the cursor by *summing
//! terminator-stripped line lengths* (`line_text(i).chars().count()`) while
//! indexing into the terminator-*included* `rope.to_string()` — so from any
//! line `n > 0` the search started `n` characters too early. The bug only
//! shows when one of those skipped (`= line_idx`) characters immediately before
//! the cursor is itself a brace/paren, which is exactly what these fixtures
//! engineer. (OV-00264)

mod helpers;
use helpers::EditorTest;

#[test]
fn enclosing_open_brace_from_second_line() {
    // `}{x` on line 1: the char right before the cursor (`x`) is the `{` we
    // want. The old off-by-1 made the search start *on* that `{`, skip it, hit
    // the leading `}`/`{` pair, and fall through to (0, 0).
    let mut test = EditorTest::new("{\n}{x");
    test.keys("j$"); // line 1, on 'x' (col 2)
    test.keys("[{");
    test.assert_cursor(1, 1); // the enclosing '{' on the same line — not (0, 0)
}

#[test]
fn enclosing_open_brace_from_third_line() {
    // Two lines down → off-by-2. Skipped chars are `}{` immediately before the
    // cursor; the real target is the `{` at (2, 1).
    let mut test = EditorTest::new("{\n}\n}{x");
    test.keys("jj$"); // line 2, on 'x' (col 2)
    test.keys("[{");
    test.assert_cursor(2, 1);
}

#[test]
fn enclosing_open_paren_from_second_line() {
    // Same shape, generic `jump_to_enclosing_char` path (`[(`).
    let mut test = EditorTest::new("(\n)(x");
    test.keys("j$"); // line 1, on 'x' (col 2)
    test.keys("[(");
    test.assert_cursor(1, 1);
}

#[test]
fn enclosing_close_brace_from_third_line() {
    // Forward variant (`]}`). Cursor on `x` in `}}x}` (line 2): the real
    // unmatched `}` is the trailing one at col 3. The old off-by-2 started the
    // forward scan two chars early, landing on the `}` at col 1 instead.
    let mut test = EditorTest::new("{\n}\n}}x}");
    test.keys("jjfx"); // line 2, on 'x' (col 2)
    test.keys("]}");
    test.assert_cursor(2, 3);
}
