//! Counts on `i`/`a`/`I`/`A`/`o`/`O` (the typed text is inserted that many times), on text objects
//! (`d2aw`, `c2ip`, `d2i(`), and `[count].` after them. Every row was produced with
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
fn counted_change_and_delete_repeat_with_a_count() {
    check(&[
        (
            "abc\ndef",
            "3ccfoo<Esc>",
            "foo\n",
            (0, 2),
            Some("abc\ndef\n"),
        ),
        ("abc", "2sfoo<Esc>", "fooc\n", (0, 2), Some("ab")),
        (
            "foo bar baz qux",
            "ciwX<Esc>w2.",
            "X Xbaz qux\n",
            (0, 2),
            Some("bar "),
        ),
        (
            "foo bar baz",
            "ciwX<Esc>w.",
            "X X baz\n",
            (0, 2),
            Some("bar"),
        ),
        (
            "foo bar baz qux",
            "cwX<Esc>w2.",
            "X X qux\n",
            (0, 2),
            Some("bar baz"),
        ),
        (
            "foo bar baz qux",
            "diww2.",
            " baz qux\n",
            (0, 1),
            Some("bar "),
        ),
        (
            "foo bar baz qux",
            "dawl2.",
            "qux\n",
            (0, 0),
            Some("bar baz "),
        ),
    ]);
}

#[test]
fn counted_insert_commands() {
    check(&[
        ("abc", "3ifoo<Esc>", "foofoofooabc\n", (0, 8), None),
        ("abc", "3afoo<Esc>", "afoofoofoobc\n", (0, 9), None),
        ("  abc", "3Ifoo<Esc>", "  foofoofooabc\n", (0, 10), None),
        ("abc", "3Afoo<Esc>", "abcfoofoofoo\n", (0, 11), None),
        (
            "abc\ndef",
            "3ofoo<Esc>",
            "abc\nfoo\nfoo\nfoo\ndef\n",
            (3, 2),
            None,
        ),
        ("abc\ndef", "jO3<Esc>", "abc\n3\ndef\n", (1, 0), None),
        (
            "abc\ndef",
            "j3Ofoo<Esc>",
            "abc\nfoo\nfoo\nfoo\ndef\n",
            (3, 2),
            None,
        ),
        (
            "abc",
            "80i-<Esc>",
            "--------------------------------------------------------------------------------abc\n",
            (0, 79),
            None,
        ),
        (
            "abc",
            "2ofoo<CR>bar<Esc>",
            "abc\nfoo\nbar\nfoo\nbar\n",
            (4, 2),
            None,
        ),
        (
            "abc",
            "2ifoo<CR>bar<Esc>",
            "foo\nbarfoo\nbarabc\n",
            (2, 2),
            None,
        ),
        ("abc", "3ifoo<BS>x<Esc>", "foxfoxfoxabc\n", (0, 8), None),
        ("abc", "3i<Esc>", "abc\n", (0, 0), None),
        ("abc", "1ifoo<Esc>", "fooabc\n", (0, 2), None),
        (
            "abc\ndef",
            "3ifoo<Esc>j.",
            "foofoofooabc\ndefoofoofoof\n",
            (1, 10),
            None,
        ),
        (
            "abc\ndef",
            "3ifoo<Esc>j2.",
            "foofoofooabc\ndefoofoof\n",
            (1, 7),
            None,
        ),
        (
            "abc\ndef",
            "2ofoo<Esc>j.",
            "abc\nfoo\nfoo\ndef\nfoo\nfoo\n",
            (5, 2),
            None,
        ),
        (
            "abc\ndef",
            "ofoo<Esc>j3.",
            "abc\nfoo\ndef\nfoo\nfoo\nfoo\n",
            (5, 2),
            None,
        ),
        (
            "abc\ndef",
            "2afoo<Esc>j.",
            "afoofoobc\ndeffoofoo\n",
            (1, 8),
            None,
        ),
        (
            "abc\ndef",
            "2Afoo<Esc>j.",
            "abcfoofoo\ndeffoofoo\n",
            (1, 8),
            None,
        ),
        (
            "abc\ndef",
            "3Ifoo<Esc>j.",
            "foofoofooabc\nfoofoofoodef\n",
            (1, 8),
            None,
        ),
        ("abc", "3ifoo<C-o>0bar<Esc>", "barfooabc\n", (0, 2), None),
        ("abc", "3ifoo<Esc>x", "foofoofoabc\n", (0, 8), Some("o")),
        ("  abc", "2ofoo<Esc>", "  abc\n  foo\n  foo\n", (2, 4), None),
        ("  abc", "2Ofoo<Esc>", "  foo\n  foo\n  abc\n", (1, 4), None),
        (
            "abc\ndef\nghi",
            "ofoo<Esc>jj2.",
            "abc\nfoo\ndef\nghi\nfoo\nfoo\n",
            (5, 2),
            None,
        ),
        (
            "abc\ndef\nghi",
            "Ofoo<Esc>jj2.",
            "foo\nabc\nfoo\nfoo\ndef\nghi\n",
            (3, 2),
            None,
        ),
    ]);
}

#[test]
fn counted_text_objects() {
    check(&[
        (
            "foo bar baz qux",
            "d2aw",
            "baz qux\n",
            (0, 0),
            Some("foo bar "),
        ),
        (
            "foo bar baz qux",
            "d2iw",
            "bar baz qux\n",
            (0, 0),
            Some("foo "),
        ),
        (
            "foo bar baz qux",
            "d3iw",
            " baz qux\n",
            (0, 0),
            Some("foo bar"),
        ),
        (
            "foo bar baz qux",
            "d3aw",
            "qux\n",
            (0, 0),
            Some("foo bar baz "),
        ),
        (
            "foo.x bar.y baz",
            "d2aW",
            "baz\n",
            (0, 0),
            Some("foo.x bar.y "),
        ),
        (
            "foo.x bar.y baz",
            "d2iW",
            "bar.y baz\n",
            (0, 0),
            Some("foo.x "),
        ),
        (
            "foo bar baz qux",
            "c2awX<Esc>",
            "Xbaz qux\n",
            (0, 0),
            Some("foo bar "),
        ),
        (
            "foo bar baz qux",
            "c2iwX<Esc>",
            "Xbar baz qux\n",
            (0, 0),
            Some("foo "),
        ),
        (
            "foo bar baz qux",
            "y2aw",
            "foo bar baz qux\n",
            (0, 0),
            Some("foo bar "),
        ),
        (
            "foo bar baz qux",
            "y3iw",
            "foo bar baz qux\n",
            (0, 0),
            Some("foo bar"),
        ),
        (
            "foo bar baz qux",
            "2daw",
            "baz qux\n",
            (0, 0),
            Some("foo bar "),
        ),
        (
            "foo bar baz qux",
            "2d2aw",
            "\n",
            (0, 0),
            Some("foo bar baz qux"),
        ),
        (
            "foo bar baz qux",
            "wd2aw",
            "foo qux\n",
            (0, 4),
            Some("bar baz "),
        ),
        (
            "a\nb\n\nc\nd\n\ne",
            "d2ip",
            "c\nd\n\ne\n",
            (0, 0),
            Some("a\nb\n\n"),
        ),
        (
            "a\nb\n\nc\nd\n\ne",
            "d2ap",
            "e\n",
            (0, 0),
            Some("a\nb\n\nc\nd\n\n"),
        ),
        (
            "a\nb\n\nc\nd\n\ne",
            "d3ip",
            "\ne\n",
            (0, 0),
            Some("a\nb\n\nc\nd\n"),
        ),
        (
            "a\nb\n\nc\nd\n\ne",
            "y2ap",
            "a\nb\n\nc\nd\n\ne\n",
            (0, 0),
            Some("a\nb\n\nc\nd\n\n"),
        ),
        (
            "a\nb\n\nc\nd\n\ne",
            "c2ipX<Esc>",
            "X\nc\nd\n\ne\n",
            (0, 0),
            Some("a\nb\n\n"),
        ),
        (
            "f(a, (b, c), d)",
            "fcd2i(",
            "f()\n",
            (0, 2),
            Some("a, (b, c), d"),
        ),
        (
            "f(a, (b, c), d)",
            "fcd2a(",
            "f\n",
            (0, 0),
            Some("(a, (b, c), d)"),
        ),
        (
            "{ a { b } c }",
            "fbd2i{",
            "{}\n",
            (0, 1),
            Some(" a { b } c "),
        ),
        (
            "{ a { b } c }",
            "fbd2a{",
            "\n",
            (0, 0),
            Some("{ a { b } c }"),
        ),
        (
            "f(a, (b, c), d)",
            "fcc2i(X<Esc>",
            "f(X)\n",
            (0, 2),
            Some("a, (b, c), d"),
        ),
        (
            "f(a, (b, (c)), d)",
            "fcd3i(",
            "f()\n",
            (0, 2),
            Some("a, (b, (c)), d"),
        ),
        ("f(a)", "fad2i(", "f(a)\n", (0, 2), None),
        ("foo bar baz", "gU2iw", "FOO bar baz\n", (0, 0), None),
        ("foo bar baz", "gU2aw", "FOO BAR baz\n", (0, 0), None),
        (
            "a b c d e f g h",
            "d2aw.",
            "e f g h\n",
            (0, 0),
            Some("c d "),
        ),
        (
            "a b c d e f g h",
            "d2aw3.",
            "f g h\n",
            (0, 0),
            Some("c d e "),
        ),
        (
            "a b c d e f g h",
            "daw2.",
            "d e f g h\n",
            (0, 0),
            Some("b c "),
        ),
        (
            "a b c d e f g h",
            "ciwX<Esc>w3.",
            "X X d e f g h\n",
            (0, 2),
            Some("b c"),
        ),
        (
            "a b c d e f g h",
            "c2iwX<Esc>w.",
            "Xb Xd e f g h\n",
            (0, 3),
            Some("c "),
        ),
        (
            "a b c d e f g h",
            "cawX<Esc>2.",
            "Xd e f g h\n",
            (0, 0),
            Some("Xb c "),
        ),
        (
            "a\n\nb\n\nc\n\nd",
            "dap2.",
            "d\n",
            (0, 0),
            Some("b\n\nc\n\n"),
        ),
        (
            "a\nb\n\nc\nd\n\ne",
            "dipj2.",
            "\ne\n",
            (1, 0),
            Some("c\nd\n\n"),
        ),
        (
            "One. Two. Three. Four.",
            "d2as",
            "Three. Four.\n",
            (0, 0),
            Some("One. Two. "),
        ),
    ]);
}

#[test]
fn paragraph_objects_on_blank_lines_and_changes() {
    check(&[
        ("a\n\n\nb", "jdip", "a\nb\n", (1, 0), Some("\n\n")),
        ("a\n\nb", "jdip", "a\nb\n", (1, 0), Some("\n")),
        ("a\n\n\nb", "jyip", "a\n\n\nb\n", (1, 0), Some("\n\n")),
        ("a\n\n\nb", "jcipX<Esc>", "a\nX\nb\n", (1, 0), Some("\n\n")),
        ("a\n\n\nb", "jvipd", "a\nb\n", (1, 0), Some("\n\n")),
        ("a\nb\n\nc", "cipX<Esc>", "X\n\nc\n", (0, 0), Some("a\nb\n")),
        ("a\nb\n\nc", "capX<Esc>", "X\nc\n", (0, 0), Some("a\nb\n\n")),
        (
            "a\n\nb\nc",
            "Gcip X<Esc>",
            "a\n\n X\n",
            (2, 1),
            Some("b\nc\n"),
        ),
        (
            "a\nb\n\nc\nd",
            "cipX<Esc>jj.",
            "X\n\nX\n",
            (2, 0),
            Some("c\nd\n"),
        ),
    ]);
}
