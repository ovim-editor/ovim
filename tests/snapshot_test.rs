mod helpers;
use helpers::EditorTest;
use insta::assert_snapshot;

#[test]
fn test_o_command_snapshot() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('o')
        .type_text("new line")
        .press_esc();

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_o_with_indentation_snapshot() {
    let mut test = EditorTest::new("start\n    indented line\nend");

    test.press('j') // Move to indented line
        .press('o')
        .type_text("same indent")
        .press_esc();

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_o_last_line_no_newline() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('j') // Move to last line
        .press('o')
        .type_text("line 3")
        .press_esc();

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_simple_delete_word() {
    let mut test = EditorTest::new("hello world test");

    test.keys("dw"); // Delete word

    assert_snapshot!(test.snapshot_buffer_and_cursor());
}

#[test]
fn test_yank_paste_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("yy") // Yank line
        .keys("p"); // Paste after

    assert_snapshot!(test.snapshot_buffer_and_cursor());
}
