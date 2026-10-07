//! `.` after a command that changed the text must repeat that command, never
//! an older one. Expectations come from `nvim --clean --headless` (`normal!`).

mod helpers;
use helpers::EditorTest;

fn run(content: &str, keys: &str) -> EditorTest {
    let mut test = EditorTest::new(content);
    test.keys(keys);
    test
}

#[test]
fn case_operators_with_motions_are_repeatable() {
    // nvim: `ddgUwj.` -> the `.` uppercases a word, it does not delete a line.
    let test = run("one two\nthree four\nfive six\nseven\n", "ddgUwj.");
    assert_eq!(test.buffer_content(), "THREE four\nFIVE six\nseven\n");
    test.assert_cursor(1, 0);

    // nvim: `gUww.` -> "ONE TWO three four"; same for gu / g~ and for `e`.
    assert_eq!(
        run("one two three four\n", "gUww.").buffer_content(),
        "ONE TWO three four\n"
    );
    assert_eq!(
        run("ONE TWO THREE FOUR\n", "guww.").buffer_content(),
        "one two THREE FOUR\n"
    );
    assert_eq!(
        run("one two three four\n", "g~ww.").buffer_content(),
        "ONE TWO three four\n"
    );
    assert_eq!(
        run("one two three four\n", "gUewe.").buffer_content(),
        "ONE twO THREE four\n"
    );
    // nvim: `wgU$j0w.` uppercases to the end of the line again.
    assert_eq!(
        run("one two\nthree four\n", "wgU$j0w.").buffer_content(),
        "one TWO\nthree FOUR\n"
    );
}

#[test]
fn doubled_case_operators_are_repeatable() {
    // nvim: `gUUj.`, `g~~j.` -> two lines changed.
    assert_eq!(
        run("one\ntwo\nthree\nfour\n", "gUUj.").buffer_content(),
        "ONE\nTWO\nthree\nfour\n"
    );
    assert_eq!(
        run("one\ntwo\nthree\nfour\n", "g~~j.").buffer_content(),
        "ONE\nTWO\nthree\nfour\n"
    );
    assert_eq!(
        run("ONE\nTWO\nTHREE\nFOUR\n", "guuj.").buffer_content(),
        "one\ntwo\nTHREE\nFOUR\n"
    );
    // nvim: `2gUUjj.` repeats with the same count.
    assert_eq!(
        run("one\ntwo\nthree\nfour\nfive\n", "2gUUjj.").buffer_content(),
        "ONE\nTWO\nTHREE\nFOUR\nfive\n"
    );
    // nvim: an unrelated earlier `x` is not replayed (xjgUUj. -> "bc\nDEF\nGHI\njkl").
    assert_eq!(
        run("abc\ndef\nghi\njkl\n", "xjgUUj.").buffer_content(),
        "bc\nDEF\nGHI\njkl\n"
    );
}

#[test]
fn case_operator_on_text_object_moves_to_its_start_and_repeats_even_when_unchanged() {
    // nvim: `wlguiw` leaves the cursor on the start of the word.
    let test = run("ONE TWO THREE\n", "wlguiw");
    assert_eq!(test.buffer_content(), "ONE two THREE\n");
    test.assert_cursor(0, 4);
    // nvim: `wlgUiw` on an already upper-case word still moves there.
    run("ONE TWO THREE\n", "wlgUiw").assert_cursor(0, 4);
    // nvim: `guiww.` repeats the operator on the next word.
    let test = run("ONE TWO THREE\n", "guiww.");
    assert_eq!(test.buffer_content(), "one two THREE\n");
    test.assert_cursor(0, 4);
}

#[test]
fn visual_case_and_replace_are_repeatable_on_the_same_amount_of_text() {
    // nvim: `vlUw.` -> "ONe TWo three four five", `vluw.`, `vl~w.`, `vlrxw.`.
    assert_eq!(
        run("one two three four five\n", "vlUw.").buffer_content(),
        "ONe TWo three four five\n"
    );
    assert_eq!(
        run("ONE TWO THREE FOUR FIVE\n", "vluw.").buffer_content(),
        "onE twO THREE FOUR FIVE\n"
    );
    assert_eq!(
        run("one two three four five\n", "vl~w.").buffer_content(),
        "ONe TWo three four five\n"
    );
    assert_eq!(
        run("one two three four five\n", "vlrxw.").buffer_content(),
        "xxe xxo three four five\n"
    );
    // nvim: `VUjj.` / `Vr-jj.` repeat on one line; `vjUjj.` on two lines.
    assert_eq!(
        run("one\ntwo\nthree\nfour\n", "VUjj.").buffer_content(),
        "ONE\ntwo\nTHREE\nfour\n"
    );
    assert_eq!(
        run("one\ntwo\nthree\nfour\n", "Vr-jj.").buffer_content(),
        "---\ntwo\n-----\nfour\n"
    );
    let test = run("ab\ncd\nef\ngh\nij\n", "vjUjj.");
    assert_eq!(test.buffer_content(), "AB\nCd\nEF\nGh\nij\n");
    test.assert_cursor(2, 0);
}

#[test]
fn empty_insert_replaces_the_repeat_with_a_no_op() {
    // nvim: `ddi<Esc>j.` -> only the first line was deleted; `.` repeats the empty insert.
    for keys in ["ddi<Esc>j.", "dda<Esc>j."] {
        assert_eq!(
            run("one\ntwo\nthree\nfour\n", keys).buffer_content(),
            "two\nthree\nfour\n",
            "{keys}"
        );
    }
    // nvim: `ddo<Esc>j.` opens a second blank line.
    assert_eq!(
        run("one\ntwo\nthree\nfour\n", "ddo<Esc>j.").buffer_content(),
        "two\n\nthree\n\nfour\n"
    );
}

#[test]
fn mark_operators_do_not_replay_an_older_change() {
    // nvim: `jjmakd'ax.` -- d'a is not repeated by `.`, and `x` is its own change.
    let test = run("a\nb\nc\nd\ne\nf\ng\n", "jjmakd'ax.");
    assert_eq!(test.buffer_content(), "a\n\ne\nf\ng\n");
    // nvim: `jjmakc'aX<Esc>j.` -- the `.` finds the mark gone and changes nothing.
    let test = run("a\nb\nc\nd\ne\nf\ng\n", "jjmakc'aX<Esc>j.");
    assert_eq!(test.buffer_content(), "a\nX\nd\ne\nf\ng\n");
}

#[test]
fn ex_commands_do_not_replace_the_repeat() {
    // nvim: "abc abc", `x:s/c/Z/<CR>.` -> "Z abc": `.` still repeats the `x`.
    let mut test = run("abc abc\nxyz\n", "x");
    test.keys(":s/c/Z/<CR>");
    test.keys(".");
    assert_eq!(test.buffer_content(), "Z abc\nxyz\n");
}

#[test]
fn undo_does_not_replace_the_repeat() {
    // (`u` inside one `nvim -c normal!` is a single undo block, so this is checked
    // by hand in nvim: dd, j, u, `.` deletes a line again.)
    let test = run("a\nb\nc\nd\n", "ddju.");
    assert_eq!(test.buffer_content(), "b\nc\nd\n");
}

#[test]
fn dgn_repeats_as_delete_of_the_next_match() {
    // nvim: "x foo y foo z foo", `/foo<CR>dgn.` -> "x  y  z foo" (not "delete three characters").
    let test = run("x foo y foo z foo\n", "/foo<CR>dgn.");
    assert_eq!(test.buffer_content(), "x  y  z foo\n");
    // nvim: `cgnQ<Esc>.` -> "x Q y Q z foo".
    let test = run("x foo y foo z foo\n", "/foo<CR>cgnQ<Esc>.");
    assert_eq!(test.buffer_content(), "x Q y Q z foo\n");
}
