//! A macro register is a register: `qa...q` leaves the typed keys in `"a` (`qA` appends), `"ap`
//! pastes them, and writing to the register (`"ayy`) changes what `@a` executes. Every row was
//! produced with `nvim --clean --headless -s` (the keys typed from a script file, which is what
//! makes `q` record them): the buffer and the cursor as (line, column).

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
fn recording_a_macro_puts_the_keys_in_its_register() {
    check(&[
        ("abc\ndef", "qaxq\"ap", "bxc\ndef\n", (0, 1)),
        ("abc\ndef", "qaxxq\"aP", "xxc\ndef\n", (0, 1)),
        ("abc\ndef", "qaA!<Esc>q\"ap", "abc!A!\x1b\ndef\n", (0, 6)),
        ("abc\ndef", "qaxqqAxq\"ap", "cxx\ndef\n", (0, 2)),
        ("abc\ndef", "qaxq\"ayyP", "bc\nbc\ndef\n", (0, 0)),
        ("1\n1\n1", "qa<C-a>q\"ap", "2\n1\n1\n", (0, 1)),
        (
            "abc",
            "qaihello <Esc>q\"ap",
            "hello ihello \x1babc\n",
            (0, 13),
        ),
        ("abc", "qaxqqaq\"ap", "bc\n", (0, 0)),
        ("abc", "qaxqqaiQ<Esc>q\"ap", "QiQ\x1bbc\n", (0, 3)),
        ("abc\ndef", "qbxq\"bp", "bxc\ndef\n", (0, 1)),
        ("abc\ndef", "qAxq\"ap", "bxc\ndef\n", (0, 1)),
    ]);
}

#[test]
fn a_macro_still_replays_what_was_recorded() {
    check(&[
        ("abc\ndef", "qaxqj@a", "bc\nef\n", (1, 0)),
        ("abc\ndef", "qaA!<Esc>qj@a", "abc!\ndef!\n", (1, 3)),
        ("abc\ndef", "qaxqqAjxqu@a", "bc\nef\n", (1, 0)),
        ("1\n1\n1", "qa<C-a>qj@a", "2\n2\n1\n", (1, 0)),
        ("a\na\na\na", "qaA!<Esc>jq3@a", "a!\na!\na!\na!\n", (3, 1)),
        ("a\na\na\na", "qaA!<Esc>jq@a@@", "a!\na!\na!\na\n", (3, 0)),
        ("abc\nd", "qaA<CR>x<Esc>qj@a", "abc\nx\nd\nx\n", (3, 0)),
    ]);
}

#[test]
fn recording_leaves_the_other_registers_alone() {
    check(&[
        ("abc\ndef", "qaxq\"\"p", "bac\ndef\n", (0, 1)),
        ("abc\ndef", "qadwq\"ap", "dw\ndef\n", (0, 1)),
        ("abc\ndef", "yyqaxq\"ap", "bxc\ndef\n", (0, 1)),
        ("abc\ndef", "qaxq\"1p", "bc\ndef\n", (0, 0)),
        ("abc\ndef", "qadwq\"-p", "abc\ndef\n", (0, 2)),
    ]);
}

#[test]
fn a_register_replaces_the_recorded_keys_when_written_to() {
    check(&[
        ("abc\ndef", "qaxqj\"ayy@a", "bc\n\n", (1, 0)),
        ("abc def", "\"ayiw@a<Esc>", "abcbc def\n", (0, 2)),
        ("abc\nx", "j\"ayl0k@a", "bc\nx\n", (0, 0)),
        ("abc\nx", "j\"ayl0k@a@@", "c\nx\n", (0, 0)),
        ("abc\nxyz", "qaxqj\"ayl@a", "bc\nyz\n", (1, 0)),
    ]);
}
