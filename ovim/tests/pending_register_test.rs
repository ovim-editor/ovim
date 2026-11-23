//! A register typed with `"x` belongs to the command that follows it: Esc, a motion or a
//! cancelled operator must not leave it behind for the next command, and Ctrl chords
//! are not plain-letter commands. Every row was produced with `nvim --clean --headless`
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
fn esc_motion_and_cancelled_operator_drop_the_register() {
    check(&[
        (
            "one two three\nfour five six",
            "\"cylwyiwj0\"c<Esc>p",
            "one two three\nftwoour five six\n",
            (1, 3),
            Some("two"),
        ),
        (
            "one two three",
            "yw\"c<Esc>$p",
            "one two threeone \n",
            (0, 16),
            Some("one "),
        ),
        (
            "one two three",
            "yw\"cw$p",
            "one two threeone \n",
            (0, 16),
            Some("one "),
        ),
        (
            "one two three",
            "yiw\"cwP",
            "one onetwo three\n",
            (0, 6),
            Some("one"),
        ),
        (
            "one two three",
            "yw\"cdzp",
            "one two three\n",
            (0, 0),
            Some("one "),
        ),
        (
            "one two three",
            "yw\"cdxp",
            "one two three\n",
            (0, 0),
            Some("one "),
        ),
        (
            "one two three",
            "yw\"c<C-c>p",
            "oone ne two three\n",
            (0, 4),
            Some("one "),
        ),
        (
            "a\nb\nc",
            "\"ayyj\"ak\"ap",
            "a\na\nb\nc\n",
            (1, 0),
            Some("a\n"),
        ),
    ]);
}

#[test]
fn register_survives_counts_and_is_replaced_by_the_latest() {
    check(&[
        (
            "one two three four",
            "\"cyiww\"c2dwP",
            "one two three four\n",
            (0, 13),
            Some("two three "),
        ),
        (
            "one two three four",
            "\"cyiww2\"cdw\"cP",
            "one two three four\n",
            (0, 13),
            Some("two three "),
        ),
        (
            "one two three",
            "\"ayiww\"b\"cywP",
            "one two two three\n",
            (0, 7),
            Some("two "),
        ),
        (
            "one two three",
            "\"ayiww\"b\"cyw$\"bp",
            "one two three\n",
            (0, 12),
            Some("two "),
        ),
        (
            "one two three",
            "\"ayiwwdiw\"ap",
            "one  onethree\n",
            (0, 7),
            Some("two"),
        ),
        (
            "one two three",
            "\"ayiw\"_dwP",
            "onetwo three\n",
            (0, 2),
            Some("one"),
        ),
        (
            "one two",
            "\"ayiwA!<Esc>\"ap",
            "one two!one\n",
            (0, 10),
            Some("one"),
        ),
        (
            "one two three",
            "\"ayiwdwu\"-p",
            "oone ne two three\n",
            (0, 4),
            Some("one "),
        ),
        (
            "one two three",
            "\"ayiwwviw\"ap",
            "one one three\n",
            (0, 6),
            Some("two"),
        ),
        (
            "one two three",
            "\"ayiwwv<Esc>p",
            "one tonewo three\n",
            (0, 7),
            Some("one"),
        ),
        (
            "one two three",
            "\"ayiwmawd'a\"ap",
            "one\n",
            (0, 2),
            Some("one two three\n"),
        ),
    ]);
}

#[test]
fn ctrl_chords_in_normal_mode_are_not_commands() {
    check(&[
        (
            "one two three",
            "<C-c>x",
            "ne two three\n",
            (0, 0),
            Some("o"),
        ),
        (
            "one two three",
            "3<C-c>x",
            "ne two three\n",
            (0, 0),
            Some("o"),
        ),
        (
            "one two three",
            "d<C-c>x",
            "ne two three\n",
            (0, 0),
            Some("o"),
        ),
        (
            "one two three",
            "<C-g>x",
            "ne two three\n",
            (0, 0),
            Some("o"),
        ),
        (
            "one two three",
            "<C-l>x",
            "ne two three\n",
            (0, 0),
            Some("o"),
        ),
    ]);
}
