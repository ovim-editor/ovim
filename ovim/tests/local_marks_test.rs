//! Marks belong to their buffer and move with the text: `a`-`z` survive buffer switches, line
//! inserts, deletes and joins adjust them, and leaving Visual mode records `'<` and `'>`.
//! Every row was produced with `nvim --clean --headless` (`normal! {keys}` ending in a jump to the
//! mark): the buffer and the cursor as (line, column) after that jump. A deleted mark makes the
//! jump fail, which the rows reach by moving the cursor elsewhere first.

mod helpers;
use helpers::EditorTest;
use ovim::buffer::Buffer;

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
fn lines_inserted_above_a_mark_move_it() {
    check(&[
        (
            "l1\nl2\nl3\nl4\nl5",
            "3Gma1GOnew<Esc>'a",
            "new\nl1\nl2\nl3\nl4\nl5\n",
            (3, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "3Gma1Gonew<Esc>'a",
            "l1\nnew\nl2\nl3\nl4\nl5\n",
            (3, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "3GmaOnew<Esc>'a",
            "l1\nl2\nnew\nl3\nl4\nl5\n",
            (3, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "4Gma1Gyyp'a",
            "l1\nl1\nl2\nl3\nl4\nl5\n",
            (4, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "4Gma1GyyP'a",
            "l1\nl1\nl2\nl3\nl4\nl5\n",
            (4, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "3GmayyP'a",
            "l1\nl2\nl3\nl3\nl4\nl5\n",
            (3, 0),
        ),
        (
            "abc def\nl2\nl3",
            "3Gma1Gwi<CR><Esc>'a",
            "abc \ndef\nl2\nl3\n",
            (3, 0),
        ),
    ]);
}

#[test]
fn lines_inserted_below_a_mark_leave_it() {
    check(&[
        (
            "l1\nl2\nl3\nl4\nl5",
            "3Gmaonew<Esc>'a",
            "l1\nl2\nl3\nnew\nl4\nl5\n",
            (2, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "3Gmayyp'a",
            "l1\nl2\nl3\nl3\nl4\nl5\n",
            (2, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "2Gma4Gyyp'a",
            "l1\nl2\nl3\nl4\nl4\nl5\n",
            (1, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "3Gma1Gyj'a",
            "l1\nl2\nl3\nl4\nl5\n",
            (2, 0),
        ),
    ]);
}

#[test]
fn deleting_lines_above_a_mark_moves_it_up() {
    check(&[
        (
            "l1\nl2\nl3\nl4\nl5",
            "4Gma1Gdd'a",
            "l2\nl3\nl4\nl5\n",
            (2, 0),
        ),
        ("l1\nl2\nl3\nl4\nl5", "4Gma1G2dd'a", "l3\nl4\nl5\n", (1, 0)),
        (
            "l1\nl2\nl3\nl4\nl5",
            "2Gma4Gdd'a",
            "l1\nl2\nl3\nl5\n",
            (1, 0),
        ),
        ("l1\nl2\nl3\nl4\nl5", "3Gma2Gdj'a", "l1\nl4\nl5\n", (1, 0)),
        (
            "l1\nl2\nl3\nl4\nl5",
            "4Gma:1,2d<CR>'a",
            "l3\nl4\nl5\n",
            (1, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "5Gma1G2cchi<Esc>'a",
            "hi\nl3\nl4\nl5\n",
            (3, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "3Gma5Gmbgg3Gdd'b",
            "l1\nl2\nl4\nl5\n",
            (3, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "4Gma2G2cchi<Esc>gg'a",
            "l1\nhi\nl4\nl5\n",
            (2, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "5Gma2G2cchi<Esc>gg'a",
            "l1\nhi\nl4\nl5\n",
            (3, 0),
        ),
    ]);
}

#[test]
fn a_deleted_line_loses_its_mark() {
    check(&[
        (
            "l1\nl2\nl3\nl4\nl5",
            "3Gmaddgg'a",
            "l1\nl2\nl4\nl5\n",
            (0, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "5Gmaddgg'a",
            "l1\nl2\nl3\nl4\n",
            (0, 0),
        ),
        ("l1\nl2\nl3\nl4\nl5", "3Gma1G3ddG'a", "l4\nl5\n", (1, 0)),
        (
            "l1\nl2\nl3\nl4\nl5",
            "2Gma:1,2d<CR>G'a",
            "l3\nl4\nl5\n",
            (2, 0),
        ),
    ]);
}

#[test]
fn joining_lines_moves_marks_with_the_text() {
    check(&[
        (
            "l1\nl2\nl3\nl4\nl5",
            "4Gma1GJ'a",
            "l1 l2\nl3\nl4\nl5\n",
            (2, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "3Gma2GJ'a",
            "l1\nl2 l3\nl4\nl5\n",
            (1, 0),
        ),
        (
            "l1\nl2 x\nl3 y\nl4\nl5",
            "3G$ma2GJ`a",
            "l1\nl2 x l3 y\nl4\nl5\n",
            (1, 8),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "2GmaJ`a",
            "l1\nl2 l3\nl4\nl5\n",
            (1, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "4Gma2GJ'a",
            "l1\nl2 l3\nl4\nl5\n",
            (2, 0),
        ),
    ]);
}

#[test]
fn changing_lines_keeps_the_marks_of_the_first() {
    check(&[
        (
            "l1\nl2\nl3\nl4\nl5",
            "4Gma1Gcchi<Esc>'a",
            "hi\nl2\nl3\nl4\nl5\n",
            (3, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "3Gmacchi<Esc>gg'a",
            "l1\nl2\nhi\nl4\nl5\n",
            (2, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "3G$macchi<Esc>gg`a",
            "l1\nl2\nhi\nl4\nl5\n",
            (2, 1),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "2Gma2G2cchi<Esc>gg'a",
            "l1\nhi\nl4\nl5\n",
            (1, 0),
        ),
    ]);
}

#[test]
fn edits_inside_a_line_leave_marks_alone() {
    check(&[
        (
            "abc def\nl2\nl3",
            "wmaIx<CR><Esc>`a",
            "x\nabc def\nl2\nl3\n",
            (0, 0),
        ),
        (
            "abc def\nl2\nl3",
            "wma0lli<CR><Esc>`a",
            "ab\nc def\nl2\nl3\n",
            (0, 1),
        ),
        (
            "abc def\nl2\nl3",
            "0mawi<CR><Esc>`a",
            "abc \ndef\nl2\nl3\n",
            (0, 0),
        ),
        (
            "abc def\nl2\nl3",
            "wmaIxx<Esc>`a",
            "xxabc def\nl2\nl3\n",
            (0, 4),
        ),
        ("abc def\nl2\nl3", "wma0xx`a", "c def\nl2\nl3\n", (0, 4)),
        (
            "one two\nthree four\nfive",
            "2Gwma1Gwdw`a",
            "one \nthree four\nfive\n",
            (1, 6),
        ),
        (
            "one two\nthree four\nfive six\nseven",
            "3Gma1G/five<CR>d?two<CR>'a",
            "one \nfive six\nseven\n",
            (1, 0),
        ),
    ]);
}

#[test]
fn undo_and_redo_move_marks_back() {
    check(&[
        (
            "l1\nl2\nl3\nl4\nl5",
            "4Gma1Gddu'a",
            "l1\nl2\nl3\nl4\nl5\n",
            (3, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "3Gma1GOnew<Esc>u'a",
            "l1\nl2\nl3\nl4\nl5\n",
            (2, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "4Gma1Gddu<C-r>'a",
            "l2\nl3\nl4\nl5\n",
            (2, 0),
        ),
    ]);
}

#[test]
fn visual_marks_record_the_selection() {
    check(&[
        ("a\na\na\na\na", "jVj<Esc>gg'>", "a\na\na\na\na\n", (2, 0)),
        ("a\na\na\na\na", "jVj<Esc>G'<", "a\na\na\na\na\n", (1, 0)),
        (
            "abcdef\nabcdef\nabcdef",
            "lvjl<Esc>gg`<",
            "abcdef\nabcdef\nabcdef\n",
            (0, 1),
        ),
        (
            "abcdef\nabcdef\nabcdef",
            "lvjl<Esc>gg`>",
            "abcdef\nabcdef\nabcdef\n",
            (1, 2),
        ),
        (
            "abcdef\nabcdef\nabcdef",
            "3Gllvkh<Esc>gg`<",
            "abcdef\nabcdef\nabcdef\n",
            (1, 1),
        ),
        (
            "abcdef\nabcdef\nabcdef",
            "3Gllvkh<Esc>gg`>",
            "abcdef\nabcdef\nabcdef\n",
            (2, 2),
        ),
        (
            "abc\nabc\nabc",
            "l<C-v>jl<Esc>gg`<",
            "abc\nabc\nabc\n",
            (0, 1),
        ),
        (
            "abc\nabc\nabc",
            "l<C-v>jl<Esc>gg`>",
            "abc\nabc\nabc\n",
            (1, 2),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "2GVjy3G`<",
            "l1\nl2\nl3\nl4\nl5\n",
            (1, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "2GVjyG`>",
            "l1\nl2\nl3\nl4\nl5\n",
            (2, 1),
        ),
        ("l1\nl2\nl3\nl4\nl5", "3G'<", "l1\nl2\nl3\nl4\nl5\n", (2, 0)),
        ("l1\nl2\nl3\nl4\nl5", "2GVj<Esc>4Gd'<", "l1\nl5\n", (1, 0)),
        ("abcdef\nabcdef", "lvl<Esc>$d`<", "af\nabcdef\n", (0, 1)),
        (
            "l1\nl2\nl3\nl4\nl5",
            "2GVj<Esc>1Gy'>",
            "l1\nl2\nl3\nl4\nl5\n",
            (0, 0),
        ),
    ]);
}

#[test]
fn visual_marks_address_ex_ranges() {
    check(&[
        (
            "a\na\na\na\na",
            "jVj<Esc>:'<lt>,'>s/a/X/<CR>",
            "a\nX\nX\na\na\n",
            (2, 0),
        ),
        ("a\na\na\na\na", "jVj:s/a/X/<CR>", "a\nX\nX\na\na\n", (2, 0)),
        (
            "l1\nl2\nl3\nl4\nl5",
            "2GVj<Esc>:'<lt>,'>d<CR>",
            "l1\nl4\nl5\n",
            (1, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "2GVj<Esc>:'<lt>d<CR>",
            "l1\nl3\nl4\nl5\n",
            (1, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "3Gma2GVj<Esc>:'a,'>d<CR>",
            "l1\nl2\nl4\nl5\n",
            (2, 0),
        ),
    ]);
}

#[test]
fn visual_marks_follow_edits() {
    check(&[
        ("l1\nl2\nl3\nl4\nl5", "5Gma1GVjjd'a", "l4\nl5\n", (1, 0)),
        ("l1\nl2\nl3\nl4\nl5", "jVjdG'<", "l1\nl4\nl5\n", (1, 0)),
        (
            "l1\nl2\nl3\nl4\nl5",
            "3GVj<Esc>1GOnew<Esc>gg'<",
            "new\nl1\nl2\nl3\nl4\nl5\n",
            (3, 0),
        ),
        (
            "l1\nl2\nl3\nl4\nl5",
            "3GVj<Esc>1Gddgg'>",
            "l2\nl3\nl4\nl5\n",
            (2, 0),
        ),
    ]);
}

/// nvim: `3Gma` in one file, `:bnext`, `:bprev` — each buffer keeps its own marks, and a mark
/// set in one is not visible in the other.
#[test]
fn marks_stay_with_their_buffer_across_switches() {
    let mut test = EditorTest::new("a1\na2\na3\na4\na5\n");
    test.keys("3Gma");
    test.editor.add_buffer(Buffer::new_from_str("b1\nb2\nb3\n"));
    assert_eq!(test.editor.buffer().local_mark('a'), None);

    test.keys("2Gmb");
    test.keys(":bn<CR>");
    assert_eq!(test.buffer_content(), "a1\na2\na3\na4\na5\n");
    assert_eq!(test.editor.buffer().local_mark('b'), None);
    test.keys("gg'a");
    assert_eq!(test.cursor(), (2, 0));

    test.keys(":bp<CR>");
    assert_eq!(test.buffer_content(), "b1\nb2\nb3\n");
    assert_eq!(test.editor.buffer().local_mark('a'), None);
    test.keys("gg'b");
    assert_eq!(test.cursor(), (1, 0));
}

/// nvim: a mark set in a buffer that is switched away from and back to is still there, at its
/// column, and `:marks` lists the current buffer's marks.
#[test]
fn marks_keep_their_column_across_switches() {
    let mut test = EditorTest::new("hello world\nsecond\n");
    test.keys("wma");
    test.editor.add_buffer(Buffer::new_from_str("other\n"));
    test.keys(":bn<CR>:bp<CR>:bn<CR>");
    test.keys("G`a");
    assert_eq!(test.cursor(), (0, 6));
}

/// nvim: `Vj<Esc>` then `:'<,'>s/a/X/` substitutes on the two selected lines; the marks keep
/// working after the selection is gone and are per buffer.
#[test]
fn visual_marks_belong_to_the_buffer_they_were_set_in() {
    let mut test = EditorTest::new("a\na\na\na\n");
    test.keys("jVj<Esc>");
    assert_eq!(
        test.editor.buffer().local_marks().len(),
        2,
        "the Visual marks"
    );
    test.editor.add_buffer(Buffer::new_from_str("a\na\na\n"));
    test.keys(":bn<CR>");
    test.keys(":'<lt>,'>s/a/X/<CR>");
    assert_eq!(test.buffer_content(), "a\nX\nX\na\n");

    test.keys(":bp<CR>");
    test.keys(":'<lt>,'>s/a/X/<CR>");
    assert_eq!(
        test.buffer_content(),
        "a\na\na\n",
        "E20: no marks in this buffer"
    );
}
