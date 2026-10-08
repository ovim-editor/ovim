//! `ciw` on an empty line starts inserting there (it used to fail, so the typed text ran as
//! commands), `.` repeats it, and `diw` / `yiw` / `viw` there select nothing. Every row was
//! produced with `nvim --clean --headless` (`normal! {keys}`): the buffer and the cursor as
//! (line, column).

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
fn ciw_on_an_empty_line_starts_inserting_there() {
    check(&[
        ("a\n\nb", "jciwfoo<Esc>", "a\nfoo\nb\n", (1, 2)),
        ("\nb", "ciwfoo<Esc>", "foo\nb\n", (0, 2)),
        ("a\n\nb", "jciWfoo<Esc>", "a\nfoo\nb\n", (1, 2)),
        ("a\n   \nb", "jciwfoo<Esc>", "a\nfoo\nb\n", (1, 2)),
        ("a\n   \nb", "j$ciwfoo<Esc>", "a\nfoo\nb\n", (1, 2)),
        (
            "a\n\nb\n\nc",
            "jciwfoo<Esc>jj.",
            "a\nfoo\nb\nfoo\nc\n",
            (3, 2),
        ),
    ]);
}

#[test]
fn diw_yiw_and_viw_on_an_empty_line_select_nothing() {
    check(&[
        ("a\n\nb", "jdiw", "a\n\nb\n", (1, 0)),
        ("a\n\nb", "jyiw", "a\n\nb\n", (1, 0)),
        ("a\n\nb", "jviwd", "a\nb\n", (1, 0)),
        ("a\n   \nb", "jdiw", "a\n\nb\n", (1, 0)),
    ]);
}
