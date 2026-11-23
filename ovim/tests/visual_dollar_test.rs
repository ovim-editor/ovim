//! Characterwise `v$` puts the cursor on the line break, so the selection includes it
//! (`v$d` joins the next line, `v$y` yanks the newline) except on the last line, which has none;
//! `v` on an empty line selects its line break. Every row was produced with
//! `nvim --clean --headless` (`normal! {keys}`): the buffer and the cursor as (line, column).

mod helpers;
use helpers::EditorTest;
use ovim::mode::Mode;

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

/// nvim: after `v$` the cursor is on the line break (column 3 of `abc`) and `l` cannot move it
/// back onto the last character.
#[test]
fn l_on_the_line_break_stays_there() {
    let mut test = EditorTest::new("abc\ndef\n");
    test.keys("v$");
    assert_eq!(test.cursor(), (0, 3));
    test.keys("l");
    assert_eq!(test.cursor(), (0, 3));
    assert_eq!(test.mode(), Mode::Visual);
}

#[test]
fn v_dollar_selects_the_line_break() {
    check(&[
        ("abc\ndef\nghi", "v$d", "def\nghi\n", (0, 0)),
        ("abc\ndef\nghi", "v$y", "abc\ndef\nghi\n", (0, 0)),
        ("abc\ndef\nghi", "v$cX<Esc>", "Xdef\nghi\n", (0, 0)),
        ("abc\ndef\nghi", "lv$d", "adef\nghi\n", (0, 1)),
        ("abc\ndef\nghi", "v$jd", "ghi\n", (0, 0)),
        ("abc\ndef\nghi", "v$jy", "abc\ndef\nghi\n", (0, 0)),
        ("abc\n\nghi", "2Gv$d", "abc\nghi\n", (1, 0)),
        ("abc\ndef\nghi", "yyjv$p", "abc\n\nabc\nghi\n", (2, 0)),
        ("abc\ndef\nghi", "v$x", "def\nghi\n", (0, 0)),
        ("abc\ndef\nghi", "v$<Esc>`>", "abc\ndef\nghi\n", (0, 2)),
        ("abc\ndef\nghi", "v$hd", "\ndef\nghi\n", (0, 0)),
        ("abc\ndef\nghi", "v2$d", "ghi\n", (0, 0)),
        ("abc def\nghi", "wv$y", "abc def\nghi\n", (0, 4)),
        ("abc\ndef\nghi", "v$j$d", "ghi\n", (0, 0)),
        ("abc\ndef\nghi", "2Gv$kd", "abcef\nghi\n", (0, 3)),
        ("abc\ndef\nghi", "v$<Esc>gvd", "def\nghi\n", (0, 0)),
        ("abc\ndef\nghi", "v$<Esc>", "abc\ndef\nghi\n", (0, 2)),
        ("abc\ndef\nghi", "vj$y", "abc\ndef\nghi\n", (0, 0)),
        ("abc\ndef\nghi", "vj$d", "ghi\n", (0, 0)),
        ("abc\ndef", "v5$y", "abc\ndef\n", (0, 0)),
        ("abc\ndef", "v5$d", "\n", (0, 0)),
        ("aéb\ndef", "v$y", "aéb\ndef\n", (0, 0)),
        ("abc\ndef\nghi", "v$<Esc>gvy", "abc\ndef\nghi\n", (0, 0)),
        ("abc\ndef\nghi", "v$<Esc>2Ggvd", "def\nghi\n", (0, 0)),
    ]);
}

#[test]
fn v_dollar_on_the_last_line_has_no_line_break() {
    check(&[
        ("abc\ndef\nghi", "3Gv$d", "abc\ndef\n\n", (2, 0)),
        ("abc\ndef\nghi", "3Gv$y", "abc\ndef\nghi\n", (2, 0)),
        ("ab\n", "Gvd", "ab\n\n", (1, 0)),
        ("ab\n", "Gvy", "ab\n\n", (1, 0)),
        ("abc\ndef", "Gv$y", "abc\ndef\n", (1, 0)),
        ("abc\ndef", "Gv$hhd", "abc\nf\n", (1, 0)),
    ]);
}

#[test]
fn v_on_an_empty_line_selects_its_line_break() {
    check(&[
        ("ab\n\ncd", "2Gvd", "ab\ncd\n", (1, 0)),
        ("ab\n\ncd", "2Gvy", "ab\n\ncd\n", (1, 0)),
        ("\nab", "vd", "ab\n", (0, 0)),
    ]);
}

#[test]
fn other_v_dollar_forms_are_unchanged() {
    check(&[
        ("abc\ndef", "v$~", "ABC\ndef\n", (0, 0)),
        ("abc\ndef", "v$U", "ABC\ndef\n", (0, 0)),
        ("abc\ndef\nghi", "v$>", "    abc\ndef\nghi\n", (0, 4)),
        ("abc\ndef\nghi", "v$rx", "xxx\ndef\nghi\n", (0, 0)),
        ("abc\ndef\nghi", "V$d", "def\nghi\n", (0, 0)),
    ]);
}
