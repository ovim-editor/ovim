//! Operators followed by a motion that has no hand-written arm, and keys that are
//! not motions at all. Every row was produced with `nvim --clean --headless`
//! (`set startofline shiftwidth=4 expandtab`, then `normal! {keys}`): the buffer,
//! the cursor as (line, column) and the unnamed register afterwards.

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
fn unknown_keys_cancel_the_operator_and_are_swallowed() {
    check(&[
        ("hello world", "wdx", "hello world\n", (0, 6), None),
        ("hello world", "wdQ", "hello world\n", (0, 6), None),
        ("hello world", "wd<Esc>x", "hello orld\n", (0, 6), Some("w")),
    ]);
}

#[test]
fn sentence_motions() {
    check(&[
        (
            "One two. Three four. Five six.",
            "wd)",
            "One Three four. Five six.\n",
            (0, 4),
            Some("two. "),
        ),
        (
            "One two. Three four. Five six.",
            "d2)",
            "Five six.\n",
            (0, 0),
            Some("One two. Three four. "),
        ),
        (
            "One two. Three four. Five six.",
            "$d(",
            "One two. Three four. .\n",
            (0, 21),
            Some("Five six"),
        ),
        (
            "One two. Three four. Five six.",
            "$d2(",
            "One two. .\n",
            (0, 9),
            Some("Three four. Five six"),
        ),
        (
            "One two. Three four. Five six.",
            "wc)X<Esc>",
            "One XThree four. Five six.\n",
            (0, 4),
            Some("two. "),
        ),
        (
            "One two. Three four. Five six.",
            "wy)",
            "One two. Three four. Five six.\n",
            (0, 4),
            Some("two. "),
        ),
        (
            "One two. Three four. Five six.",
            "$y(",
            "One two. Three four. Five six.\n",
            (0, 21),
            Some("Five six"),
        ),
        (
            "one two. three four. five six.",
            "wgU)",
            "one TWO. three four. five six.\n",
            (0, 4),
            None,
        ),
        (
            "one two. three four. five six.",
            "$gU(",
            "one two. three four. FIVE SIX.\n",
            (0, 21),
            None,
        ),
        (
            "One two.\nThree four.\n\nFive six.",
            "d)",
            "Three four.\n\nFive six.\n",
            (0, 0),
            Some("One two.\n"),
        ),
        (
            "One two.\nThree four.\n\nFive six.",
            "Gd(",
            "One two.\nThree four.\nFive six.\n",
            (2, 0),
            Some("\n"),
        ),
        (
            "One. Two. Three.",
            "d)",
            "Two. Three.\n",
            (0, 0),
            Some("One. "),
        ),
        (
            "One. Two. Three. Four.",
            "3d)",
            "Four.\n",
            (0, 0),
            Some("One. Two. Three. "),
        ),
        (
            "One. Two. Three. Four.",
            "d2)",
            "Three. Four.\n",
            (0, 0),
            Some("One. Two. "),
        ),
    ]);
}

#[test]
fn search_repeat_motions() {
    check(&[
        (
            "foo bar foo baz foo end",
            "/foo<CR>0dn",
            "foo baz foo end\n",
            (0, 0),
            Some("foo bar "),
        ),
        (
            "foo bar foo baz foo end",
            "/foo<CR>0d2n",
            "foo end\n",
            (0, 0),
            Some("foo bar foo baz "),
        ),
        (
            "foo bar foo baz foo end",
            "$?foo<CR>dN",
            "foo end\n",
            (0, 0),
            Some("foo bar foo baz "),
        ),
        (
            "foo bar foo baz foo end",
            "/foo<CR>dN",
            "foo baz foo end\n",
            (0, 0),
            Some("foo bar "),
        ),
        (
            "foo bar foo baz foo end",
            "/foo<CR>0cnX<Esc>",
            "Xfoo baz foo end\n",
            (0, 0),
            Some("foo bar "),
        ),
        (
            "foo bar foo baz foo end",
            "/foo<CR>0yn",
            "foo bar foo baz foo end\n",
            (0, 0),
            Some("foo bar "),
        ),
        (
            "foo bar foo baz foo end",
            "/foo<CR>0gUn",
            "FOO BAR foo baz foo end\n",
            (0, 0),
            None,
        ),
    ]);
}

#[test]
fn find_repeat_motions() {
    check(&[
        ("a b c d e f g", "fcd;", "a b c d e f g\n", (0, 4), None),
        ("axbxcxdxe", "fxd;", "acxdxe\n", (0, 1), Some("xbx")),
        ("axbxcxdxe", "$Fxd,", "axbxcxdxe\n", (0, 7), None),
        ("axbxcxdxe", "tx;d;", "axxdxe\n", (0, 2), Some("bxc")),
        ("axbxcxdxe", "tx;d,", "axbxcxdxe\n", (0, 2), None),
        ("axbxcxdxe", "fxy;", "axbxcxdxe\n", (0, 1), Some("xbx")),
        ("axbxcxdxe", "fxc;Q<Esc>", "aQcxdxe\n", (0, 1), Some("xbx")),
    ]);
}

#[test]
fn underscore_motion() {
    check(&[
        (
            "one\ntwo\nthree",
            "jd_",
            "one\nthree\n",
            (1, 0),
            Some("two\n"),
        ),
        (
            "one\ntwo\nthree\nfour",
            "d2_",
            "three\nfour\n",
            (0, 0),
            Some("one\ntwo\n"),
        ),
        (
            "  one\n  two\nthree",
            "jc_X<Esc>",
            "  one\n  X\nthree\n",
            (1, 2),
            Some("  two\n"),
        ),
        (
            "one\ntwo\nthree",
            "jy_",
            "one\ntwo\nthree\n",
            (1, 0),
            Some("two\n"),
        ),
        (
            "one\ntwo\nthree",
            "y2_",
            "one\ntwo\nthree\n",
            (0, 0),
            Some("one\ntwo\n"),
        ),
        ("one\ntwo\nthree", "jgU_", "one\nTWO\nthree\n", (1, 0), None),
        (
            "one\ntwo\nthree",
            "j>_",
            "one\n    two\nthree\n",
            (1, 4),
            None,
        ),
    ]);
}

#[test]
fn word_end_backward_motions() {
    check(&[
        ("foo bar baz", "wwdge", "foo baaz\n", (0, 6), Some("r b")),
        ("foo bar baz", "$d2ge", "fo\n", (0, 1), Some("o bar baz")),
        ("foo.bar baz.q", "$dgE", "foo.ba\n", (0, 5), Some("r baz.q")),
        (
            "foo bar baz",
            "wwcgeX<Esc>",
            "foo baXaz\n",
            (0, 6),
            Some("r b"),
        ),
        ("foo bar baz", "wwyge", "foo bar baz\n", (0, 6), Some("r b")),
        ("foo bar baz", "wwgUge", "foo baR Baz\n", (0, 6), None),
        ("foo\nbar", "jdge", "foar\n", (0, 2), Some("o\nb")),
    ]);
}

#[test]
fn search_as_motion() {
    check(&[
        (
            "foo bar baz",
            "d/baz<CR>",
            "baz\n",
            (0, 0),
            Some("foo bar "),
        ),
        (
            "foo bar baz foo",
            "2d/foo<CR>",
            "foo bar baz foo\n",
            (0, 0),
            None,
        ),
        (
            "foo\nbar\nbaz",
            "d/baz<CR>",
            "baz\n",
            (0, 0),
            Some("foo\nbar\n"),
        ),
        (
            "foo bar\nbaz qux",
            "wd/qux<CR>",
            "foo qux\n",
            (0, 4),
            Some("bar\nbaz "),
        ),
        (
            "foo\nbar\nbaz",
            "wd/baz<CR>",
            "foo\nbaz\n",
            (1, 0),
            Some("bar\n"),
        ),
        (
            "foo bar baz",
            "$d?bar<CR>",
            "foo z\n",
            (0, 4),
            Some("bar ba"),
        ),
        (
            "foo bar baz",
            "$d?foo<CR>",
            "z\n",
            (0, 0),
            Some("foo bar ba"),
        ),
        (
            "foo bar baz",
            "c/baz<CR>X<Esc>",
            "Xbaz\n",
            (0, 0),
            Some("foo bar "),
        ),
        (
            "foo bar baz",
            "y/baz<CR>",
            "foo bar baz\n",
            (0, 0),
            Some("foo bar "),
        ),
        (
            "foo bar baz",
            "$y?bar<CR>",
            "foo bar baz\n",
            (0, 4),
            Some("bar ba"),
        ),
        ("foo bar baz", "gU/baz<CR>", "FOO BAR baz\n", (0, 0), None),
        ("a\nb\nc", ">/c<CR>", "    a\n    b\nc\n", (0, 4), None),
        ("foo bar baz", "wd/bar<CR>", "foo bar baz\n", (0, 4), None),
        (
            "foo bar baz",
            "$d/foo<CR>",
            "z\n",
            (0, 0),
            Some("foo bar ba"),
        ),
    ]);
}

#[test]
fn case_and_indent_operators_take_any_motion() {
    check(&[
        ("foo\nbar\nbaz", "gUj", "FOO\nBAR\nbaz\n", (0, 0), None),
        ("foo\nbar\nbaz", "jgUk", "FOO\nBAR\nbaz\n", (0, 0), None),
        ("foo bar baz", "$gUb", "foo bar BAz\n", (0, 8), None),
        (
            "foo.x bar.y baz",
            "$gUB",
            "foo.x bar.y BAz\n",
            (0, 12),
            None,
        ),
        ("foo bar baz", "gUW", "FOO bar baz\n", (0, 0), None),
        ("foo bar baz", "gUE", "FOO bar baz\n", (0, 0), None),
        ("foo\n\nbar baz", "jjgU{", "foo\n\nbar baz\n", (1, 0), None),
        ("foo bar\n\nbaz", "gU}", "FOO BAR\n\nbaz\n", (0, 0), None),
        ("foo bar", "$gUh", "foo bAr\n", (0, 5), None),
        ("foo bar", "gUl", "Foo bar\n", (0, 0), None),
        ("foo bar", "$gU0", "FOO BAr\n", (0, 0), None),
        ("  foo bar", "$gU^", "  FOO BAr\n", (0, 2), None),
        ("(foo bar) baz", "gU%", "(FOO BAR) baz\n", (0, 0), None),
        ("foo\nbar\nbaz", "jgUG", "foo\nBAR\nBAZ\n", (1, 0), None),
        ("foo\nbar\nbaz", "jgUgg", "FOO\nBAR\nbaz\n", (0, 0), None),
        ("foo bar", "gUiw", "FOO bar\n", (0, 0), None),
        ("a\nb\n\nc", ">}", "    a\n    b\n\nc\n", (0, 4), None),
        ("a\n\nb\nc", "G>{", "a\n\n    b\nc\n", (1, 0), None),
        ("  a\n  b\n\nc", "<}", "a\nb\n\nc\n", (0, 0), None),
        ("a\nb\nc", ">j", "    a\n    b\nc\n", (0, 4), None),
    ]);
}

#[test]
fn indent_operators_take_text_objects() {
    check(&[
        ("a\nb\n\nc", ">ip", "    a\n    b\n\nc\n", (0, 4), None),
        ("a\nb\n\nc", ">ap", "    a\n    b\n\nc\n", (0, 4), None),
        ("  a\n  b\n\nc", "<ip", "a\nb\n\nc\n", (0, 0), None),
        ("a\n  b\n\nc", "=ip", "a\nb\n\nc\n", (0, 0), None),
        (
            "f {\na\nb\n}",
            "jj>i{",
            "f {\n    a\n    b\n}\n",
            (1, 4),
            None,
        ),
        (
            "f {\na\nb\n}",
            "jj>a{",
            "    f {\n    a\n    b\n    }\n",
            (0, 4),
            None,
        ),
        (
            "f {\n    a\n    b\n}",
            "jj<i{",
            "f {\na\nb\n}\n",
            (1, 0),
            None,
        ),
        ("f (\na\n)", "j>ib", "f (\n    a\n)\n", (1, 4), None),
        ("a \"b\" c", "f\">i\"", "    a \"b\" c\n", (0, 4), None),
        ("a b", ">iw", "    a b\n", (0, 4), None),
        ("a\nb\n\nc", "j>ip", "    a\n    b\n\nc\n", (0, 4), None),
    ]);
}

#[test]
fn line_and_column_motions() {
    check(&[
        ("a\nb\nc", "d+", "c\n", (0, 0), Some("a\nb\n")),
        ("a\nb\nc", "jd-", "c\n", (0, 0), Some("a\nb\n")),
        ("abcdef", "$d|", "f\n", (0, 0), Some("abcde")),
        ("abcdef", "$d3|", "abf\n", (0, 2), Some("cde")),
        ("abc", "d<Space>", "bc\n", (0, 0), Some("a")),
        ("abc", "ld<BS>", "bc\n", (0, 0), Some("a")),
    ]);
}

#[test]
fn char_find_and_mark_motions_with_any_operator() {
    check(&[
        ("foo bar baz", "gUfa", "FOO BAr baz\n", (0, 0), None),
        ("foo bar baz", "gUta", "FOO Bar baz\n", (0, 0), None),
        ("foo bar baz", "$gUFf", "FOO BAR BAz\n", (0, 0), None),
        ("foo bar baz", "$gUTf", "fOO BAR BAz\n", (0, 1), None),
        ("a\nb a\nc", ">fa", "a\nb a\nc\n", (0, 0), None),
        ("foo\nbar\nbaz", ">fz", "foo\nbar\nbaz\n", (0, 0), None),
        ("a\nb\nc\nd", "jmajjgU'a", "a\nB\nC\nD\n", (1, 0), None),
        (
            "a\nb\nc\nd",
            "jmagg>'a",
            "    a\n    b\nc\nd\n",
            (0, 4),
            None,
        ),
        (
            "  a\n  b\n  c\n  d",
            "jmajj<'a",
            "  a\nb\nc\nd\n",
            (1, 0),
            None,
        ),
        (
            "foo bar\nbaz qux",
            "jwmakgU`a",
            "foo BAR\nBAZ qux\n",
            (0, 4),
            None,
        ),
        (
            "one two three\nfour",
            "wmawwd`a",
            "one \nfour\n",
            (0, 3),
            Some("two three"),
        ),
        (
            "a\nb\nc\nd\ne",
            "jmajjd'a",
            "a\ne\n",
            (1, 0),
            Some("b\nc\nd\n"),
        ),
        (
            "a\nb\nc\nd\ne",
            "jmajjy'a",
            "a\nb\nc\nd\ne\n",
            (1, 0),
            Some("b\nc\nd\n"),
        ),
        (
            "foo bar baz",
            "wmaec`aX<Esc>",
            "foo Xr baz\n",
            (0, 4),
            Some("ba"),
        ),
    ]);
}

#[test]
fn escape_and_failed_searches_cancel_the_operator() {
    // nvim --clean (typed keys): `d/ba<Esc>x` -> Esc abandons the search *and* the
    // operator, then `x` deletes one character.
    let mut test = EditorTest::new("foo bar\n");
    test.keys("d/ba<Esc>x");
    assert_eq!(test.buffer_content(), "oo bar\n");

    // nvim --clean (typed keys): a pattern that is not found deletes nothing,
    // and the command after it runs normally.
    let mut test = EditorTest::new("foo bar baz\n");
    test.keys("d/zzz<CR>x");
    assert_eq!(test.buffer_content(), "oo bar baz\n");
}

#[test]
fn operator_search_motion_undoes_in_one_step() {
    // nvim --clean (typed keys): `wd/baz<CR>u` restores the text.
    let mut test = EditorTest::new("foo bar baz\n");
    test.keys("wd/baz<CR>");
    assert_eq!(test.buffer_content(), "foo baz\n");
    test.keys("u");
    assert_eq!(test.buffer_content(), "foo bar baz\n");
}
