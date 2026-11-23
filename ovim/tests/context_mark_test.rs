//! `''` and `` `` `` jump to the position before the latest jump (the start of the buffer before the
//! first one), even when the jump stayed on the line, and repeating them toggles between the two
//! places. A `G` or `gg` that does not move is not a jump. Every row was produced with
//! `nvim --clean --headless -s` (the keys typed from a script file): the buffer and the cursor as
//! (line, column) after the `x` that follows the jump back.

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
fn backtick_backtick_returns_to_the_position_before_the_latest_jump() {
    check(&[
        (
            "alpha beta gamma\nepsilon",
            "wwgg``x",
            "alpha beta amma\nepsilon\n",
            (0, 11),
        ),
        ("alpha beta gamma", "wGx``x", "lpha bta gamma\n", (0, 6)),
        ("(a b c) d", "f(l%``x", "( b c) d\n", (0, 1)),
        ("(a b c) d", "%``x", "a b c) d\n", (0, 0)),
        (
            "foo bar foo baz foo",
            "/foo<CR>n``x",
            "foo bar oo baz foo\n",
            (0, 8),
        ),
        ("foo bar foo baz", "/baz<CR>``x", "oo bar foo baz\n", (0, 0)),
        ("foo bar foo baz", "*``x", "oo bar foo baz\n", (0, 0)),
        ("a b c\n\nd", "wj{``x", "a b c\n\nd\n", (1, 0)),
        ("a. b. c.", ")``x", ". b. c.\n", (0, 0)),
        (
            "alpha beta gamma",
            "wmaw0`a``x",
            "lpha beta gamma\n",
            (0, 0),
        ),
        (
            "alpha beta gamma\nepsilon",
            "wwgg''x",
            "lpha beta gamma\nepsilon\n",
            (0, 0),
        ),
        ("alpha\nbeta gamma", "wG''x", "lpha\nbeta gamma\n", (0, 0)),
        (
            "alpha beta gamma\nepsilon",
            "wwgg````x",
            "lpha beta gamma\nepsilon\n",
            (0, 0),
        ),
        (
            "alpha\nbeta\ngamma",
            "jjgg''''x",
            "lpha\nbeta\ngamma\n",
            (0, 0),
        ),
        (
            "alpha beta gamma\nepsilon",
            "wwgg<C-o>x",
            "lpha beta gamma\nepsilon\n",
            (0, 0),
        ),
        (
            "alpha\nbeta\ngamma",
            "wGgg<C-o>x",
            "alpha\nbeta\namma\n",
            (2, 0),
        ),
        (
            "alpha\nbeta\ngamma",
            "Glgg``x",
            "alpha\nbeta\ngmma\n",
            (2, 1),
        ),
        ("alpha\nbeta\ngamma", "GH``x", "alpha\nbeta\namma\n", (2, 0)),
        ("alpha\nbeta\ngamma", "L``x", "lpha\nbeta\ngamma\n", (0, 0)),
        ("alpha\nbeta\ngamma", "M``x", "lpha\nbeta\ngamma\n", (0, 0)),
        ("alpha\nbeta\ngamma", "jj''x", "lpha\nbeta\ngamma\n", (0, 0)),
        (
            "alpha beta\nbeta\ngamma",
            "jjl``x",
            "lpha beta\nbeta\ngamma\n",
            (0, 0),
        ),
        (
            "alpha\nbeta\ngamma",
            "jGx''x''x",
            "alpha\neta\nmma\n",
            (2, 0),
        ),
        (
            "alpha\nbeta\ngamma",
            "Gggx''x''x''x",
            "pha\nbeta\nmma\n",
            (2, 0),
        ),
        ("alpha\nbeta\ngamma", "Gggdd''x", "beta\namma\n", (1, 0)),
        (
            "alpha\nbeta\ngamma",
            "jmaG'a''x",
            "alpha\nbeta\namma\n",
            (2, 0),
        ),
        (
            "alpha\nbeta\ngamma",
            "/gamma<CR>''x",
            "lpha\nbeta\ngamma\n",
            (0, 0),
        ),
        (
            "alpha\nbeta\ngamma",
            "/gamma<CR>''x''x",
            "lpha\nbeta\namma\n",
            (2, 0),
        ),
        (
            "alpha\nbeta\ngamma",
            "G''<C-o>x",
            "alpha\nbeta\namma\n",
            (2, 0),
        ),
        (
            "alpha\nbeta\ngamma",
            "Gggjj<C-o>x",
            "lpha\nbeta\ngamma\n",
            (0, 0),
        ),
        (
            "alpha\nbeta\nsome gamma",
            "jlG''x",
            "alpha\neta\nsome gamma\n",
            (1, 0),
        ),
        (
            "alpha beta\nbeta\ngamma",
            "$''x",
            "lpha beta\nbeta\ngamma\n",
            (0, 0),
        ),
        (
            "alpha beta\nbeta\ngamma",
            "w0''x",
            "lpha beta\nbeta\ngamma\n",
            (0, 0),
        ),
        ("alpha\nbeta", "jG''x", "lpha\nbeta\n", (0, 0)),
        ("alpha\nbeta", "jGx''x", "lpha\neta\n", (0, 0)),
        ("alpha\nbeta", "ggx''x", "pha\nbeta\n", (0, 0)),
        ("alpha\nbeta\ngamma", "jG''x", "alpha\neta\ngamma\n", (1, 0)),
        (
            "alpha\nbeta\ngamma",
            "jllG''x",
            "alpha\neta\ngamma\n",
            (1, 0),
        ),
        (
            "alpha beta gamma\nepsilon",
            "jwgg``x",
            "alpha beta gamma\nepsilo\n",
            (1, 5),
        ),
    ]);
}
