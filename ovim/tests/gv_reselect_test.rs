//! `gv` reselects the area as it was when Visual mode ended, even when the operator that ended
//! it (`>`, `~`, `u`, `y`, `c`, `r`, ...) moved the cursor first. Every row was produced with
//! `nvim --clean --headless` (`normal! {keys}`): the buffer and the cursor as (line, column) after
//! the `gvd` that ends each row.

mod helpers;
use helpers::EditorTest;

/// (content, keys, buffer, cursor)
type Row = (&'static str, &'static str, &'static str, (usize, usize));

fn check(rows: &[Row]) {
    for &(content, keys, buffer, cursor) in rows {
        let mut test = EditorTest::new(&format!("{content}\n"));
        test.keys(keys);
        let what = format!("{keys} on {content:?}");
        assert_eq!(test.buffer_content(), buffer, "buffer after {what}");
        assert_eq!(test.cursor(), cursor, "cursor after {what}");
    }
}

#[test]
fn gv_after_an_operator_that_moves_the_cursor_reselects_the_selection() {
    check(&[
        ("l1\nl2\nl3\nl4\nl5", "Vj>gvd", "l3\nl4\nl5\n", (0, 0)),
        (
            "abc\ndef\nghi\njkl",
            "lvj>gvd",
            "   def\nghi\njkl\n",
            (0, 1),
        ),
        (
            "abc def\nabc def\nabc def",
            "vjlUgvd",
            "c def\nabc def\n",
            (0, 0),
        ),
        ("l1\nl2\nl3\nl4", "Vj~gvd", "l3\nl4\n", (0, 0)),
        ("L1\nL2\nL3\nL4", "Vjugvd", "L3\nL4\n", (0, 0)),
        ("l1\nl2\nl3\nl4", "Vjcx<Esc>gvd", "l4\n", (0, 0)),
        ("    l1\n    l2\nl3\nl4", "Vj<gvd", "l3\nl4\n", (0, 0)),
        ("1\n1\n1\n1", "Vj<C-a>gvd", "1\n1\n", (0, 0)),
        ("L1\nL2\nL3\nL4", "Vjjgugvd", "L4\n", (0, 0)),
        ("l1\nl2\nl3\nl4", "Vj=gvd", "l3\nl4\n", (0, 0)),
        ("l1\nl2\nl3\nl4", "vjrxgvd", "2\nl3\nl4\n", (0, 0)),
        (
            "l1\nl2\nl3\nl4\nl5",
            "Vj>gv<Esc>gvd",
            "l3\nl4\nl5\n",
            (0, 0),
        ),
    ]);
}

#[test]
fn gv_after_leaving_visual_reselects_the_selection() {
    check(&[
        ("l1\nl2\nl3\nl4\nl5", "Vjjygvd", "l4\nl5\n", (0, 0)),
        (
            "abcdef\nabcdef\nabcdef",
            "lvjly3Ggvd",
            "adef\nabcdef\n",
            (0, 1),
        ),
        (
            "abcdef\nabcdef\nabcdef",
            "3Gllvkhy1Ggvd",
            "abcdef\nadef\n",
            (1, 1),
        ),
        ("l1\nl2\nl3\nl4\nl5", "jVj<Esc>Ggvd", "l1\nl4\nl5\n", (1, 0)),
        (
            "abcdef\nabcdef\nabcdef",
            "lvjl<Esc>Ggvd",
            "adef\nabcdef\n",
            (0, 1),
        ),
        (
            "abcd\nabcd\nabcd",
            "l<C-v>jly3Ggvd",
            "ad\nad\nabcd\n",
            (0, 1),
        ),
        ("l1\nl2\nl3\nl4\nl5", "jVjoy4Ggvd", "l1\nl4\nl5\n", (1, 0)),
        (
            "l1\nl2\nl3\nl4\nl5",
            "jVj<Esc>ggjjgvd",
            "l1\nl4\nl5\n",
            (1, 0),
        ),
    ]);
}
