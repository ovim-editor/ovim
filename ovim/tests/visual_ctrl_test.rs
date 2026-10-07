//! Ctrl chords in Visual mode are not the plain-letter commands.
//! Expectations come from `nvim --clean --headless` (`normal!`).

mod helpers;
use helpers::EditorTest;
use ovim::mode::Mode;

fn run(content: &str, keys: &str) -> EditorTest {
    let mut test = EditorTest::new(content);
    test.keys(keys);
    test
}

#[test]
fn ctrl_c_and_ctrl_bracket_leave_visual_mode_without_changing_text() {
    // nvim: "foo bar baz", `wve<C-c>` leaves Visual with the text intact (not `c`).
    let test = run("foo bar baz\n", "wve<C-c>");
    assert_eq!(test.buffer_content(), "foo bar baz\n");
    test.assert_mode(Mode::Normal);
    test.assert_cursor(0, 6);
    assert_eq!(test.get_register_content('"'), None);

    let test = run("foo bar baz\n", "wve<C-[>");
    assert_eq!(test.buffer_content(), "foo bar baz\n");
    test.assert_mode(Mode::Normal);

    // nvim: the next `x` deletes one character, not the old selection.
    let test = run("foo bar baz\n", "wve<C-c>x");
    assert_eq!(test.buffer_content(), "foo ba baz\n");
}

#[test]
fn ctrl_x_without_a_number_changes_nothing() {
    // nvim: `wve<C-x>` -> no deletion, back in Normal mode at the selection start.
    let test = run("foo bar baz\n", "wve<C-x>");
    assert_eq!(test.buffer_content(), "foo bar baz\n");
    test.assert_mode(Mode::Normal);
    test.assert_cursor(0, 4);
}

#[test]
fn ctrl_a_and_ctrl_x_change_the_first_number_on_each_selected_line() {
    // nvim: `Vjj<C-a>` -> every line +1; `<C-x>` -1; `3<C-a>` -> +3.
    let test = run("1\n1\n1\n", "Vjj<C-a>");
    assert_eq!(test.buffer_content(), "2\n2\n2\n");
    test.assert_mode(Mode::Normal);
    test.assert_cursor(0, 0);
    assert_eq!(
        run("5 6\n5 6\n5 6\n", "Vjj<C-x>").buffer_content(),
        "4 6\n4 6\n4 6\n"
    );
    assert_eq!(run("1\n1\n1\n", "Vjj3<C-a>").buffer_content(), "4\n4\n4\n");
    // nvim: characterwise selections only reach numbers inside them:
    // "a1 b2\nc3 d4", `wvj<C-a>` -> "a1 b3\nc4 d4".
    let test = run("a1 b2\nc3 d4\n", "wvj<C-a>");
    assert_eq!(test.buffer_content(), "a1 b3\nc4 d4\n");
    test.assert_cursor(0, 3);
    // nvim: a block over a column without numbers changes nothing.
    assert_eq!(
        run("1 1\n1 1\n1 1\n", "l<C-v>jj<C-a>").buffer_content(),
        "1 1\n1 1\n1 1\n"
    );
}

#[test]
fn g_ctrl_a_adds_an_increasing_multiple_starting_with_the_first_line() {
    // nvim: `Vjjg<C-a>` on 1,1,1 -> 2,3,4; `2g<C-a>` -> 3,5,7.
    assert_eq!(run("1\n1\n1\n", "Vjjg<C-a>").buffer_content(), "2\n3\n4\n");
    assert_eq!(run("1\n1\n1\n", "Vjj2g<C-a>").buffer_content(), "3\n5\n7\n");
    // nvim: lines without a number are skipped and do not advance the sequence.
    assert_eq!(
        run("1\nx\n1\n1\n", "Vjjjg<C-a>").buffer_content(),
        "2\nx\n3\n4\n"
    );
}

#[test]
fn other_ctrl_chords_do_not_run_plain_letter_commands() {
    // nvim: Ctrl-Y / Ctrl-E scroll the view; the selection survives (not `y` / `e`).
    for chord in ["<C-y>", "<C-e>"] {
        let test = run("foo bar baz\n", &format!("wv{chord}"));
        test.assert_mode(Mode::Visual);
        assert_eq!(test.get_register_content('"'), None, "{chord}");
        assert_eq!(test.buffer_content(), "foo bar baz\n", "{chord}");
    }
    // nvim: Ctrl-W waits for a window command; Ctrl-R is nothing in Visual mode.
    for chord in ["<C-w>", "<C-r>"] {
        let test = run("foo bar baz\n", &format!("wv{chord}"));
        test.assert_mode(Mode::Visual);
        assert_eq!(test.buffer_content(), "foo bar baz\n", "{chord}");
    }
    // nvim: `wlv<C-h>d` -> Ctrl-H moves left like `h`: "foo r baz".
    assert_eq!(
        run("foo bar baz\n", "wlv<C-h>d").buffer_content(),
        "foo r baz\n"
    );
    // nvim: Ctrl-N is `j`: "foo\nbar\nbaz", `v<C-n>d` -> "ar\nbaz".
    assert_eq!(
        run("foo\nbar\nbaz\n", "v<C-n>d").buffer_content(),
        "ar\nbaz\n"
    );
}
