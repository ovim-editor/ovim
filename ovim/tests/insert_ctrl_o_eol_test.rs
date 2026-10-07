//! Insert-mode Ctrl-O at the end of a line: the temporary Normal mode clamps the cursor onto
//! the last character, and returning to Insert mode puts it back past the end (vim's
//! `ins_at_eol`) -- also after `$`, or after `j`/`k` left a goal column beyond the text. Every row
//! was produced with `nvim --clean --headless` (`normal! {keys}`): the buffer, the cursor as
//! (line, column) and the unnamed register.

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
fn ctrl_o_at_end_of_line_returns_past_the_end() {
    check(&[
        ("abc", "AX<C-o>zzY<Esc>", "abcXY\n", (0, 4), None),
        (
            "foo bar",
            "ifoo<C-o>$bar<Esc>",
            "foofoo barbar\n",
            (0, 12),
            None,
        ),
        ("abc def", "i<C-o>$X<Esc>", "abc defX\n", (0, 7), None),
        ("abc def", "llli<C-o>zzX<Esc>", "abcX def\n", (0, 3), None),
        ("abc def", "A<C-o>hX<Esc>", "abc dXef\n", (0, 5), None),
        ("abc def", "A<C-o>0X<Esc>", "Xabc def\n", (0, 0), None),
        ("abc\ndef", "A<C-o>jX<Esc>", "abc\ndefX\n", (1, 3), None),
        ("abcdef\nxy", "A<C-o>jX<Esc>", "abcdef\nxyX\n", (1, 2), None),
        (
            "xy\nabcdef",
            "jA<C-o>kX<Esc>",
            "xyX\nabcdef\n",
            (0, 2),
            None,
        ),
        ("abc", "A<C-o>xY<Esc>", "abY\n", (0, 2), Some("c")),
        ("\nabc", "A<C-o>zzX<Esc>", "X\nabc\n", (0, 0), None),
        ("abc def", "A<C-o>dbX<Esc>", "abc fX\n", (0, 5), Some("de")),
        ("  abc", "A!<C-o>^X<Esc>", "  Xabc!\n", (0, 2), None),
        ("abc\ndef", "jA!<C-o>ggX<Esc>", "Xabc\ndef!\n", (0, 0), None),
        (
            "abc",
            "yiwA <C-o>pX<Esc>",
            "abc abcX\n",
            (0, 7),
            Some("abc"),
        ),
        ("abc", "A!<C-o>2hX<Esc>", "aXbc!\n", (0, 1), None),
        ("abc\ndef", "A<C-o>GX<Esc>", "abc\nXdef\n", (1, 0), None),
        (
            "abc\ndefgh",
            "i<C-o>$<C-o>jX<Esc>",
            "abc\ndefghX\n",
            (1, 5),
            None,
        ),
    ]);
}
