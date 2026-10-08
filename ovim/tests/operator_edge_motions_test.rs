//! `j` / `k` (and `+`, `-`, `<CR>`, `_`) fail on the last / first line, which cancels the pending
//! operator without touching text or registers, and so does a counted doubled operator (`2dd`)
//! on the last line; a count that reaches past the edge is clamped. `.` fails the same way.
//! Every row was produced with `nvim --clean --headless` (`normal! {keys}`): the buffer and the
//! cursor as (line, column).

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
fn j_and_k_fail_at_the_edges_and_cancel_the_operator() {
    check(&[
        ("l1\nl2\nl3", "Gdj", "l1\nl2\nl3\n", (2, 0)),
        ("l1\nl2\nl3", "dk", "l1\nl2\nl3\n", (0, 0)),
        ("l1\nl2\nl3", "Gd5j", "l1\nl2\nl3\n", (2, 0)),
        ("l1\nl2\nl3", "d5k", "l1\nl2\nl3\n", (0, 0)),
        ("l1\nl2\nl3", "Gyj", "l1\nl2\nl3\n", (2, 0)),
        ("l1\nl2\nl3", "yk", "l1\nl2\nl3\n", (0, 0)),
        ("l1\nl2\nl3", "GcjX<Esc>", "l1\nl2\nl3\n", (2, 0)),
        ("l1\nl2\nl3", "ckX<Esc>", "l1\nl2\nl3\n", (0, 0)),
        ("l1\nl2\nl3", "G>j", "l1\nl2\nl3\n", (2, 0)),
        ("  l1\nl2\nl3", "<k", "  l1\nl2\nl3\n", (0, 0)),
        ("l1\nl2\nl3", "GgUj", "l1\nl2\nl3\n", (2, 0)),
        ("l1\nl2\nl3", "gUk", "l1\nl2\nl3\n", (0, 0)),
        ("l1", "dj", "l1\n", (0, 0)),
        ("l1", "dk", "l1\n", (0, 0)),
        ("l1\nl2\nl3", "Gd+", "l1\nl2\nl3\n", (2, 0)),
        ("l1\nl2\nl3", "d-", "l1\nl2\nl3\n", (0, 0)),
        ("l1\nl2\nl3", "Gd<CR>", "l1\nl2\nl3\n", (2, 0)),
        ("l1\nl2\nl3", "Gd2_", "l1\nl2\nl3\n", (2, 0)),
        ("l1\nl2\nl3", "Gy2_", "l1\nl2\nl3\n", (2, 0)),
    ]);
}

#[test]
fn a_count_past_the_edge_is_clamped() {
    check(&[
        ("l1\nl2\nl3", "2Gd2j", "l1\n", (0, 0)),
        ("l1\nl2\nl3", "2Gd5k", "l3\n", (0, 0)),
        ("l1\nl2\nl3", "d+", "l3\n", (0, 0)),
        ("l1\nl2\nl3", "Gd-", "l1\n", (0, 0)),
        ("l1\nl2\nl3", "d<CR>", "l3\n", (0, 0)),
        ("l1\nl2\nl3", "dgg", "l2\nl3\n", (0, 0)),
        ("l1\nl2\nl3", "GdG", "l1\nl2\n", (1, 0)),
        ("l1\nl2\nl3", "d_", "l2\nl3\n", (0, 0)),
        ("l1\nl2\nl3", "2Gd2_", "l1\n", (0, 0)),
        ("l1\nl2\nl3", "2G3dd", "l1\n", (0, 0)),
        ("l1\nl2\nl3", "2G3>>", "l1\n    l2\n    l3\n", (1, 4)),
        ("l1\nl2\nl3\nl4\nl5", "dd3j.", "l2\nl3\nl4\n", (2, 0)),
    ]);
}

#[test]
fn a_counted_doubled_operator_fails_on_the_last_line() {
    check(&[
        ("l1\nl2\nl3", "G2dd", "l1\nl2\nl3\n", (2, 0)),
        ("l1\nl2\nl3", "G2yy", "l1\nl2\nl3\n", (2, 0)),
        ("l1\nl2\nl3", "G2ccX<Esc>", "l1\nl2\nl3\n", (2, 0)),
        ("l1\nl2\nl3", "G2>>", "l1\nl2\nl3\n", (2, 0)),
        ("  l1\n  l2\n  l3", "G2<<", "  l1\n  l2\n  l3\n", (2, 2)),
        ("l1\nl2\nl3", "G2guu", "l1\nl2\nl3\n", (2, 0)),
        ("L1\nl2\nl3", "G2gUU", "L1\nl2\nl3\n", (2, 0)),
        ("l1\nl2\nl3", "G2g~~", "l1\nl2\nl3\n", (2, 0)),
        ("l1\nl2\nl3", "G2SX<Esc>", "l1\nl2\nl3\n", (2, 0)),
        ("l1\nl2\nl3", "G2Y", "l1\nl2\nl3\n", (2, 0)),
    ]);
}

#[test]
fn repeating_a_line_motion_at_the_edge_does_nothing() {
    check(&[
        (
            "l1\nl2\nl3\nl4",
            "2>>jjj.",
            "    l1\n    l2\nl3\nl4\n",
            (3, 1),
        ),
        (
            "l1\nl2\nl3\nl4",
            ">jjjj.",
            "    l1\n    l2\nl3\nl4\n",
            (3, 1),
        ),
        ("l1\nl2\nl3\nl4", ">>G2.", "    l1\nl2\nl3\nl4\n", (3, 0)),
        (
            "aaa\nbbb\nccc\nddd",
            ">jjjj.",
            "    aaa\n    bbb\nccc\nddd\n",
            (3, 2),
        ),
        ("l1\nl2\nl3\nl4", "jjdkgg.", "l1\nl4\n", (0, 0)),
        ("l1\nl2\nl3\nl4\nl5\nl6", "dj.", "l5\nl6\n", (0, 0)),
        ("l1\nl2\nl3\nl4\nl5\nl6", "2dd.", "l5\nl6\n", (0, 0)),
        (
            "l1\nl2\nl3\nl4\nl5\nl6",
            "2ddGp",
            "l3\nl4\nl5\nl6\nl1\nl2\n",
            (4, 0),
        ),
        ("l1\nl2\nl3\nl4", "2ddG.", "l3\nl4\n", (1, 0)),
        ("l1\nl2\nl3\nl4", "2ccX<Esc>G.", "X\nl3\nl4\n", (2, 0)),
        ("l1\nl2\nl3\nl4", "cjX<Esc>G.", "X\nl3\nl4\n", (2, 0)),
        (
            "  l1\n  l2\n  l3\n  l4",
            "2<<G.",
            "l1\nl2\n  l3\n  l4\n",
            (3, 2),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "2>>j.",
            "    l1\n        l2\n    l3\nl4\nl5\n",
            (1, 8),
        ),
    ]);
}

/// nvim: `yyGdj` leaves the unnamed register holding the yanked line.
#[test]
fn a_failed_dj_leaves_the_registers_alone() {
    let mut test = EditorTest::new("l1\nl2\nl3\n");
    test.keys("yyGdj");
    assert_eq!(test.get_register_content('"').as_deref(), Some("l1\n"));
    assert_eq!(test.buffer_content(), "l1\nl2\nl3\n");
}
