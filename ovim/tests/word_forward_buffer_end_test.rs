//! `w` and `W` on the last word of the buffer go to its last character (and `d5w`, `cw` there take
//! the word); lines of blanks after the last word count as part of the buffer. Every row was
//! produced with `nvim --clean --headless -s` (the keys typed from a script file): the buffer and
//! the cursor as (line, column).

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
fn w_on_the_last_word_goes_to_the_last_character() {
    check(&[
        ("alpha\nepsilon", "jw", "alpha\nepsilon\n", (1, 6)),
        ("alpha\nepsilon", "jwx", "alpha\nepsilo\n", (1, 5)),
        (
            "alpha\nepsilon zeta",
            "jwwwx",
            "alpha\nepsilon zet\n",
            (1, 10),
        ),
        ("alpha\nepsilon", "jW", "alpha\nepsilon\n", (1, 6)),
        ("alpha beta", "$w", "alpha beta\n", (0, 9)),
        ("alpha beta", "wwx", "alpha bet\n", (0, 8)),
        ("alpha\nepsilon", "j5w", "alpha\nepsilon\n", (1, 6)),
        ("alpha\nepsilon", "jd5w", "alpha\n\n", (1, 0)),
        ("alpha beta\n   \n  ", "w", "alpha beta\n   \n  \n", (0, 6)),
        ("alpha beta\n   \n  ", "wwx", "alpha beta\n   \n \n", (2, 0)),
        ("alpha beta\n   \n  ", "$w", "alpha beta\n   \n  \n", (2, 1)),
        (
            "alpha-beta gamma\n   ",
            "wW",
            "alpha-beta gamma\n   \n",
            (0, 11),
        ),
        ("alpha\nb", "jw", "alpha\nb\n", (1, 0)),
        ("alpha\n", "jw", "alpha\n\n", (1, 0)),
        ("alpha\n\n", "w", "alpha\n\n\n", (1, 0)),
        ("a b c", "10w", "a b c\n", (0, 4)),
        ("alpha beta", "wvwd", "alpha \n", (0, 5)),
        ("alpha beta\nepsilon", "jvwd", "alpha beta\n\n", (1, 0)),
        ("alpha\nepsilon", "jdw", "alpha\n\n", (1, 0)),
        ("alpha\nepsilon", "jcwX<Esc>", "alpha\nX\n", (1, 0)),
        ("alpha beta", "wwwx", "alpha bet\n", (0, 8)),
    ]);
}
