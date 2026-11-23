//! `e` / `E` landing on a one-character word at the start of the next line.
//! Every expectation was produced with `nvim --clean --headless` (`normal! {keys}`).

mod helpers;
use helpers::EditorTest;

fn run(content: &str, keys: &str) -> EditorTest {
    let mut test = EditorTest::new(content);
    test.keys(keys);
    test
}

#[test]
fn e_and_big_e_stop_on_a_one_character_word_starting_the_next_line() {
    // nvim: "foo\n}\nbar", `$e` -> (1,0); `$E` -> (1,0).
    run("foo\n}\nbar\n", "$e").assert_cursor(1, 0);
    run("foo\n}\nbar\n", "$E").assert_cursor(1, 0);
    // nvim: "foo\na b\nbar baz", `$e` -> (1,0): the `a` is a word.
    run("foo\na b\nbar baz\n", "$e").assert_cursor(1, 0);
    run("foo\na b\nbar baz\n", "$E").assert_cursor(1, 0);
    // nvim: a blank line in between and trailing blanks before the break change nothing.
    run("foo\n\n}\nbar\n", "$e").assert_cursor(2, 0);
    run("foo   \n}\nbar baz\n", "ee").assert_cursor(1, 0);
    // nvim: from an empty line, `e` / `E` stop on the first word's end, even a one-character one.
    run("\n}\nbar\n", "e").assert_cursor(1, 0);
    run("\nx\nbar\n", "e").assert_cursor(1, 0);
    run("\n}\nbar\n", "E").assert_cursor(1, 0);
    run("foo.\n;\nbar baz\n", "$E").assert_cursor(1, 0);
}

#[test]
fn counted_e_counts_the_one_character_word() {
    // nvim: "foo\n}\nbar baz": `$ee` -> (2,2), `$2e` -> (2,2), `$3e` -> (2,6).
    run("foo\n}\nbar baz\n", "$ee").assert_cursor(2, 2);
    run("foo\n}\nbar baz\n", "$2e").assert_cursor(2, 2);
    run("foo\n}\nbar baz\n", "$3e").assert_cursor(2, 6);
    run("foo\n}\nbar baz\n", "$EE").assert_cursor(2, 2);
    // nvim: "a\nb\nc\nd", `e3e` -> (3,0).
    run("a\nb\nc\nd\n", "e3e").assert_cursor(3, 0);
}

#[test]
fn operators_with_e_stop_at_the_one_character_word() {
    // nvim: `$de` on "foo\n}\nbar" deletes "o\n}" and leaves "fo\nbar".
    let test = run("foo\n}\nbar\n", "$de");
    assert_eq!(test.buffer_content(), "fo\nbar\n");
    assert_eq!(test.get_register_content('"').as_deref(), Some("o\n}"));
    test.assert_cursor(0, 1);
    // nvim: `$dE` is the same.
    let test = run("foo\n}\nbar\n", "$dE");
    assert_eq!(test.buffer_content(), "fo\nbar\n");
    // nvim: `$ceX<Esc>` -> "foX\nbar".
    let test = run("foo\n}\nbar\n", "$ceX<Esc>");
    assert_eq!(test.buffer_content(), "foX\nbar\n");
    // nvim: `$2de` -> "fo baz" (register "o\n}\nbar").
    let test = run("foo\n}\nbar baz\n", "$2de");
    assert_eq!(test.buffer_content(), "fo baz\n");
    assert_eq!(test.get_register_content('"').as_deref(), Some("o\n}\nbar"));
}
