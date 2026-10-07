//! Insert-mode `<C-u>` deletes the text typed in this insert on the line first; with none left it
//! deletes what is before the cursor up to the indent, then the indent, then the line break.
//! Every row was produced with `nvim --clean --headless` (`normal! {keys}`, default
//! `backspace=indent,eol,start`): the buffer, the cursor as (line, column) and the unnamed register.

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
fn ctrl_u_in_insert_mode() {
    check(&[
        (
            "hello world",
            "A foo<C-u>X<Esc>",
            "hello worldX\n",
            (0, 11),
            None,
        ),
        ("hello world", "A foo<C-u><C-u>X<Esc>", "X\n", (0, 0), None),
        ("hello world", "A<C-u>X<Esc>", "X\n", (0, 0), None),
        ("hello world", "wi<C-u>X<Esc>", "Xworld\n", (0, 0), None),
        (
            "hello world",
            "wiab<C-u>X<Esc>",
            "hello Xworld\n",
            (0, 6),
            None,
        ),
        (
            "hello world",
            "wiab<C-u><C-u>X<Esc>",
            "Xworld\n",
            (0, 0),
            None,
        ),
        ("  hello", "A x<C-u>X<Esc>", "  helloX\n", (0, 7), None),
        ("  hello", "A x<C-u><C-u>X<Esc>", "  X\n", (0, 2), None),
        ("  hello", "A x<C-u><C-u><C-u>X<Esc>", "X\n", (0, 0), None),
        ("hello", "0i<C-u>X<Esc>", "Xhello\n", (0, 0), None),
        ("hello", "oab<C-u>X<Esc>", "hello\nX\n", (1, 0), None),
        ("hello", "oab<C-u><C-u>X<Esc>", "helloX\n", (0, 5), None),
        ("  hello", "oab<C-u>X<Esc>", "  hello\n  X\n", (1, 2), None),
        ("hello", "Aab<BS><C-u>X<Esc>", "helloX\n", (0, 5), None),
        ("foo bar", "wiXY<C-u>Z<Esc>", "foo Zbar\n", (0, 4), None),
        (
            "hello",
            "Aab<CR>cd<C-u>X<Esc>",
            "helloab\nX\n",
            (1, 0),
            None,
        ),
        (
            "hello",
            "Aab<CR>cd<C-u><C-u>X<Esc>",
            "helloabX\n",
            (0, 7),
            None,
        ),
        (
            "hello",
            "Aab<CR>cd<C-u><C-u><C-u>X<Esc>",
            "helloX\n",
            (0, 5),
            None,
        ),
        ("héllo 👍", "A🎉é<C-u>X<Esc>", "héllo 👍X\n", (0, 7), None),
        ("hello", "aab<C-u>X<Esc>", "hXello\n", (0, 1), None),
        (
            "hello world",
            "wcwab<C-u>X<Esc>",
            "hello X\n",
            (0, 6),
            Some("world"),
        ),
        (
            "hello world",
            "wcwab<C-u><C-u>X<Esc>",
            "X\n",
            (0, 0),
            Some("world"),
        ),
        (
            "hello world",
            "Aab cd<C-w><C-u>X<Esc>",
            "hello worldX\n",
            (0, 11),
            None,
        ),
    ]);
}
