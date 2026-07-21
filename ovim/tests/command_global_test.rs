mod helpers;

use helpers::EditorTest;
use ovim::editor::InputHandler;

#[test]
fn test_command_global_percent_delete_matching_lines() {
    let mut test = EditorTest::new("keep\nfoo 1\ndrop foo 2\nkeep2\n");

    InputHandler::execute_command_string(&mut test.editor, "%g/foo/d").unwrap();

    assert_eq!(test.buffer_content(), "keep\nkeep2\n");
    assert_eq!(test.editor.status_message(), "Deleted 2 line(s)");
    test.assert_cursor(1, 0);
}

#[test]
fn test_command_global_percent_delete_non_matching_lines_g_bang() {
    let mut test = EditorTest::new("keep\nfoo 1\ndrop foo 2\nkeep2\n");

    InputHandler::execute_command_string(&mut test.editor, "%g!/foo/d").unwrap();

    assert_eq!(test.buffer_content(), "foo 1\ndrop foo 2\n");
    assert_eq!(test.editor.status_message(), "Deleted 2 line(s)");
    test.assert_cursor(0, 0);
}

#[test]
fn test_command_vglobal_percent_delete_non_matching_lines_v() {
    let mut test = EditorTest::new("keep\nfoo 1\ndrop foo 2\nkeep2\n");

    InputHandler::execute_command_string(&mut test.editor, "%v/foo/d").unwrap();

    assert_eq!(test.buffer_content(), "foo 1\ndrop foo 2\n");
    assert_eq!(test.editor.status_message(), "Deleted 2 line(s)");
}

#[test]
fn test_command_global_percent_default_print_command() {
    let mut test = EditorTest::new("one\nfoo two\nthree foo\nfour\n");

    InputHandler::execute_command_string(&mut test.editor, "%g/foo/").unwrap();

    let status = test.editor.status_message();
    assert!(status.contains("2: foo two"), "status: {}", status);
    assert!(status.contains("3: three foo"), "status: {}", status);
}

#[test]
fn test_command_global_percent_yank_matching_lines() {
    let mut test = EditorTest::new("one\nfoo two\nthree foo\nfour\n");

    InputHandler::execute_command_string(&mut test.editor, "%g/foo/y").unwrap();

    assert_eq!(test.buffer_content(), "one\nfoo two\nthree foo\nfour\n");
    assert_eq!(test.editor.status_message(), "Yanked 2 line(s)");

    let yanked = test
        .get_register_content('"')
        .expect("expected unnamed register to contain yanked text");
    assert_eq!(yanked, "foo two\nthree foo\n");
}

#[test]
fn test_command_global_percent_substitute_on_matching_lines() {
    let mut test = EditorTest::new("one\nfoo two\nthree foo\nfour\n");

    InputHandler::execute_command_string(&mut test.editor, "%g/foo/s/foo/bar/").unwrap();

    assert_eq!(test.buffer_content(), "one\nbar two\nthree bar\nfour\n");
    assert_eq!(test.editor.status_message(), "Substituted on 2 line(s)");
}

#[test]
fn test_command_global_range_restricts_matches() {
    let mut test = EditorTest::new("foo\nfoo\nfoo\nfoo\n");

    InputHandler::execute_command_string(&mut test.editor, "2,3g/foo/d").unwrap();

    assert_eq!(test.buffer_content(), "foo\nfoo\n");
    assert_eq!(test.editor.status_message(), "Deleted 2 line(s)");
}

#[test]
fn test_command_global_percent_pattern_with_pipe_is_not_command_chain() {
    let mut test = EditorTest::new("keep\nfoo\nbar\nbaz\n");

    // If '|' were treated as command chaining, this would split and fail.
    InputHandler::execute_command_string(&mut test.editor, "%g/foo|bar/d").unwrap();

    assert_eq!(test.buffer_content(), "keep\nbaz\n");
    assert_eq!(test.editor.status_message(), "Deleted 2 line(s)");
}

#[test]
fn test_command_global_percent_undo_restores_deleted_lines() {
    let mut test = EditorTest::new("keep\nfoo\nbar\nbaz\n");

    InputHandler::execute_command_string(&mut test.editor, "%g/foo|bar/d").unwrap();
    assert_eq!(test.buffer_content(), "keep\nbaz\n");

    test.keys("u");
    assert_eq!(test.buffer_content(), "keep\nfoo\nbar\nbaz\n");
}

#[test]
fn test_command_global_percent_delete_undo_redo_macro_flow() {
    editor_flow_test! {
        content "keep\nfoo\nbar\nbaz\n";
        step ":%g/foo|bar/d<Enter>" => |test| {
            assert_eq!(test.buffer_content(), "keep\nbaz\n");
            assert_eq!(test.editor.status_message(), "Deleted 2 line(s)");
        }
        step "u" => |test| {
            assert_eq!(test.buffer_content(), "keep\nfoo\nbar\nbaz\n");
        }
        step "<C-r>" => |test| {
            assert_eq!(test.buffer_content(), "keep\nbaz\n");
        }
    }
}

#[test]
fn test_command_global_no_matches_is_non_destructive() {
    let mut test = EditorTest::new("one\ntwo\nthree\n");

    InputHandler::execute_command_string(&mut test.editor, "%g/foo/d").unwrap();

    assert_eq!(test.buffer_content(), "one\ntwo\nthree\n");
    assert_eq!(test.editor.status_message(), "No matching lines found");
}
