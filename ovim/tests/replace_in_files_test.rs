//! Replace in files (`:SearchReplace`, the review panel) and the quickfix
//! `:grep` / `:cdo` / `:cfdo` route.

mod helpers;

use helpers::EditorTest;
use ovim_core::{KeyCode, Mode, Modifiers};
use std::fs;
use std::path::PathBuf;

struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Project {
    fn new(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir(root.join(".git")).unwrap();
        for (name, content) in files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        Self { _dir: dir, root }
    }

    fn path(&self, name: &str) -> String {
        self.root.join(name).to_string_lossy().to_string()
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.root.join(name)).unwrap()
    }
}

fn editor_in(project: &Project, open: &str) -> EditorTest {
    let mut test = EditorTest::new("");
    test.load_file(&project.path(open));
    test
}

/// Applies the review the way a user driving ex commands would: leave the
/// panel (its state is kept), then `:ReplaceApply`.
fn apply(test: &mut EditorTest) {
    test.press_esc();
    test.command("ReplaceApply");
}

fn status(test: &EditorTest) -> String {
    test.editor.status_message().to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn replace_in_files_edits_open_and_unopened_files_saves_and_undoes_per_buffer() {
    let project = Project::new(&[
        ("src/A.java", "class Circle { Circle c; }\n"),
        ("src/B.java", "new Circle();\nnew Circle();\n"),
        ("docs/notes.txt", "Circle notes\n"),
        ("src/keep.txt", "square\n"),
    ]);
    let mut test = editor_in(&project, "src/A.java");
    test.command("SearchReplace /Circle/Round/c");
    assert!(
        status(&test).contains("5 matches in 3 files"),
        "{}",
        status(&test)
    );
    // The case-sensitive search must not have matched nothing in keep.txt.
    apply(&mut test);
    assert!(
        status(&test).starts_with("Replaced 5 matches in 3 files"),
        "{}",
        status(&test)
    );

    // Open buffer, and two files that were never opened, are written to disk.
    assert_eq!(project.read("src/A.java"), "class Round { Round c; }\n");
    assert_eq!(project.read("src/B.java"), "new Round();\nnew Round();\n");
    assert_eq!(project.read("docs/notes.txt"), "Round notes\n");
    assert_eq!(project.read("src/keep.txt"), "square\n");
    assert_eq!(test.buffer_content(), "class Round { Round c; }\n");

    // One undo step in the open buffer restores it...
    test.keys("u");
    assert_eq!(test.buffer_content(), "class Circle { Circle c; }\n");

    // ...and :ReplaceUndo reverts every touched buffer, unopened ones included,
    // and writes the restored text back.
    test.command("ReplaceUndo");
    assert_eq!(project.read("src/B.java"), "new Circle();\nnew Circle();\n");
    assert_eq!(project.read("docs/notes.txt"), "Circle notes\n");
    // The open buffer was already undone with `u`; it is not undone twice.
    assert!(status(&test).contains("left 1 alone"), "{}", status(&test));
    assert_eq!(test.buffer_content(), "class Circle { Circle c; }\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn replace_undo_never_undoes_edits_made_after_the_replace() {
    let project = Project::new(&[("a.txt", "foo\nkeep\n")]);
    let mut test = editor_in(&project, "a.txt");
    test.command("SearchReplace /foo/bar/");
    apply(&mut test);
    test.keys("Gonew<Esc>");
    test.command("ReplaceUndo");
    assert!(status(&test).contains("left 1 alone"), "{}", status(&test));
    assert_eq!(
        test.buffer_content(),
        "bar\nkeep\nnew\n",
        "the later edit survives"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn unchecked_matches_are_left_alone_and_survive_a_new_search() {
    let project = Project::new(&[("a.txt", "x1\nx2\nx3\n")]);
    let mut test = editor_in(&project, "a.txt");
    test.command("SearchReplace /x/y/");
    // Rows: file header, x1, x2, x3. Select the second match and uncheck it.
    {
        let panel = test.editor.search_replace_panel_mut().unwrap();
        panel.selected = 2;
    }
    test.press_with(KeyCode::Char('t'), Modifiers::CONTROL);
    assert_eq!(test.mode(), Mode::SearchReplace);
    test.editor.run_search_replace_now();
    let panel = test.editor.search_replace_panel().unwrap();
    assert_eq!(
        panel.checked_matches(),
        2,
        "the choice survives a re-search"
    );
    apply(&mut test);
    assert_eq!(project.read("a.txt"), "y1\nx2\ny3\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn regex_mode_expands_capture_groups_and_literal_mode_does_not() {
    let project = Project::new(&[("a.txt", "get_name get_id\n")]);
    let mut test = editor_in(&project, "a.txt");
    test.command(r"SearchReplace /get_(\w+)/$1_of/r");
    apply(&mut test);
    assert_eq!(project.read("a.txt"), "name_of id_of\n");

    fs::write(project.root.join("a.txt"), "cost $5\n").unwrap();
    let mut test = editor_in(&project, "a.txt");
    test.command("SearchReplace /$5/$1/");
    apply(&mut test);
    assert_eq!(project.read("a.txt"), "cost $1\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn search_sees_unsaved_buffer_text_and_apply_saves_it() {
    let project = Project::new(&[("a.txt", "old\n")]);
    let mut test = editor_in(&project, "a.txt");
    test.keys("Gonew line<Esc>");
    test.command("SearchReplace /new/fresh/");
    assert!(
        status(&test).contains("1 matches in 1 files"),
        "{}",
        status(&test)
    );
    apply(&mut test);
    assert_eq!(project.read("a.txt"), "old\nfresh line\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn stale_lines_are_skipped_not_corrupted() {
    let project = Project::new(&[("a.txt", "foo one\nfoo two\n")]);
    let mut test = editor_in(&project, "a.txt");
    test.command("SearchReplace /foo/bar/");
    // The file changes behind the review's back.
    fs::write(project.root.join("a.txt"), "foo one\nchanged\n").unwrap();
    test.editor.reload_background_buffers_changed_on_disk();
    let _ = test.editor.buffer_mut().reload_from_disk();
    apply(&mut test);
    assert!(
        status(&test).contains("skipped 1 stale"),
        "{}",
        status(&test)
    );
    assert_eq!(project.read("a.txt"), "bar one\nchanged\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn file_globs_include_and_exclude() {
    let project = Project::new(&[
        ("a.java", "needle\n"),
        ("b.txt", "needle\n"),
        ("build/c.java", "needle\n"),
    ]);
    let mut test = editor_in(&project, "a.java");
    test.command("SearchReplace /needle/thread/ *.java !build");
    apply(&mut test);
    assert_eq!(project.read("a.java"), "thread\n");
    assert_eq!(project.read("b.txt"), "needle\n");
    assert_eq!(project.read("build/c.java"), "needle\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn keyboard_driven_review_types_a_query_toggles_and_applies() {
    let project = Project::new(&[("a.txt", "alpha\nalpha\n"), ("b.txt", "alpha\n")]);
    let mut test = editor_in(&project, "a.txt");
    test.command("SearchReplace");
    assert_eq!(test.mode(), Mode::SearchReplace);
    test.type_text("alpha");
    test.press_key(KeyCode::Tab);
    test.type_text("omega");
    test.editor.run_search_replace_now();
    assert_eq!(
        test.editor.search_replace_panel().unwrap().total_matches(),
        3
    );
    // Alt-Enter applies from any field.
    test.press_with(KeyCode::Enter, Modifiers::ALT);
    assert_eq!(project.read("a.txt"), "omega\nomega\n");
    assert_eq!(project.read("b.txt"), "omega\n");
    assert_eq!(test.mode(), Mode::Normal);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn grep_fills_quickfix_and_cdo_runs_a_command_per_entry() {
    let project = Project::new(&[("a.txt", "foo 1\nbar\nfoo 2\n"), ("b.txt", "foo 3\n")]);
    let mut test = editor_in(&project, "a.txt");
    test.command("grep foo");
    assert_eq!(test.editor.quickfix_list().len(), 3);
    test.command("cdo s/foo/baz/ | update");
    assert_eq!(project.read("a.txt"), "baz 1\nbar\nbaz 2\n");
    assert_eq!(project.read("b.txt"), "baz 3\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn cfdo_runs_once_per_file() {
    let project = Project::new(&[("a.txt", "foo\nfoo\n"), ("b.txt", "foo\n")]);
    let mut test = editor_in(&project, "a.txt");
    test.command("grep foo");
    test.command("cfdo %s/foo/bar/g | update");
    assert_eq!(project.read("a.txt"), "bar\nbar\n");
    assert_eq!(project.read("b.txt"), "bar\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn cdo_stops_at_the_first_error() {
    let project = Project::new(&[("a.txt", "foo\nfoo\n"), ("b.txt", "foo\n")]);
    let mut test = editor_in(&project, "a.txt");
    test.command("grep foo");
    // The pattern is absent on the entry line after the first substitution
    // has run for that line, so the second entry on the same line fails.
    test.command("cdo s/nomatch/x/");
    assert!(status(&test).contains("E486"), "{}", status(&test));
    assert!(
        status(&test).contains("stopped at entry 1"),
        "{}",
        status(&test)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn update_writes_only_when_modified() {
    let project = Project::new(&[("a.txt", "one\n")]);
    let mut test = editor_in(&project, "a.txt");
    test.keys("xu");
    test.keys("ix<Esc>");
    test.command("update");
    assert_eq!(project.read("a.txt"), "xone\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn substitute_chains_with_a_following_command_and_reports_e486() {
    let mut test = EditorTest::new("foo 1\nbar\n");
    test.command("s/foo/baz/ | s/1/2/");
    assert_eq!(test.buffer_content(), "baz 2\nbar\n");
    // A bar inside the pattern stays part of the pattern.
    test.command("s/x|baz/q/");
    assert_eq!(test.buffer_content(), "q 2\nbar\n");
    test.command("s/nomatch/x/");
    assert!(status(&test).starts_with("E486"), "{}", status(&test));
    test.editor.set_status_message(String::new());
    test.command("s/nomatch/x/e");
    assert!(!status(&test).starts_with("E486"), "{}", status(&test));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn crlf_files_keep_their_line_endings() {
    let project = Project::new(&[("win.txt", "foo\r\nbar foo\r\n")]);
    let mut test = editor_in(&project, "win.txt");
    test.command("SearchReplace /foo/x/");
    apply(&mut test);
    assert_eq!(project.read("win.txt"), "x\r\nbar x\r\n");
}

// nvim --clean, a.rs "foo one / bar / foo two", c.rs "foo rs2", b.txt "foo
// three": `:vimgrep /foo/ *.rs` lists the three matches in the .rs files and
// `:vimgrep /foo/ *.rs *.txt` all four.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn vimgrep_takes_the_files_after_the_closing_delimiter() {
    let project = Project::new(&[
        ("a.rs", "foo one\nbar\nfoo two\n"),
        ("b.txt", "foo three\n"),
        ("c.rs", "foo rs2\n"),
    ]);
    let mut test = editor_in(&project, "a.rs");

    test.command("vimgrep /foo/ *.rs");
    assert_eq!(test.editor.quickfix_list().len(), 3, "{}", status(&test));

    test.command("vimgrep /foo/ *.rs *.txt");
    assert_eq!(test.editor.quickfix_list().len(), 4, "{}", status(&test));

    // The files are matched, not the text after the pattern.
    test.command("vimgrep /nomatch/ *.rs");
    assert!(
        status(&test).starts_with("E486: Pattern not found: nomatch"),
        "{}",
        status(&test)
    );
}

// nvim --clean: flags after the closing delimiter (`g`, `j`) are not files,
// and `j` leaves the cursor where it is.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn vimgrep_flags_follow_the_closing_delimiter() {
    let project = Project::new(&[
        ("a.rs", "foo one\nbar\nfoo two\n"),
        ("b.txt", "foo three\n"),
    ]);
    let mut test = editor_in(&project, "a.rs");
    test.keys("G");

    test.command("vimgrep /foo/gj *.rs");
    assert_eq!(test.editor.quickfix_list().len(), 2, "{}", status(&test));
    assert_eq!(test.cursor().0, 2, "j does not jump to the first match");

    test.command("vimgrep /foo/g");
    assert_eq!(test.editor.quickfix_list().len(), 3, "{}", status(&test));
}

// A pattern that merely starts with a slash, and one without a delimiter,
// stay literal text.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn grep_without_a_closing_delimiter_searches_the_text_as_typed() {
    let project = Project::new(&[("a.txt", "see /usr/bin now\nfoo bar\n")]);
    let mut test = editor_in(&project, "a.txt");

    test.command("grep /usr/bin");
    assert_eq!(test.editor.quickfix_list().len(), 1, "{}", status(&test));

    test.command("grep foo bar");
    assert_eq!(test.editor.quickfix_list().len(), 1, "{}", status(&test));

    test.command("grep /foo/ -- *.txt");
    assert_eq!(test.editor.quickfix_list().len(), 1, "{}", status(&test));
}
