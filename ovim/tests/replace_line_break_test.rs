//! `r<CR>` replaces `[count]` characters with a single line break and leaves the cursor at the
//! start of the new line. Every row was produced with `nvim --clean --headless -s` (the keys typed
//! from a script file): the buffer and the cursor as (line, column).

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
fn r_enter_replaces_the_characters_with_one_line_break() {
    check(&[
        (
            "alpha beta gamma\nepsilon",
            "wr<CR>",
            "alpha \neta gamma\nepsilon\n",
            (1, 0),
        ),
        (
            "alpha beta gamma\nepsilon",
            "w3r<CR>",
            "alpha \na gamma\nepsilon\n",
            (1, 0),
        ),
        (
            "alpha beta gamma\nepsilon",
            "$r<CR>",
            "alpha beta gamm\n\nepsilon\n",
            (1, 0),
        ),
        (
            "alpha beta gamma\nepsilon",
            "r<CR>",
            "\nlpha beta gamma\nepsilon\n",
            (1, 0),
        ),
        (
            "alpha beta\nepsilon",
            "w4r<CR>",
            "alpha \n\nepsilon\n",
            (1, 0),
        ),
        (
            "alpha beta gamma\nepsilon",
            "wr<CR>u",
            "alpha beta gamma\nepsilon\n",
            (0, 6),
        ),
        (
            "alpha beta gamma\nepsilon",
            "wr<CR>x",
            "alpha \nta gamma\nepsilon\n",
            (1, 0),
        ),
        ("aé🇳🇴b c", "lr<CR>", "a\n🇳🇴b c\n", (1, 0)),
    ]);
}

#[test]
fn r_enter_fails_on_an_empty_line_or_with_too_big_a_count() {
    check(&[
        ("alpha\n\nepsilon", "jr<CR>", "alpha\n\nepsilon\n", (1, 0)),
        (
            "alpha beta\nepsilon",
            "w9r<CR>",
            "alpha beta\nepsilon\n",
            (0, 6),
        ),
    ]);
}

#[test]
fn r_enter_repeats_with_dot() {
    check(&[
        (
            "alpha beta gamma\nepsilon",
            "wr<CR>j0.",
            "alpha \neta gamma\n\npsilon\n",
            (3, 0),
        ),
        (
            "alpha beta gamma\nepsilon zeta",
            "w2r<CR>j0.",
            "alpha \nta gamma\n\nsilon zeta\n",
            (3, 0),
        ),
    ]);
}
