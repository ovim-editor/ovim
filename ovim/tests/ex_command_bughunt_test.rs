//! Regression tests for ex-command bugs found in the bug hunt:
//!  - :t/:m to address 0 insert at the top of the file
//!  - :t/:m without a space (:3t0) are recognized
//!  - ranges beginning with a mark (:'a,'bd) parse correctly
//!  - empty search pattern repeats the last search

#![allow(non_snake_case)]

mod helpers;
use helpers::EditorTest;

#[test]
fn test_copy_to_address_zero_inserts_at_top() {
    let mut test = EditorTest::new("a\nb\nc\n");
    test.command("3t0"); // copy line 3 ("c") to the very top
    assert_eq!(test.buffer_content(), "c\na\nb\nc\n");
}

#[test]
fn test_move_to_address_zero_inserts_at_top() {
    let mut test = EditorTest::new("a\nb\nc\nd\n");
    test.command("3m0"); // move line 3 ("c") to the very top
    assert_eq!(test.buffer_content(), "c\na\nb\nd\n");
}

#[test]
fn test_copy_no_space_form() {
    let mut test = EditorTest::new("a\nb\nc\n");
    test.command("1t2"); // copy line 1 to after line 2
    assert_eq!(test.buffer_content(), "a\nb\na\nc\n");
}

#[test]
fn test_copy_with_space_still_works() {
    let mut test = EditorTest::new("a\nb\nc\n");
    test.command("1t 2");
    assert_eq!(test.buffer_content(), "a\nb\na\nc\n");
}

#[test]
fn test_move_no_space_form() {
    let mut test = EditorTest::new("a\nb\nc\nd\n");
    test.command("1m2"); // move line 1 ("a") to after line 2 ("b") -> b,a,c,d
    assert_eq!(test.buffer_content(), "b\na\nc\nd\n");
}

#[test]
fn test_range_starting_with_mark_delete() {
    let mut test = EditorTest::new("l0\nl1\nl2\nl3\nl4\n");
    // set mark a on line 1, mark b on line 3
    test.keys("j");
    test.keys("ma");
    test.keys("jj");
    test.keys("mb");
    // :'a,'bd deletes lines 1..3 (l1,l2,l3)
    test.command("'a,'bd");
    assert_eq!(test.buffer_content(), "l0\nl4\n");
}

#[test]
fn test_address_single_mark_jumps() {
    let mut test = EditorTest::new("l0\nl1\nl2\nl3\n");
    test.keys("jjj"); // cursor line 3
    test.keys("ma");
    test.keys("gg"); // back to top
    test.command("'a"); // jump to mark a
    assert_eq!(test.cursor().0, 3, "':a should jump to mark a's line");
}

#[test]
fn test_interactive_substitute_same_line_length_change() {
    // :s/a/XX/gc on "aaa" — confirming all three matches must not corrupt the
    // buffer when the replacement is longer than the match.
    let mut test = EditorTest::new("aaa");
    test.command("s/a/XX/gc");
    test.keys("a"); // confirm all remaining
    assert_eq!(test.buffer_content(), "XXXXXX\n");
}

#[test]
fn test_interactive_substitute_shorter_replacement() {
    let mut test = EditorTest::new("aa aa aa");
    test.command("s/aa/b/gc");
    test.keys("a");
    assert_eq!(test.buffer_content(), "b b b\n");
}

#[test]
fn test_interactive_substitute_individual_confirm() {
    // Confirm first, skip second, confirm third with differing length.
    let mut test = EditorTest::new("a a a");
    test.command("s/a/XX/gc");
    test.keys("y"); // first -> XX
    test.keys("n"); // skip second
    test.keys("y"); // third -> XX
    assert_eq!(test.buffer_content(), "XX a XX\n");
}

#[test]
fn test_empty_search_repeats_last() {
    let mut test = EditorTest::new("b x b x b");
    test.keys("/b<CR>"); // search for b, lands on next b
                         // Now an empty search should repeat, not wipe the search.
    test.keys("/<CR>");
    // n should still work: find another b
    let before = test.cursor();
    test.keys("n");
    assert_ne!(
        test.cursor(),
        before,
        "n should still advance after an empty-pattern search"
    );
}

// nvim --clean (0.12.2): `:nmap Q :normal Q<CR>` then a typed `Q` stops with
// "E169: Command too recursive"; `:normal Q` run from a script stops with
// "E192: Recursive use of :normal too deep". The buffer is left alone and the
// editor survives. Before the guard ovim overflowed its stack and aborted.
fn assert_stopped_by_recursion_guard(test: &EditorTest) {
    let status = test.editor.status_message();
    assert!(
        status.contains("E192") || status.contains("E169"),
        "expected a recursion error, got: {status:?}"
    );
}

#[test]
fn recursive_normal_mapping_stops_with_an_error() {
    let mut test = EditorTest::new("a\nb\n");
    // Not `test.command`, which would type the `<CR>` as an Enter key.
    ovim::commands::execute_command(&mut test.editor, "nmap Q :normal Q<CR>");
    test.keys("Q");
    assert_stopped_by_recursion_guard(&test);
    assert_eq!(test.buffer_content(), "a\nb\n");
}

#[test]
fn recursive_normal_mapping_stops_inside_global() {
    // nvim --clean: `:g/a/normal Q` with the same mapping reports E169.
    let mut test = EditorTest::new("a\nb\n");
    ovim::commands::execute_command(&mut test.editor, "nmap Q :normal Q<CR>");
    test.command("g/a/normal Q");
    assert_stopped_by_recursion_guard(&test);
    assert_eq!(test.buffer_content(), "a\nb\n");
}

#[test]
fn nested_normal_still_works_a_few_levels_deep() {
    // nvim --clean: `:nmap Q :normal x<CR>` then `:normal Q` deletes one character.
    let mut test = EditorTest::new("abc\n");
    ovim::commands::execute_command(&mut test.editor, "nmap Q :normal x<CR>");
    test.command("normal Q");
    assert_eq!(test.buffer_content(), "bc\n");
}

fn write_pair(dir: &tempfile::TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let first = dir.path().join("a.txt");
    let second = dir.path().join("b.txt");
    std::fs::write(&first, "one\n").unwrap();
    std::fs::write(&second, "two\n").unwrap();
    (first, second)
}

// nvim --clean: after `x` in a.txt, `:sp b.txt` and `:vs b.txt` open b.txt in
// a new window (2, then 3 windows); a.txt stays loaded and modified.
#[tokio::test(flavor = "multi_thread")]
async fn split_with_a_file_works_while_the_current_buffer_is_modified() {
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = write_pair(&dir);
    let mut test = EditorTest::new("");
    test.load_file(first.to_str().unwrap());
    test.keys("x");
    assert!(test.editor.is_modified());

    test.command(&format!("sp {}", second.display()));
    assert_eq!(test.editor.window_count(), 2);
    assert_eq!(test.buffer_content(), "two\n");
    assert_eq!(
        test.editor.buffer().file_path(),
        Some(second.to_str().unwrap())
    );
    assert!(test.editor.any_buffer_modified(), "a.txt keeps its edit");

    test.command(&format!("vs {}", second.display()));
    assert_eq!(test.editor.window_count(), 3);
    assert_eq!(test.buffer_content(), "two\n");
}

// nvim --clean: `:sp unreadable` reports "[Permission Denied]". Here the
// failed split must not leave an empty extra window behind.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn failed_split_with_a_file_closes_the_new_window() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = write_pair(&dir);
    std::fs::set_permissions(&second, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&second).is_ok() {
        return; // running as root: nothing is unreadable
    }
    let mut test = EditorTest::new("");
    test.load_file(first.to_str().unwrap());

    let result =
        ovim::commands::execute_command(&mut test.editor, &format!("sp {}", second.display()));

    assert!(matches!(
        result,
        ovim::command_result::CommandResult::Error(_)
    ));
    assert_eq!(test.editor.window_count(), 1);
    assert_eq!(test.buffer_content(), "one\n");
}

fn run(test: &mut EditorTest, command: &str) -> ovim::command_result::CommandResult {
    ovim::commands::execute_command(&mut test.editor, command)
}

fn current_file(test: &EditorTest) -> String {
    test.editor.buffer().file_path().unwrap_or("").to_string()
}

// nvim --clean: with b.txt current and a.txt alternate, `:e #` swaps them
// (twice returns to b.txt); `:b#` does the same through the buffer list.
#[tokio::test(flavor = "multi_thread")]
async fn edit_hash_opens_the_alternate_file() {
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = write_pair(&dir);
    let mut test = EditorTest::new("");
    test.load_file(first.to_str().unwrap());
    test.load_file(second.to_str().unwrap());
    let buffers = test.editor.buffer_count();

    run(&mut test, "e #");
    assert_eq!(current_file(&test), first.to_str().unwrap());
    run(&mut test, "e #");
    assert_eq!(current_file(&test), second.to_str().unwrap());
    run(&mut test, "b#");
    assert_eq!(current_file(&test), first.to_str().unwrap());
    assert_eq!(
        test.editor.buffer_count(),
        buffers,
        "no buffer named # was made"
    );
}

// nvim --clean: `:e %:h/sub/x.txt` is relative to the current file's directory.
#[tokio::test(flavor = "multi_thread")]
async fn edit_percent_head_is_relative_to_the_current_file() {
    let dir = tempfile::tempdir().unwrap();
    let (first, _) = write_pair(&dir);
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let nested = dir.path().join("sub").join("x.txt");
    std::fs::write(&nested, "in sub\n").unwrap();
    let mut test = EditorTest::new("");
    test.load_file(first.to_str().unwrap());

    let result = run(&mut test, "e %:h/sub/x.txt");

    assert!(matches!(
        result,
        ovim::command_result::CommandResult::Success(_)
    ));
    assert_eq!(test.buffer_content(), "in sub\n");
    assert_eq!(current_file(&test), nested.to_str().unwrap());
}

// nvim --clean: `:w! %:r.bak` writes a copy next to the file (b.txt -> b.bak).
#[tokio::test(flavor = "multi_thread")]
async fn write_expands_percent_modifiers() {
    let dir = tempfile::tempdir().unwrap();
    let (first, _) = write_pair(&dir);
    let mut test = EditorTest::new("");
    test.load_file(first.to_str().unwrap());

    run(&mut test, "w %:r.bak");

    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.bak")).unwrap(),
        "one\n"
    );
}

// nvim --clean: `\%` is a literal percent sign (`:e a.txt\%x` edits "a.txt%x").
#[tokio::test(flavor = "multi_thread")]
async fn escaped_percent_in_a_file_argument_stays_literal() {
    let dir = tempfile::tempdir().unwrap();
    let (first, _) = write_pair(&dir);
    let mut test = EditorTest::new("");
    test.load_file(first.to_str().unwrap());

    run(&mut test, &format!("e {}\\%x", first.display()));

    assert_eq!(current_file(&test), format!("{}%x", first.display()));
}

// nvim --clean: `:e %` in an unnamed buffer is E499 and `:e #` without an
// alternate file is E194; neither opens a buffer named after the pattern.
#[test]
fn unusable_percent_and_hash_are_errors() {
    let mut test = EditorTest::new("text\n");

    let unnamed = run(&mut test, "e %");
    assert!(
        matches!(&unnamed, ovim::command_result::CommandResult::Error(e) if e.error.starts_with("E499")),
        "{unnamed:?}"
    );
    let no_alternate = run(&mut test, "e #");
    assert!(
        matches!(&no_alternate, ovim::command_result::CommandResult::Error(e) if e.error.starts_with("E194")),
        "{no_alternate:?}"
    );
    assert_eq!(test.editor.buffer_count(), 1);
}

fn quickfix_fixture(dir: &tempfile::TempDir) -> Vec<ovim::editor::QuickfixEntry> {
    let path = dir.path().join("list.txt");
    std::fs::write(&path, "a\nb\nc\n").unwrap();
    (1..=3)
        .map(|line| {
            ovim::editor::QuickfixEntry::new(
                Some(path.clone()),
                line,
                1,
                ovim::editor::QuickfixEntryType::Info,
                format!("item {line}"),
            )
        })
        .collect()
}

// nvim --clean: with three items, `:cnext` on the last one is
// "E553: No more items" and stays there (a `999@q` macro relies on it to
// stop); `:cprev` on the first one is the same error. They do not wrap.
#[tokio::test(flavor = "multi_thread")]
async fn cnext_and_cprev_stop_at_the_ends_of_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let mut test = EditorTest::new("");
    test.editor
        .set_quickfix_list(quickfix_fixture(&dir), "test".to_string());
    let cursor_line = |test: &EditorTest| test.editor.buffer().cursor().line();

    run(&mut test, "cfirst");
    assert_eq!(cursor_line(&test), 0);
    let before_first = run(&mut test, "cprev");
    assert!(
        matches!(&before_first, ovim::command_result::CommandResult::Error(e) if e.error.starts_with("E553")),
        "{before_first:?}"
    );
    assert_eq!(cursor_line(&test), 0);

    run(&mut test, "cnext");
    run(&mut test, "cnext");
    assert_eq!(cursor_line(&test), 2);
    let past_last = run(&mut test, "cnext");
    assert!(
        matches!(&past_last, ovim::command_result::CommandResult::Error(e) if e.error.starts_with("E553")),
        "{past_last:?}"
    );
    assert_eq!(cursor_line(&test), 2);
    run(&mut test, "cprev");
    assert_eq!(cursor_line(&test), 1);
}

// nvim --clean: `:cclose` closes the quickfix window and keeps the list, so
// `:cnext` still moves through it.
#[tokio::test(flavor = "multi_thread")]
async fn cclose_keeps_the_quickfix_list() {
    let dir = tempfile::tempdir().unwrap();
    let mut test = EditorTest::new("");
    test.editor
        .set_quickfix_list(quickfix_fixture(&dir), "test".to_string());
    test.editor.open_quickfix_window();
    run(&mut test, "cfirst");

    run(&mut test, "cclose");

    assert!(!test.editor.is_quickfix_window_open());
    assert_eq!(test.editor.quickfix_list().len(), 3);
    run(&mut test, "cnext");
    assert_eq!(test.editor.buffer().cursor().line(), 1);
}

// nvim --clean: `:make hello | let g:x = 1` runs makeprg with just "hello" and
// then the command after the bar.
#[test]
fn make_arguments_end_at_a_bar() {
    let mut test = EditorTest::new("text\n");
    test.editor.options.makeprg = "echo".to_string();

    let result = run(&mut test, "make hello | set number");

    match result {
        ovim::command_result::CommandResult::Success(success) => {
            assert_eq!(success.message.as_deref(), Some("Running: echo hello"));
        }
        other => panic!("unexpected result: {other:?}"),
    }
    assert!(test.editor.options.number, "the command after | ran");
}

// nvim --clean: after another process deleted the file, `:w` writes it again
// (no E211 refusal); the deleted file has nothing of the user's to overwrite.
#[tokio::test(flavor = "multi_thread")]
async fn write_recreates_a_file_deleted_externally() {
    let dir = tempfile::tempdir().unwrap();
    let (first, _) = write_pair(&dir);
    let mut test = EditorTest::new("");
    test.load_file(first.to_str().unwrap());
    test.keys("x");
    std::fs::remove_file(&first).unwrap();

    let result = run(&mut test, "w");

    assert!(
        matches!(result, ovim::command_result::CommandResult::Success(_)),
        "{result:?}"
    );
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "ne\n");
    assert!(!test.editor.is_modified());
}

fn error_message(result: ovim::command_result::CommandResult) -> String {
    match result {
        ovim::command_result::CommandResult::Error(error) => error.error,
        other => panic!("expected an error, got {other:?}"),
    }
}

fn assert_success(result: ovim::command_result::CommandResult) {
    assert!(
        matches!(result, ovim::command_result::CommandResult::Success(_)),
        "{result:?}"
    );
}

// nvim --clean, buffer l1 l2 l3: `:1,2w part.txt` writes "l1\nl2\n" to the new
// file, again without ! is E13, with ! it overwrites; the buffer stays
// unnamed and modified.
#[tokio::test(flavor = "multi_thread")]
async fn ranged_write_to_another_file_needs_bang_only_for_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("part.txt");
    let mut test = EditorTest::new("l1\nl2\nl3\n");
    test.keys("x");

    assert_success(run(&mut test, &format!("1,2w {}", part.display())));
    assert_eq!(std::fs::read_to_string(&part).unwrap(), "1\nl2\n");

    let message = error_message(run(&mut test, &format!("1,2w {}", part.display())));
    assert!(message.starts_with("E13:"), "{message}");

    assert_success(run(&mut test, &format!("2,3w! {}", part.display())));
    assert_eq!(std::fs::read_to_string(&part).unwrap(), "l2\nl3\n");

    assert_eq!(
        current_file(&test),
        "",
        "a partial write does not name the buffer"
    );
    assert!(test.editor.is_modified());
}

// nvim --clean, editing own.txt (own1 own2 own3) after changing line 1:
// `:1,2w` and `:1,2w own.txt` are E140; `:1,2w!` writes the two lines to
// own.txt and leaves the buffer modified; a range covering the whole buffer
// is an ordinary write.
#[tokio::test(flavor = "multi_thread")]
async fn ranged_write_to_the_buffers_own_file_needs_bang() {
    let dir = tempfile::tempdir().unwrap();
    let own = dir.path().join("own.txt");
    std::fs::write(&own, "own1\nown2\nown3\n").unwrap();
    let mut test = EditorTest::new("");
    test.load_file(own.to_str().unwrap());
    test.keys("x");

    let message = error_message(run(&mut test, "1,2w"));
    assert!(message.starts_with("E140:"), "{message}");
    let message = error_message(run(&mut test, &format!("1,2w {}", own.display())));
    assert!(message.starts_with("E140:"), "{message}");
    assert_eq!(std::fs::read_to_string(&own).unwrap(), "own1\nown2\nown3\n");

    assert_success(run(&mut test, "1,3w"));
    assert_eq!(std::fs::read_to_string(&own).unwrap(), "wn1\nown2\nown3\n");
    assert!(!test.editor.is_modified());

    test.keys("x");
    assert_success(run(&mut test, "1,2w!"));
    assert_eq!(std::fs::read_to_string(&own).unwrap(), "n1\nown2\n");
    assert!(test.editor.is_modified());
}

// nvim --clean, buffer l1 l2 l3: `:w >> f` appends the buffer, `:1,2w >> f`
// and `:w>>f` append the range / without a blank; `:w >> missing` is E212 and
// creates nothing; `:w! >> missing` creates it.
#[tokio::test(flavor = "multi_thread")]
async fn write_append_adds_to_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("log.txt");
    let missing = dir.path().join("missing.txt");
    std::fs::write(&target, "old\n").unwrap();
    let mut test = EditorTest::new("l1\nl2\nl3\n");

    assert_success(run(&mut test, &format!("w >> {}", target.display())));
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "old\nl1\nl2\nl3\n"
    );
    assert_success(run(&mut test, &format!("2,3w >> {}", target.display())));
    assert_success(run(&mut test, &format!("1w>>{}", target.display())));
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "old\nl1\nl2\nl3\nl2\nl3\nl1\n"
    );

    let message = error_message(run(&mut test, &format!("w >> {}", missing.display())));
    assert!(message.starts_with("E212:"), "{message}");
    assert!(!missing.exists());
    assert_success(run(&mut test, &format!("w! >> {}", missing.display())));
    assert_eq!(std::fs::read_to_string(&missing).unwrap(), "l1\nl2\nl3\n");
}

// nvim --clean, editing own.txt: `:2,3w >>` appends lines 2-3 to the buffer's
// own file without E140 and leaves the buffer modified; with no file name it
// is E32.
#[tokio::test(flavor = "multi_thread")]
async fn write_append_without_a_file_uses_the_buffers_file() {
    let dir = tempfile::tempdir().unwrap();
    let own = dir.path().join("own.txt");
    std::fs::write(&own, "own1\nown2\nown3\n").unwrap();
    let mut test = EditorTest::new("");
    test.load_file(own.to_str().unwrap());
    test.keys("x");

    assert_success(run(&mut test, "2,3w >>"));
    assert_eq!(
        std::fs::read_to_string(&own).unwrap(),
        "own1\nown2\nown3\nown2\nown3\n"
    );
    assert!(test.editor.is_modified());

    let mut unnamed = EditorTest::new("text\n");
    let message = error_message(run(&mut unnamed, "w >>"));
    assert!(message.starts_with("E32:"), "{message}");
}

// nvim --clean: `:w existing.txt` in an unnamed buffer is E13; `:w!` writes it
// and names the buffer.
#[tokio::test(flavor = "multi_thread")]
async fn write_of_an_unnamed_buffer_to_an_existing_file_needs_bang() {
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("existing.txt");
    std::fs::write(&existing, "old\n").unwrap();
    let mut test = EditorTest::new("l1\nl2\nl3\n");

    let message = error_message(run(&mut test, &format!("w {}", existing.display())));
    assert!(message.starts_with("E13:"), "{message}");
    assert_eq!(std::fs::read_to_string(&existing).unwrap(), "old\n");

    assert_success(run(&mut test, &format!("w! {}", existing.display())));
    assert_eq!(std::fs::read_to_string(&existing).unwrap(), "l1\nl2\nl3\n");
    assert!(current_file(&test).ends_with("existing.txt"));
}
