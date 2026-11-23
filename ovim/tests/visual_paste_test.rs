//! Visual-mode `p`/`P` under the default `clipboard=unnamedplus`.
//!
//! The expectations below come from `nvim --clean` with `clipboard=unnamedplus`
//! backed by a recording provider: the selection is replaced by the register
//! contents captured *before* the selection is deleted, and afterwards the
//! unnamed register / clipboard hold the replaced text (`P` leaves them alone).

mod helpers;
use helpers::EditorTest;

fn clipboard(test: &EditorTest) -> String {
    test.editor.registers().get_clipboard()
}

#[test]
fn visual_p_replaces_selection_with_clipboard_text() {
    // nvim: "foo bar\nbaz qux", `yiwjwviwp` -> "baz foo", clipboard "qux".
    let mut test = EditorTest::with_default_clipboard("foo bar\nbaz qux\n");
    test.keys("yiwjwviwp");
    assert_eq!(test.buffer_content(), "foo bar\nbaz foo\n");
    assert_eq!(clipboard(&test), "qux");
    assert_eq!(test.get_register_content('"').as_deref(), Some("qux"));
    test.assert_cursor(1, 6);
}

#[test]
fn visual_p_at_end_of_line_keeps_text_before_the_selection() {
    // nvim: "foo bar", `yiwwviwp` -> "foo foo", clipboard "bar".
    let mut test = EditorTest::with_default_clipboard("foo bar\n");
    test.keys("yiwwviwp");
    assert_eq!(test.buffer_content(), "foo foo\n");
    assert_eq!(clipboard(&test), "bar");
}

#[test]
fn visual_p_twice_swaps_back_and_forth() {
    // nvim: after the first `p` the replaced text is what the next `p` pastes.
    let mut test = EditorTest::with_default_clipboard("foo bar\n");
    test.keys("yiwwviwp0viwp");
    assert_eq!(test.buffer_content(), "bar foo\n");
}

#[test]
fn visual_capital_p_leaves_register_and_clipboard_alone() {
    // nvim: "foo bar", `yiwwviwP` -> "foo foo", unnamed register and clipboard stay "foo".
    let mut test = EditorTest::with_default_clipboard("foo bar\n");
    test.keys("yiwwviwP");
    assert_eq!(test.buffer_content(), "foo foo\n");
    assert_eq!(clipboard(&test), "foo");
    assert_eq!(test.get_register_content('"').as_deref(), Some("foo"));
}

#[test]
fn visual_p_linewise_register_over_charwise_selection_splits_the_line() {
    // nvim: "one\ntwo three", `yyjwviwp` -> "one\ntwo \none\n" + empty remainder line.
    let mut test = EditorTest::with_default_clipboard("one\ntwo three\n");
    test.keys("yyjwviwp");
    assert_eq!(test.buffer_content(), "one\ntwo \none\n\n");
    assert_eq!(clipboard(&test), "three");
}

#[test]
fn visual_line_p_with_charwise_register_replaces_the_line() {
    // nvim: "one\ntwo three", `yiwjVp` -> "one\none", unnamed register is the linewise "two three".
    let mut test = EditorTest::with_default_clipboard("one\ntwo three\n");
    test.keys("yiwjVp");
    assert_eq!(test.buffer_content(), "one\none\n");
    assert_eq!(clipboard(&test), "two three\n");
}

#[test]
fn visual_p_works_without_clipboard_option_too() {
    // Same keys with `clipboard=` (what EditorTest::new uses): identical buffer.
    let mut test = EditorTest::new("foo bar\nbaz qux\n");
    test.keys("yiwjwviwp");
    assert_eq!(test.buffer_content(), "foo bar\nbaz foo\n");
    assert_eq!(test.get_register_content('"').as_deref(), Some("qux"));
}

#[test]
fn visual_p_undoes_in_one_step() {
    // nvim: `yiwwviwpu` restores "foo bar" in a single undo.
    let mut test = EditorTest::with_default_clipboard("foo bar\nbaz\n");
    test.keys("yiwwviwpu");
    assert_eq!(test.buffer_content(), "foo bar\nbaz\n");
}

#[test]
fn visual_line_p_on_whole_buffer_leaves_only_the_pasted_text() {
    // nvim: "a b\nbar", `yiwVGp` -> "a" (no leftover blank line).
    let mut test = EditorTest::with_default_clipboard("a b\nbar\n");
    test.keys("yiwVGp");
    assert_eq!(test.buffer_content(), "a\n");
}

#[test]
fn visual_line_p_over_last_lines_puts_below_the_remaining_text() {
    // nvim: "a b\nbar\nbaz", `yyjVGp` -> "a b\na b".
    let mut test = EditorTest::with_default_clipboard("a b\nbar\nbaz\n");
    test.keys("yyjVGp");
    assert_eq!(test.buffer_content(), "a b\na b\n");
    test.assert_cursor(1, 0);

    // nvim: "a b\nbar", `yiwjVp` -> "a b\na" (charwise text becomes its own line).
    let mut test = EditorTest::with_default_clipboard("a b\nbar\n");
    test.keys("yiwjVp");
    assert_eq!(test.buffer_content(), "a b\na\n");
}

#[test]
fn visual_line_p_over_leading_lines_puts_above_the_rest() {
    // nvim: "a b\nbar\nbaz", `yyVjp` -> "a b\nbaz".
    let mut test = EditorTest::with_default_clipboard("a b\nbar\nbaz\n");
    test.keys("yyVjp");
    assert_eq!(test.buffer_content(), "a b\nbaz\n");
    test.assert_cursor(0, 0);
}

#[test]
fn visual_p_linewise_over_middle_of_line_and_at_line_start() {
    // nvim: "one\ntwo three\nfour", `yyjwlvlp` -> "one\ntwo t\none\nee\nfour", undo restores.
    let mut test = EditorTest::with_default_clipboard("one\ntwo three\nfour\n");
    test.keys("yyjwlvlp");
    assert_eq!(test.buffer_content(), "one\ntwo t\none\nee\nfour\n");
    test.keys("u");
    assert_eq!(test.buffer_content(), "one\ntwo three\nfour\n");

    // nvim: "one\ntwo three", `yyjviwp` -> "one\n\none\n three".
    let mut test = EditorTest::with_default_clipboard("one\ntwo three\n");
    test.keys("yyjviwp");
    assert_eq!(test.buffer_content(), "one\n\none\n three\n");
}
