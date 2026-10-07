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
    // vim (nvim --clean): the cursor ends where the last deleted line was,
    // here line 2.
    test.assert_cursor(1, 0);
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

    // Multi-line output goes to the hover popup like every other command.
    let output = test.editor.hover_info().unwrap_or_default().to_string();
    assert!(output.contains("2: foo two"), "output: {}", output);
    assert!(output.contains("3: three foo"), "output: {}", output);
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
fn test_command_global_substitute_honors_escaped_delimiters() {
    let mut test = EditorTest::new("pick a/b\nskip a/b\n");

    InputHandler::execute_command_string(&mut test.editor, r"%g/pick/s/a\/b/c\/d/").unwrap();

    assert_eq!(test.buffer_content(), "pick c/d\nskip a/b\n");
    assert_eq!(test.editor.status_message(), "Substituted on 1 line(s)");
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
    // vim's message for a :g without matches.
    assert_eq!(test.editor.status_message(), "Pattern not found: foo");
}

// nvim --clean: `:set ic` then `:g/foo/d` deletes "Foo" too.
#[test]
fn global_honours_ignorecase() {
    let mut test = EditorTest::new("a\nFoo\nb\n");
    test.command("set ic");
    test.command("g/foo/d");
    assert_eq!(test.buffer_content(), "a\nb\n");
}

// nvim --clean: `:set ic` then `:/foo/d` (a search address) finds "Foo".
#[test]
fn search_address_honours_ignorecase() {
    let mut test = EditorTest::new("a\nFoo\nb\n");
    test.command("set ic");
    test.command("/foo/d");
    assert_eq!(test.buffer_content(), "a\nb\n");
}

// nvim --clean: `/foo\c` finds "FOO" without 'ignorecase'.
#[test]
fn search_honours_case_escape() {
    let mut test = EditorTest::new("a FOO b\n");
    test.keys("/foo\\c<CR>");
    test.assert_cursor(0, 2);
}

// nvim --clean: "a1 b a2 c" with `:g/a/norm yyGp` gives "a1 b a2 c a1 a2": the
// second visit is still the line that held "a2", although the first visit
// appended a line at the end.
#[test]
fn global_normal_follows_the_marked_lines() {
    let mut test = EditorTest::new("a1\nb\na2\nc\n");
    test.command("g/a/norm yyGp");
    assert_eq!(test.buffer_content(), "a1\nb\na2\nc\na1\na2\n");
    // One undo step for the whole :g.
    test.keys("u");
    assert_eq!(test.buffer_content(), "a1\nb\na2\nc\n");
}

// nvim --clean: `:g/a/norm Ox` puts a line above each match; the marks move
// down with their lines, so every "a" gets exactly one line above it.
#[test]
fn global_normal_follows_lines_pushed_down_by_inserts_above() {
    let mut test = EditorTest::new("a\nb\na\n");
    test.command("g/a/norm Ox");
    assert_eq!(test.buffer_content(), "x\na\nb\nx\na\n");
}

// nvim --clean: `:g/a/j` on "a a b" joins the first pair only; the second
// "a" was joined away, so its mark is gone: "a a" and "b".
#[test]
fn global_skips_lines_joined_away() {
    let mut test = EditorTest::new("a\na\nb\n");
    test.command("g/a/j");
    assert_eq!(test.buffer_content(), "a a\nb\n");
}

// nvim --clean: `:g/b/norm dd` on "a b b c" deletes both b lines; a mark on a
// line deleted by an earlier visit is dropped ("a c").
#[test]
fn global_skips_lines_deleted_by_earlier_visits() {
    let mut test = EditorTest::new("a\nb\nb\nc\n");
    test.command("g/b/norm dd");
    assert_eq!(test.buffer_content(), "a\nc\n");
}

// nvim --clean: `:g/b/norm jdd` on "b b c": the first visit deletes the line
// holding the second match, so that mark is dropped and "b c" is left.
#[test]
fn global_drops_marks_on_lines_deleted_below_the_cursor() {
    let mut test = EditorTest::new("b\nb\nc\n");
    test.command("g/b/norm jdd");
    assert_eq!(test.buffer_content(), "b\nc\n");
}

// nvim --clean: `:g/a/m0` reverses the matching lines onto the top.
#[test]
fn global_runs_other_commands_per_line() {
    let mut test = EditorTest::new("c1\nb2\na3\nd4a\n");
    test.command("g/a/m0");
    assert_eq!(test.buffer_content(), "d4a\na3\nc1\nb2\n");
}

// nvim --clean: more than the edit log keeps (64 edits) in one visit does not
// confuse the marks: three lines each get 78 typed characters.
#[test]
fn global_normal_survives_visits_with_many_edits() {
    let typed = "abcdefghijklmnopqrstuvwxyz".repeat(3);
    let mut test = EditorTest::new("x\nx\nx\n");
    test.command(&format!("g/x/normal A{typed}"));
    let expected = format!("x{typed}\n").repeat(3);
    assert_eq!(test.buffer_content(), expected);
}

// nvim --clean: `:g/a/g/b/d` nested without a range is allowed in vim; ovim
// refuses (E147) rather than guess the inner range.
#[test]
fn nested_global_is_refused() {
    let mut test = EditorTest::new("a\nb\n");
    test.command("g/a/g/b/d");
    assert_eq!(
        test.editor.status_message(),
        "E147: Cannot do :global recursive"
    );
}

// nvim --clean: `:g/b/d|3d` deletes b2, then line 3 of the text left, for
// each match: "c1 b2 a3 d4a" gives "c1 a3".
#[test]
fn global_runs_the_commands_after_a_bar() {
    let mut test = EditorTest::new("c1\nb2\na3\nd4a\n");
    test.command("g/b/d|3d");
    assert_eq!(test.buffer_content(), "c1\na3\n");
}

// nvim --clean: `:g/o/d | w file` writes after every deletion; the last write
// holds what is left ("x").
#[tokio::test(flavor = "multi_thread")]
async fn global_bar_write_writes_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("g.txt");
    std::fs::write(&path, "oo\nx\no\n").unwrap();
    let mut test = EditorTest::new("");
    test.load_file(path.to_str().unwrap());

    test.command("g/o/d | w");

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x\n");
}
