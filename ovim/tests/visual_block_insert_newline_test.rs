//! Visual-block `I`/`A` with typed text that contains a line break puts that text on the first
//! line only (vim does not copy it onto the other block lines), and block `c` that reaches the
//! end of the lines inserts after the last character. Every row was produced with
//! `nvim --clean --headless` (`normal! {keys}`): the buffer, the cursor as (line, column) and
//! the unnamed register.

mod helpers;
use helpers::EditorTest;

/// (content, keys, buffer, cursor, unnamed register)
type Row = (
    &'static str,
    &'static str,
    &'static str,
    (usize, usize),
    Option<&'static str>,
);

fn check(rows: &[Row]) {
    for &(content, keys, buffer, cursor, register) in rows {
        let mut test = EditorTest::new(&format!("{content}\n"));
        test.keys(keys);
        let what = format!("{keys} on {content:?}");
        assert_eq!(test.buffer_content(), buffer, "buffer after {what}");
        assert_eq!(test.cursor(), cursor, "cursor after {what}");
        if let Some(register) = register {
            assert_eq!(
                test.get_register_content('"').as_deref(),
                Some(register),
                "register after {what}"
            );
        }
    }
}

#[test]
fn block_insert_with_a_line_break_stays_on_the_first_line() {
    check(&[
        (
            "one two\nthree four\nfive six",
            "w<C-v>jIfoo<CR>bar<Esc>",
            "one foo\nbartwo\nthree four\nfive six\n",
            (1, 2),
            None,
        ),
        (
            "one two\nthree four\nfive six",
            "w<C-v>jjAfoo<CR>bar<Esc>",
            "one tfoo\nbarwo\nthree four\nfive six\n",
            (1, 2),
            None,
        ),
        (
            "ab\ncd\nef",
            "<C-v>jIx<CR>y<Esc>",
            "x\nyab\ncd\nef\n",
            (1, 0),
            None,
        ),
        (
            "ab\ncd\nef\ngh",
            "<C-v>jIx<CR>y<Esc>jj.",
            "x\nyab\ncd\nx\nyef\ngh\n",
            (4, 0),
            None,
        ),
        (
            "one two\nthree four",
            "w<C-v>jIfoo<Esc>",
            "one footwo\nthrefooe four\n",
            (0, 4),
            None,
        ),
        (
            "one\ntwo\nthree",
            "<C-v>j$Afoo<CR>bar<Esc>",
            "onefoo\nbar\ntwo\nthree\n",
            (1, 2),
            None,
        ),
        (
            "ab\ncd\nef",
            "<C-v>jIx<CR>y<Esc>u",
            "ab\ncd\nef\n",
            (0, 0),
            None,
        ),
    ]);
}

#[test]
fn block_change_reaching_the_end_of_the_line() {
    check(&[
        (
            "abc\ndef",
            "l<C-v>j$cX<Esc>",
            "aX\ndX\n",
            (0, 1),
            Some("bc\nef"),
        ),
        (
            "abc\ndef",
            "$<C-v>jcX<Esc>",
            "abX\ndeX\n",
            (0, 2),
            Some("c\nf"),
        ),
        (
            "abc\ndef\nghi",
            "ll<C-v>jjcX<Esc>",
            "abX\ndeX\nghX\n",
            (0, 2),
            Some("c\nf\ni"),
        ),
        (
            "abc\nd\nghi",
            "ll<C-v>jjIX<Esc>",
            "abXc\nd\nghXi\n",
            (0, 2),
            None,
        ),
        (
            "abc\nd\nghi",
            "ll<C-v>jjAX<Esc>",
            "abcX\nd  X\nghiX\n",
            (0, 2),
            None,
        ),
        (
            "abc\nd\nghi",
            "l<C-v>jj$AX<Esc>",
            "abcX\ndX\nghiX\n",
            (0, 1),
            None,
        ),
    ]);
}
