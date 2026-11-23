//! Jump motions (`/`, `?`, `n`, `N`, `*`, `#`, `%`, `(`, `)`, `{`, `}`, `H`, `M`, `L`, `[[`, `]]`) record the
//! position they leave so `<C-o>` returns to it (a jump that stays on the same line has nowhere
//! to go back to), and `*`/`#` set the last search pattern for `:s//`. Every row was produced with
//! `nvim --clean --headless` (`normal! {keys}`): the buffer, the cursor as (line, column) and the
//! unnamed register.

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
fn jumps_return_with_ctrl_o() {
    check(&[
        (
            "l1\nl2 foo\nl3\nl4 foo\nl5",
            "/foo<CR><C-o>",
            "l1\nl2 foo\nl3\nl4 foo\nl5\n",
            (0, 0),
            None,
        ),
        (
            "l1\nl2 foo\nl3\nl4 foo\nl5",
            "/foo<CR>/foo<CR><C-o>",
            "l1\nl2 foo\nl3\nl4 foo\nl5\n",
            (1, 3),
            None,
        ),
        (
            "l1\nl2 foo\nl3\nl4 foo\nl5",
            "/foo<CR>/foo<CR><C-o><C-o>",
            "l1\nl2 foo\nl3\nl4 foo\nl5\n",
            (0, 0),
            None,
        ),
        (
            "l1\nl2 foo\nl3\nl4 foo\nl5",
            "/foo<CR>/foo<CR><C-o><C-o><Tab>",
            "l1\nl2 foo\nl3\nl4 foo\nl5\n",
            (1, 3),
            None,
        ),
        (
            "l1\nl2 foo\nl3\nl4 foo\nl5",
            "G?foo<CR><C-o>",
            "l1\nl2 foo\nl3\nl4 foo\nl5\n",
            (4, 0),
            None,
        ),
        (
            "l1\nl2 foo\nl3\nl4 foo\nl5\nl6 foo",
            "/foo<CR>n<C-o>",
            "l1\nl2 foo\nl3\nl4 foo\nl5\nl6 foo\n",
            (1, 3),
            None,
        ),
        (
            "l1\nl2 foo\nl3\nl4 foo\nl5\nl6 foo",
            "/foo<CR>N<C-o>",
            "l1\nl2 foo\nl3\nl4 foo\nl5\nl6 foo\n",
            (1, 3),
            None,
        ),
        (
            "l1\nl2 foo\nl3\nl4 foo\nl5",
            "j$b*<C-o>",
            "l1\nl2 foo\nl3\nl4 foo\nl5\n",
            (1, 3),
            None,
        ),
        (
            "l1\nl2 foo\nl3\nl4 foo\nl5",
            "jjj$b#<C-o>",
            "l1\nl2 foo\nl3\nl4 foo\nl5\n",
            (3, 3),
            None,
        ),
        ("f(a, b)\nxyz", "%<C-o>", "f(a, b)\nxyz\n", (0, 6), None),
        ("f(a, b)\nxyz", "f(%<C-o>", "f(a, b)\nxyz\n", (0, 6), None),
        (
            "a\nb\n\nc\nd\n\ne",
            "}<C-o>",
            "a\nb\n\nc\nd\n\ne\n",
            (0, 0),
            None,
        ),
        (
            "a\nb\n\nc\nd\n\ne",
            "}}<C-o>",
            "a\nb\n\nc\nd\n\ne\n",
            (2, 0),
            None,
        ),
        (
            "a\nb\n\nc\nd\n\ne",
            "}}<C-o><C-o>",
            "a\nb\n\nc\nd\n\ne\n",
            (0, 0),
            None,
        ),
        (
            "a\nb\n\nc\nd\n\ne",
            "G{<C-o>",
            "a\nb\n\nc\nd\n\ne\n",
            (6, 0),
            None,
        ),
        (
            "One. Two. Three.",
            ")<C-o>",
            "One. Two. Three.\n",
            (0, 5),
            None,
        ),
        (
            "One. Two. Three.",
            "$(<C-o>",
            "One. Two. Three.\n",
            (0, 10),
            None,
        ),
        ("a\nb\nc\nd", "GH<C-o>", "a\nb\nc\nd\n", (3, 0), None),
        ("a\nb\nc\nd", "L<C-o>", "a\nb\nc\nd\n", (0, 0), None),
        ("a\nb\nc\nd", "M<C-o>", "a\nb\nc\nd\n", (0, 0), None),
        (
            "a\n{\nb\n}\nc\n{\nd\n}",
            "]]<C-o>",
            "a\n{\nb\n}\nc\n{\nd\n}\n",
            (0, 0),
            None,
        ),
        (
            "a\n{\nb\n}\nc\n{\nd\n}",
            "G[[<C-o>",
            "a\n{\nb\n}\nc\n{\nd\n}\n",
            (7, 0),
            None,
        ),
        ("a\nb\nc", "G<C-o>", "a\nb\nc\n", (0, 0), None),
        ("a\nb\nc", "Ggg<C-o>", "a\nb\nc\n", (2, 0), None),
        ("a\nb\nc", "jmaG'a<C-o>", "a\nb\nc\n", (2, 0), None),
        (
            "a b\nc d\ne f",
            "/d<CR>/f<CR><C-o>x",
            "a b\nc \ne f\n",
            (1, 1),
            Some("d"),
        ),
        ("foo x foo y foo", "*n", "foo x foo y foo\n", (0, 12), None),
        (
            "foo x foo y foo z foo",
            "2*<C-o>",
            "foo x foo y foo z foo\n",
            (0, 12),
            None,
        ),
    ]);
}

#[test]
fn star_and_hash_set_the_search_pattern() {
    check(&[
        (
            "foo bar foo baz foo",
            "*:%s//Y/g<CR>",
            "Y bar Y baz Y\n",
            (0, 0),
            None,
        ),
        (
            "foo bar foo baz foo",
            "$b#:%s//Y/g<CR>",
            "Y bar Y baz Y\n",
            (0, 0),
            None,
        ),
        ("foo bar foo", "*:s//Z/<CR>", "Z bar foo\n", (0, 0), None),
        (
            "foo bar foo",
            "/foo<CR>:s//Z/<CR>",
            "Z bar foo\n",
            (0, 0),
            None,
        ),
    ]);
}
