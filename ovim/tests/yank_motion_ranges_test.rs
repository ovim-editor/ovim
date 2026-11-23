//! Characterwise yanks (`yw`, `ye`, `yb`, `yl`, `y$`, ...) build their range in character
//! space: whole graphemes (flags, ZWJ emoji, combining marks) and line breaks inside
//! the range are kept. Every row was produced with `nvim --clean --headless`
//! (`normal! {keys}`): the buffer, the cursor as (line, column) and the unnamed register.

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
fn yanks_with_flags_and_emoji_use_whole_graphemes() {
    check(&[
        (
            "🇳🇴 flag foo bar",
            "wwye",
            "🇳🇴 flag foo bar\n",
            (0, 7),
            Some("foo"),
        ),
        (
            "🇳🇴 flag foo bar",
            "ye",
            "🇳🇴 flag foo bar\n",
            (0, 0),
            Some("🇳🇴 flag"),
        ),
        (
            "🇳🇴 flag foo bar",
            "wyw",
            "🇳🇴 flag foo bar\n",
            (0, 2),
            Some("flag "),
        ),
        (
            "🇳🇴 flag foo bar",
            "wwyb",
            "🇳🇴 flag foo bar\n",
            (0, 2),
            Some("flag "),
        ),
        (
            "🇳🇴 flag foo.x bar",
            "wwwyB",
            "🇳🇴 flag foo.x bar\n",
            (0, 7),
            Some("foo"),
        ),
        (
            "🇳🇴 flag foo.x bar",
            "wyE",
            "🇳🇴 flag foo.x bar\n",
            (0, 2),
            Some("flag"),
        ),
        (
            "🇳🇴 flag foo.x bar",
            "wyW",
            "🇳🇴 flag foo.x bar\n",
            (0, 2),
            Some("flag "),
        ),
        (
            "🇳🇴 flag foo bar",
            "yl",
            "🇳🇴 flag foo bar\n",
            (0, 0),
            Some("🇳🇴"),
        ),
        ("🇳🇴🇸🇪 flag", "y2l", "🇳🇴🇸🇪 flag\n", (0, 0), Some("🇳🇴🇸🇪")),
        (
            "🇳🇴 flag foo bar",
            "llyh",
            "🇳🇴 flag foo bar\n",
            (0, 1),
            Some(" "),
        ),
        (
            "🇳🇴 flag foo bar",
            "wyh",
            "🇳🇴 flag foo bar\n",
            (0, 1),
            Some(" "),
        ),
        (
            "🇳🇴 flag foo bar",
            "wwy0",
            "🇳🇴 flag foo bar\n",
            (0, 0),
            Some("🇳🇴 flag "),
        ),
        (
            "🇳🇴 flag foo bar",
            "wwy$",
            "🇳🇴 flag foo bar\n",
            (0, 7),
            Some("foo bar"),
        ),
        (
            "cafe\u{301} bar",
            "ye",
            "cafe\u{301} bar\n",
            (0, 0),
            Some("cafe\u{301}"),
        ),
        (
            "a 👨\u{200d}👩\u{200d}👧\u{200d}👦 b c",
            "wye",
            "a 👨\u{200d}👩\u{200d}👧\u{200d}👦 b c\n",
            (0, 2),
            Some("👨\u{200d}👩\u{200d}👧\u{200d}👦 b"),
        ),
        (
            "a 👨\u{200d}👩\u{200d}👧\u{200d}👦 b c",
            "wyw",
            "a 👨\u{200d}👩\u{200d}👧\u{200d}👦 b c\n",
            (0, 2),
            Some("👨\u{200d}👩\u{200d}👧\u{200d}👦 "),
        ),
        (
            "👨\u{200d}👩\u{200d}👧\u{200d}👦 b c",
            "yw",
            "👨\u{200d}👩\u{200d}👧\u{200d}👦 b c\n",
            (0, 0),
            Some("👨\u{200d}👩\u{200d}👧\u{200d}👦 "),
        ),
        (
            "a 👨\u{200d}👩\u{200d}👧\u{200d}👦 b c",
            "wly$",
            "a 👨\u{200d}👩\u{200d}👧\u{200d}👦 b c\n",
            (0, 3),
            Some(" b c"),
        ),
        (
            "a 👨\u{200d}👩\u{200d}👧\u{200d}👦 b c",
            "wy$",
            "a 👨\u{200d}👩\u{200d}👧\u{200d}👦 b c\n",
            (0, 2),
            Some("👨\u{200d}👩\u{200d}👧\u{200d}👦 b c"),
        ),
        (
            "a 👨\u{200d}👩\u{200d}👧\u{200d}👦 b c",
            "$y$",
            "a 👨\u{200d}👩\u{200d}👧\u{200d}👦 b c\n",
            (0, 6),
            Some("c"),
        ),
        (
            "a 👨\u{200d}👩\u{200d}👧\u{200d}👦 b",
            "wyl",
            "a 👨\u{200d}👩\u{200d}👧\u{200d}👦 b\n",
            (0, 2),
            Some("👨\u{200d}👩\u{200d}👧\u{200d}👦"),
        ),
        (
            "a 👨\u{200d}👩\u{200d}👧\u{200d}👦 b c",
            "wwy0",
            "a 👨\u{200d}👩\u{200d}👧\u{200d}👦 b c\n",
            (0, 0),
            Some("a 👨\u{200d}👩\u{200d}👧\u{200d}👦 "),
        ),
        (
            "  a 👨\u{200d}👩\u{200d}👧\u{200d}👦 b c",
            "wwy^",
            "  a 👨\u{200d}👩\u{200d}👧\u{200d}👦 b c\n",
            (0, 2),
            Some("a "),
        ),
    ]);
}

#[test]
fn yanks_across_lines_keep_the_line_breaks() {
    check(&[
        (
            "alpha beta\ngamma\ndelta\nepsilon",
            "wy3e",
            "alpha beta\ngamma\ndelta\nepsilon\n",
            (0, 6),
            Some("beta\ngamma\ndelta"),
        ),
        (
            "alpha beta\ngamma\ndelta epsilon",
            "y4e",
            "alpha beta\ngamma\ndelta epsilon\n",
            (0, 0),
            Some("alpha beta\ngamma\ndelta"),
        ),
        (
            "foo bar\nbaz qux",
            "wy2w",
            "foo bar\nbaz qux\n",
            (0, 4),
            Some("bar\nbaz "),
        ),
        (
            "foo bar\nbaz qux\nzed",
            "wy3w",
            "foo bar\nbaz qux\nzed\n",
            (0, 4),
            Some("bar\nbaz qux"),
        ),
        (
            "foo bar\nbaz qux",
            "jyb",
            "foo bar\nbaz qux\n",
            (0, 4),
            Some("bar"),
        ),
        (
            "foo bar\nbaz qux",
            "jy2b",
            "foo bar\nbaz qux\n",
            (0, 0),
            Some("foo bar\n"),
        ),
        (
            "foo.x bar\nbaz.y qux",
            "jyB",
            "foo.x bar\nbaz.y qux\n",
            (0, 6),
            Some("bar"),
        ),
        (
            "foo.x bar\nbaz.y qux",
            "wy2E",
            "foo.x bar\nbaz.y qux\n",
            (0, 3),
            Some(".x bar"),
        ),
        (
            "foo.x bar\nbaz.y qux",
            "wy2W",
            "foo.x bar\nbaz.y qux\n",
            (0, 3),
            Some(".x bar"),
        ),
    ]);
}

#[test]
fn yw_stops_at_the_end_of_the_last_word_on_a_line() {
    check(&[
        (
            "foo bar\nbaz",
            "$byw",
            "foo bar\nbaz\n",
            (0, 4),
            Some("bar"),
        ),
        ("foo bar\nbaz", "wyw", "foo bar\nbaz\n", (0, 4), Some("bar")),
        ("foo bar", "wyw", "foo bar\n", (0, 4), Some("bar")),
        ("foo\n\nbar", "wyw", "foo\n\nbar\n", (1, 0), Some("\n")),
        ("foo\n\nbar", "y2w", "foo\n\nbar\n", (0, 0), Some("foo\n\n")),
    ]);
}

#[test]
fn simple_yanks_are_unchanged() {
    check(&[
        ("foo bar", "wy$", "foo bar\n", (0, 4), Some("bar")),
        ("foo\n\nbar", "jy$", "foo\n\nbar\n", (1, 0), None),
        ("foo", "$yl", "foo\n", (0, 2), Some("o")),
        ("foo", "ly5l", "foo\n", (0, 1), Some("oo")),
        ("foo bar", "y5l", "foo bar\n", (0, 0), Some("foo b")),
        ("foo", "yh", "foo\n", (0, 0), None),
    ]);
}
