//! Git from the editor: stage/unstage, commit and amend through the message
//! buffer, the status list opening diffs, line history and merge conflicts.

mod helpers;

use git2::{IndexAddOption, Repository, Signature};
use helpers::EditorTest;
use ovim_core::Mode;
use std::fs;
use std::path::PathBuf;

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    repo: Repository,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let repo = Repository::init(&root).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test").unwrap();
        config.set_str("user.email", "t@example.com").unwrap();
        // Commits run through the git binary: keep the developer's global
        // signing and hook configuration out of the tests.
        config.set_bool("commit.gpgsign", false).unwrap();
        config
            .set_str("core.hooksPath", root.join(".git/hooks").to_str().unwrap())
            .unwrap();
        Self {
            _dir: dir,
            root,
            repo,
        }
    }

    fn write(&self, name: &str, content: &str) -> String {
        let path = self.root.join(name);
        fs::write(&path, content).unwrap();
        path.to_string_lossy().to_string()
    }

    fn commit_all(&self, message: &str) {
        let mut index = self.repo.index().unwrap();
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = self.repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = Signature::now("Test", "t@example.com").unwrap();
        let parent = self.repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        self.repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parents,
            )
            .unwrap();
    }

    fn head_message(&self) -> String {
        self.repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .message()
            .unwrap()
            .to_string()
    }

    fn staged(&self, name: &str) -> String {
        let mut index = self.repo.index().unwrap();
        index.read(true).unwrap();
        let entry = index.get_path(std::path::Path::new(name), 0).unwrap();
        String::from_utf8(self.repo.find_blob(entry.id).unwrap().content().to_vec()).unwrap()
    }
}

/// Waits for the background commit started by writing the message buffer.
fn settle_commit(test: &mut EditorTest) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while test.editor.git_commit_pending() {
        test.editor.poll_git_commit();
        assert!(
            std::time::Instant::now() < deadline,
            "the commit did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// Waits for the background history lookup to open its picker.
fn settle_history(test: &mut EditorTest) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while test.editor.git_history_pending() {
        test.editor.poll_git_history();
        assert!(
            std::time::Instant::now() < deadline,
            "the history lookup did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

const TEN: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn stage_hunk_commit_and_amend_through_the_leader_keys_and_the_message_buffer() {
    let fixture = Fixture::new();
    let file = fixture.write("a.txt", TEN);
    fixture.commit_all("init");
    fixture.write("a.txt", "l1\nL2\nl3\nl4\nl5\nl6\nl7\nl8\nL9\nl10\n");

    let mut test = EditorTest::new("");
    test.load_file(&file);
    // Stage only the change on line 9.
    test.keys("9G");
    test.keys(" gs");
    assert_eq!(
        fixture.staged("a.txt"),
        "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nL9\nl10\n"
    );

    // <Space>gc opens the message buffer in insert mode; write to commit.
    test.keys(" gc");
    test.assert_mode(Mode::Insert);
    assert!(test.editor.is_commit_message_buffer());
    test.type_text("Change line nine");
    test.press_esc();
    test.command("w");
    settle_commit(&mut test);
    assert!(!test.editor.is_commit_message_buffer());
    assert_eq!(fixture.head_message(), "Change line nine\n");
    assert!(
        test.editor.buffer().file_path().unwrap().ends_with("a.txt"),
        "back in the file"
    );

    // <Space>gC amends: the previous message is the starting text.
    test.keys("2G");
    test.keys(" gS");
    test.keys(" gC");
    assert!(test
        .editor
        .buffer()
        .rope()
        .to_string()
        .starts_with("Change line nine"));
    test.keys("ggA, and two<Esc>");
    test.keys("ZZ");
    settle_commit(&mut test);
    assert_eq!(fixture.head_message(), "Change line nine, and two\n");
    assert_eq!(
        fixture
            .repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .parent_count(),
        1
    );
}

#[cfg(unix)]
fn write_hook(fixture: &Fixture, name: &str, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let hooks = fixture.root.join(".git/hooks");
    fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join(name);
    fs::write(&hook, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    hook
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_failing_hook_keeps_the_message_buffer_open_and_shows_its_output() {
    let fixture = Fixture::new();
    let file = fixture.write("a.txt", "one\n");
    fixture.commit_all("init");
    fixture.write("a.txt", "two\n");
    let hook = write_hook(
        &fixture,
        "pre-commit",
        "sleep 0.3\necho 'lint: 3 problems' >&2\nexit 1",
    );
    let mut test = EditorTest::new("");
    test.load_file(&file);
    test.command("GitStage");
    test.command("GitCommit");
    test.type_text("Needs lint");
    test.press_esc();
    test.command("w");
    assert!(
        test.editor.git_commit_pending(),
        "the editor does not wait for the hook"
    );
    assert_eq!(test.editor.status_message(), "Committing…");

    settle_commit(&mut test);
    assert!(test.editor.is_commit_message_buffer(), "still open");
    assert!(
        test.editor.status_message().contains("lint: 3 problems"),
        "{}",
        test.editor.status_message()
    );
    assert_eq!(fixture.head_message(), "init", "the hook prevented it");
    assert!(test.buffer_content().starts_with("Needs lint"));

    fs::remove_file(hook).unwrap();
    test.command("w");
    settle_commit(&mut test);
    assert!(!test.editor.is_commit_message_buffer());
    assert_eq!(fixture.head_message(), "Needs lint\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn git_commands_run_from_the_message_buffer_do_not_commit_the_message() {
    let fixture = Fixture::new();
    let file = fixture.write("a.txt", "one\n");
    fixture.write("b.txt", "b\n");
    fixture.commit_all("init");
    fixture.write("a.txt", "two\n");
    fixture.write("b.txt", "b changed\n");
    let mut test = EditorTest::new("");
    test.load_file(&file);
    test.command("GitStage");
    test.command("GitCommit");
    test.type_text("half written");
    test.press_esc();

    test.command("GitStageAll");
    settle_commit(&mut test);
    assert_eq!(fixture.head_message(), "init", "nothing was committed");
    assert!(test.editor.is_commit_message_buffer(), "message kept");
    assert!(test.buffer_content().starts_with("half written"));
    assert_eq!(
        fixture.staged("b.txt"),
        "b changed\n",
        "the staging itself still happened"
    );

    // Commands about the current file have no file to act on here.
    test.command("GitStage");
    assert!(
        test.editor
            .status_message()
            .contains("commit message buffer"),
        "{}",
        test.editor.status_message()
    );
    assert_eq!(fixture.head_message(), "init");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_merge_commit_starts_from_the_prepared_message_and_cannot_be_amended() {
    let fixture = Fixture::new();
    let file = fixture.write("a.txt", "one\n");
    fixture.commit_all("first");
    fixture.write("a.txt", "two\n");
    fixture.commit_all("second");
    let first = fixture
        .repo
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .parent_id(0)
        .unwrap();
    let git_dir = fixture.repo.path();
    fs::write(git_dir.join("MERGE_HEAD"), format!("{first}\n")).unwrap();
    fs::write(git_dir.join("MERGE_MSG"), "Merge branch 'topic'\n").unwrap();

    let mut test = EditorTest::new("");
    test.load_file(&file);
    test.command("GitAmend");
    assert!(!test.editor.is_commit_message_buffer());
    assert!(
        test.editor
            .status_message()
            .contains("middle of a merge -- cannot amend"),
        "{}",
        test.editor.status_message()
    );

    test.command("GitCommit");
    assert!(test.editor.is_commit_message_buffer());
    assert!(
        test.buffer_content().starts_with("Merge branch 'topic'"),
        "{}",
        test.buffer_content()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn quitting_the_message_buffer_aborts_and_refuses_when_modified() {
    let fixture = Fixture::new();
    let file = fixture.write("a.txt", "one\n");
    fixture.commit_all("init");
    fixture.write("a.txt", "two\n");
    let mut test = EditorTest::new("");
    test.load_file(&file);
    test.command("GitStage");
    test.command("GitCommit");
    test.type_text("draft");
    test.press_esc();
    test.command("q");
    assert!(
        test.editor.is_commit_message_buffer(),
        ":q refuses with unsaved text"
    );
    test.command("q!");
    assert!(!test.editor.is_commit_message_buffer());
    assert_eq!(fixture.head_message(), "init");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn status_list_enter_opens_the_diff_review_on_that_file() {
    let fixture = Fixture::new();
    fixture.write("a.txt", "one\n");
    fixture.write("b.txt", "one\n");
    fixture.commit_all("init");
    fixture.write("a.txt", "one changed\n");
    fixture.write("b.txt", "one also changed\n");
    let file = fixture.root.join("a.txt").to_string_lossy().to_string();

    let mut test = EditorTest::new("");
    test.load_file(&file);
    test.keys(" gg");
    test.assert_mode(Mode::Picker);
    // Second entry (b.txt), Enter.
    test.keys("<C-n>");
    test.press_enter();
    test.assert_mode(Mode::Normal);
    assert!(test.editor.is_diff_review_buffer(), "the review is showing");
    let line = test.editor.buffer().cursor().line();
    let header = test.editor.buffer().line_text(line).unwrap().to_string();
    assert!(
        header.contains("b.txt"),
        "cursor is on the b.txt section: {header:?}"
    );
}

/// A file name is data, not ex syntax: `|` must not chain a second command
/// when Enter is pressed on a status or history row.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn picker_rows_with_ex_syntax_in_the_file_name_open_that_file_only() {
    let fixture = Fixture::new();
    let name = "n.txt|r !touch PWNED";
    let file = fixture.write(name, "zero\n");
    fixture.commit_all("init");
    // A commit with a parent, so its history row opens in the review.
    fixture.write(name, "one\n");
    fixture.commit_all("edit");
    fixture.write(name, "changed\n");
    let pwned = std::env::current_dir().unwrap().join("PWNED");
    let _ = fs::remove_file(&pwned);

    let mut test = EditorTest::new("");
    test.load_file(&file);
    test.keys(" gg");
    test.assert_mode(Mode::Picker);
    test.press_enter();
    test.assert_mode(Mode::Normal);
    assert!(test.editor.is_diff_review_buffer(), "the review is showing");
    let line = test.editor.buffer().cursor().line();
    let header = test.editor.buffer().line_text(line).unwrap().to_string();
    assert!(header.contains(name), "cursor is on the file: {header:?}");

    test.command("GitLog");
    settle_history(&mut test);
    test.assert_mode(Mode::Picker);
    test.press_enter();
    assert!(test.editor.is_diff_review_buffer());
    assert!(
        test.editor.status_message().contains("^.."),
        "the commit is reviewed"
    );
    assert_eq!(
        test.editor.git_root().unwrap(),
        fs::canonicalize(&fixture.root).unwrap(),
        "the review is of the fixture repository, not the working directory's"
    );
    assert!(!pwned.exists(), "a shell command ran from a file name");
    assert!(!fixture.root.join("PWNED").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn status_list_enter_works_before_the_first_commit() {
    let fixture = Fixture::new();
    let file = fixture.write("a.txt", "brand new\n");
    let mut test = EditorTest::new("");
    test.load_file(&file);
    test.keys(" gg");
    test.assert_mode(Mode::Picker);
    test.press_enter();
    test.assert_mode(Mode::Normal);
    assert!(
        test.editor.is_diff_review_buffer(),
        "{}",
        test.editor.status_message()
    );
    assert!(test.buffer_content().contains("brand new"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn opening_a_diff_from_another_repository_replaces_the_open_review() {
    let first = Fixture::new();
    let first_file = first.write("first.txt", "one\n");
    first.commit_all("init");
    first.write("first.txt", "one changed\n");
    let second = Fixture::new();
    let second_file = second.write("second.txt", "two\n");
    second.commit_all("init");
    second.write("second.txt", "two changed\n");

    let mut test = EditorTest::new("");
    test.load_file(&first_file);
    test.keys(" gg");
    test.press_enter();
    assert!(test.editor.is_diff_review_buffer());
    assert!(test.buffer_content().contains("first.txt"));

    // Leaving keeps the review around; the next file belongs to another
    // repository.
    test.keys(" gd");
    assert!(!test.editor.is_diff_review_buffer());
    test.load_file(&second_file);
    test.keys(" gg");
    test.assert_mode(Mode::Picker);
    test.press_enter();
    assert!(test.editor.is_diff_review_buffer());
    let review = test.buffer_content();
    assert!(review.contains("second.txt"), "{review}");
    assert!(!review.contains("first.txt"), "{review}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn viewing_a_commit_works_when_the_open_file_and_its_directory_are_gone() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.root.join("nested")).unwrap();
    let file = fixture.write("nested/a.txt", "one\n");
    fixture.write("b.txt", "b\n");
    fixture.commit_all("first");
    fixture.write("b.txt", "b changed\n");
    fixture.commit_all("second");

    let mut test = EditorTest::new("");
    test.load_file(&file);
    fs::remove_dir_all(fixture.root.join("nested")).unwrap();
    test.command("GitLogAll");
    settle_history(&mut test);
    test.assert_mode(Mode::Picker);
    test.press_enter();
    assert!(
        test.editor.is_diff_review_buffer(),
        "{}",
        test.editor.status_message()
    );
    assert!(test.buffer_content().contains("b changed"));
}

/// Runs `git` in `dir` with identity and signing pinned.
fn run_git(dir: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "protocol.file.allow=always",
            "-c",
            "user.name=Test",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn submodule_rows_in_the_status_list_stage_unstage_and_open_the_diff() {
    let dir = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(dir.path()).unwrap();
    let inner = base.join("inner");
    let outer = base.join("outer");
    for repo in [&inner, &outer] {
        fs::create_dir(repo).unwrap();
        run_git(repo, &["init", "-q"]);
    }
    fs::write(inner.join("lib.txt"), "lib\n").unwrap();
    run_git(&inner, &["add", "-A"]);
    run_git(&inner, &["commit", "-qm", "inner"]);
    fs::write(outer.join("main.txt"), "main\n").unwrap();
    run_git(
        &outer,
        &["submodule", "add", "-q", inner.to_str().unwrap(), "sub"],
    );
    run_git(&outer, &["add", "-A"]);
    run_git(&outer, &["commit", "-qm", "outer"]);
    // A new commit inside the submodule: the superproject sees `sub` modified.
    run_git(
        &outer.join("sub"),
        &["commit", "-q", "--allow-empty", "-m", "newer"],
    );

    let mut test = EditorTest::new("");
    test.load_file(&outer.join("main.txt").to_string_lossy());
    test.command("GitStatus");
    test.assert_mode(Mode::Picker);
    let rows = |test: &EditorTest| -> Vec<String> {
        test.editor
            .picker()
            .unwrap()
            .collect_filtered_results(10)
            .into_iter()
            .map(|r| r.display.clone())
            .collect()
    };
    assert_eq!(rows(&test), [" M  sub"]);

    test.keys("<C-t>");
    assert_eq!(rows(&test), ["M   sub"], "Ctrl-T stages the new commit");
    test.keys("<C-t>");
    assert_eq!(rows(&test), [" M  sub"], "and unstages it again");

    test.press_enter();
    test.assert_mode(Mode::Normal);
    assert!(
        test.editor.is_diff_review_buffer(),
        "{}",
        test.editor.status_message()
    );
    assert!(test.buffer_content().contains("sub"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn history_that_arrives_during_insert_waits_for_normal_mode() {
    let fixture = Fixture::new();
    let file = fixture.write("a.txt", "one\n");
    fixture.commit_all("first");
    let mut test = EditorTest::new("");
    test.load_file(&file);

    test.command("GitLog");
    test.keys("i");
    // Long enough for the one-commit lookup to finish in the background.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
    while std::time::Instant::now() < deadline {
        test.editor.poll_git_history();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    test.assert_mode(Mode::Insert);
    test.keys("x<Esc>");
    assert_eq!(test.buffer_content(), "xone\n");

    settle_history(&mut test);
    test.assert_mode(Mode::Picker);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn history_is_looked_up_in_the_background_and_opens_when_ready() {
    let fixture = Fixture::new();
    let file = fixture.write("a.txt", "one\n");
    fixture.commit_all("first");
    let mut test = EditorTest::new("");
    test.load_file(&file);

    test.command("GitLog");
    assert!(test.editor.git_history_pending());
    assert_eq!(test.editor.status_message(), "Loading history…");
    test.assert_mode(Mode::Normal);

    settle_history(&mut test);
    test.assert_mode(Mode::Picker);
    assert_eq!(
        test.editor.picker().unwrap().title(),
        Some("History of a.txt")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn line_history_lists_the_commits_and_enter_shows_the_diff_of_one() {
    let fixture = Fixture::new();
    let file = fixture.write("a.txt", "alpha\nbeta\ngamma\n");
    fixture.commit_all("create");
    fixture.write("a.txt", "alpha\nbeta v2\ngamma\n");
    fixture.commit_all("edit beta");
    fixture.write("a.txt", "alpha\nbeta v2\ngamma\ndelta\n");
    fixture.commit_all("add delta");

    let mut test = EditorTest::new("");
    test.load_file(&file);
    test.keys("2G");
    test.keys(" gL");
    settle_history(&mut test);
    test.assert_mode(Mode::Picker);
    let picker = test.editor.picker().unwrap();
    assert_eq!(picker.title(), Some("History of line 2"));
    let rows: Vec<String> = picker
        .collect_filtered_results(10)
        .into_iter()
        .map(|r| r.display.clone())
        .collect();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows[0].ends_with("Test  edit beta"), "{rows:?}");
    assert!(rows[1].ends_with("Test  create"), "{rows:?}");

    // Enter on the newest opens that commit's diff.
    test.press_enter();
    assert!(test.editor.is_diff_review_buffer());
    let text = test.editor.buffer().rope().to_string();
    assert!(text.contains("beta v2"), "{text}");
    assert!(
        !text.contains("delta"),
        "only that commit's changes: {text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn file_history_and_root_commits_are_shown() {
    let fixture = Fixture::new();
    let file = fixture.write("a.txt", "one\n");
    fixture.commit_all("first");
    let mut test = EditorTest::new("");
    test.load_file(&file);
    test.command("GitLog");
    settle_history(&mut test);
    test.assert_mode(Mode::Picker);
    test.press_enter();
    // A root commit has no parent to review against: shown as a plain diff.
    assert!(test.editor.buffer().rope().to_string().contains("+one"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn conflict_markers_navigate_and_resolve_with_the_leader_keys() {
    let fixture = Fixture::new();
    let file = fixture.write(
        "c.txt",
        "top\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> topic\nmid\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> topic\nend\n",
    );
    let mut test = EditorTest::new("");
    test.load_file(&file);
    test.keys("]n");
    test.assert_cursor(1, 0);
    test.keys(" gmo");
    assert_eq!(
        test.buffer_content(),
        "top\nours\nmid\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> topic\nend\n"
    );
    test.keys("[n");
    test.assert_cursor(3, 0);
    test.keys(" gmt");
    assert_eq!(test.buffer_content(), "top\nours\nmid\ny\nend\n");
    test.keys("]n");
    assert!(test.editor.status_message().contains("No merge conflicts"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn git_commands_report_problems_instead_of_failing_silently() {
    let fixture = Fixture::new();
    let file = fixture.write("a.txt", "one\n");
    fixture.commit_all("init");
    let mut test = EditorTest::new("");
    test.load_file(&file);
    test.command("GitCommit");
    assert!(
        test.editor.status_message().contains("Nothing to commit"),
        "{}",
        test.editor.status_message()
    );
    test.command("GitStageHunk");
    assert!(
        test.editor.status_message().contains("No unstaged change"),
        "{}",
        test.editor.status_message()
    );
    test.command("GitStatus");
    assert!(
        test.editor.status_message().contains("clean"),
        "{}",
        test.editor.status_message()
    );

    let outside = tempfile::tempdir().unwrap();
    let plain = outside.path().join("x.txt");
    fs::write(&plain, "x\n").unwrap();
    let mut test = EditorTest::new("");
    test.load_file(&plain.to_string_lossy());
    test.command("GitStatus");
    assert!(
        test.editor.status_message().starts_with("Git:"),
        "{}",
        test.editor.status_message()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn status_list_keys_stage_unstage_and_edit_and_wq_commits() {
    let fixture = Fixture::new();
    let a = fixture.write("a.txt", "one\n");
    fixture.write("b.txt", "b\n");
    fixture.commit_all("init");
    fixture.write("a.txt", "two\n");
    let mut test = EditorTest::new("");
    test.load_file(&a);

    test.command("GitStatus");
    test.assert_mode(Mode::Picker);
    test.keys("<C-t>");
    assert_eq!(
        fixture.staged("a.txt"),
        "two\n",
        "Ctrl-T stages the selected file"
    );
    test.keys("<C-t>");
    assert_eq!(fixture.staged("a.txt"), "one\n", "and unstages it again");
    test.keys("<C-e>");
    test.assert_mode(Mode::Normal);
    assert!(test.editor.buffer().file_path().unwrap().ends_with("a.txt"));

    // :wq also commits from the message buffer.
    test.command("GitStage");
    test.command("GitCommit");
    test.type_text("Via wq");
    test.press_esc();
    test.command("wq");
    settle_commit(&mut test);
    assert_eq!(fixture.head_message(), "Via wq\n");
    assert!(
        !test.editor.is_commit_message_buffer(),
        ":wq leaves the message buffer once the commit succeeded"
    );
}

fn assert_signs(before: &str, after: &str, expected: &[(usize, ovim_core::git::LineStatus)]) {
    let fixture = Fixture::new();
    let file = fixture.write("a.txt", before);
    fixture.commit_all("base");
    fixture.write("a.txt", after);
    let status = ovim_core::git::GitStatus::from_file(&file).unwrap();
    let actual: Vec<_> = (0..before.lines().count().max(after.lines().count()) + 2)
        .filter_map(|line| status.get_line_status(line).map(|kind| (line, kind)))
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn signs_classify_a_replaced_line_as_modified() {
    use ovim_core::git::LineStatus::Modified;
    assert_signs("one\ntwo\nthree\n", "one\nTWO\nthree\n", &[(1, Modified)]);
}

#[test]
fn signs_anchor_end_deletions_to_a_surviving_line() {
    use ovim_core::git::LineStatus::Removed;
    assert_signs("one\ntwo\nthree\n", "one\n", &[(0, Removed)]);
}

#[test]
fn signs_use_worktree_coordinates_after_an_earlier_insertion() {
    use ovim_core::git::LineStatus::{Added, Removed};
    assert_signs(
        "one\ntwo\nthree\nfour\nfive\n",
        "new\none\ntwo\nthree\nfive\n",
        &[(0, Added), (3, Removed)],
    );
}

#[test]
fn signs_cover_start_middle_empty_and_unequal_replacements() {
    use ovim_core::git::LineStatus::{Added, Modified, Removed};
    assert_signs("a\nb\nc\n", "b\nc\n", &[(0, Removed)]);
    assert_signs("a\nb\nc\n", "a\nc\n", &[(0, Removed)]);
    assert_signs("a\nb\n", "", &[(0, Removed)]);
    assert_signs(
        "a\nb\nc\n",
        "a\nB\nextra\nc\n",
        &[(1, Modified), (2, Added)],
    );
    assert_signs("a\nb\nc\nd\n", "a\nB\nd\n", &[(1, Modified)]);
    assert_signs(
        "a\nb\nc\nd\ne\nf\n",
        "x\ny\nz\na\nb\nc\ne\nf\n",
        &[(0, Added), (1, Added), (2, Added), (5, Removed)],
    );
}

#[test]
fn signs_compare_head_with_combined_staged_and_unstaged_changes() {
    use ovim_core::git::{
        GitStatus,
        LineStatus::{Added, Modified},
    };
    let fixture = Fixture::new();
    let file = fixture.write("a.txt", "one\ntwo\nthree\n");
    fixture.commit_all("base");
    fixture.write("a.txt", "ONE\ntwo\nthree\n");
    ovim_core::git::ops::stage_file(std::path::Path::new(&file)).unwrap();
    fixture.write("a.txt", "ONE\ntwo\nthree\nfour\n");
    let status = GitStatus::from_file(&file).unwrap();
    assert_eq!(status.get_line_status(0), Some(Modified));
    assert_eq!(status.get_line_status(3), Some(Added));
    assert_eq!(status.hunk_starts(), vec![0, 3]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn deletion_sign_navigation_lands_on_a_visible_surviving_line() {
    let fixture = Fixture::new();
    let file = fixture.write("a.txt", "one\ntwo\nthree\nfour\n");
    fixture.commit_all("base");
    fixture.write("a.txt", "one\ntwo\n");
    let mut test = EditorTest::new("");
    test.load_file(&file);
    test.keys("]c");
    test.assert_cursor(1, 0);
    let rendered = ovim::ui::render_editor_to_ansi(&mut test.editor, 80, 16).unwrap();
    let plain = ovim::ui::strip_ansi(&rendered);
    let row = plain.lines().find(|row| row.contains("two")).unwrap();
    assert!(
        row.contains('-'),
        "deletion sign must render on the surviving row: {row}"
    );
    assert_eq!(
        test.editor.buffer().git_status().get_line_status(1),
        Some(ovim_core::git::LineStatus::Removed)
    );
}

#[test]
fn deleted_line_counts_are_not_lost_when_signs_share_an_anchor() {
    let fixture = Fixture::new();
    let file = fixture.write("a.txt", "one\ntwo\nthree\nfour\n");
    fixture.commit_all("base");
    fixture.write("a.txt", "ONE\n");
    let status = ovim_core::git::GitStatus::from_file(&file).unwrap();
    assert_eq!(status.change_counts(), (0, 1, 3));
    assert_eq!(
        status.get_line_status(0),
        Some(ovim_core::git::LineStatus::Modified)
    );
    assert_eq!(status.get_line_status(1), None);
}

#[test]
fn signs_include_untracked_files() {
    use ovim_core::git::LineStatus::Added;
    let fixture = Fixture::new();
    fixture.write("base.txt", "base\n");
    fixture.commit_all("base");
    let file = fixture.write("new.txt", "new\nfile\n");
    let status = ovim_core::git::GitStatus::from_file(&file).unwrap();
    assert_eq!(status.get_line_status(0), Some(Added));
    assert_eq!(status.get_line_status(1), Some(Added));
}

#[test]
fn signs_include_files_before_the_first_commit_and_ignore_ignored_files() {
    use ovim_core::git::{GitStatus, LineStatus::Added};
    let fixture = Fixture::new();
    let file = fixture.write("new.txt", "new\n");
    assert_eq!(
        GitStatus::from_file(&file).unwrap().get_line_status(0),
        Some(Added)
    );
    ovim_core::git::ops::stage_file(std::path::Path::new(&file)).unwrap();
    assert_eq!(
        GitStatus::from_file(&file).unwrap().get_line_status(0),
        Some(Added)
    );
    fixture.write(".gitignore", "ignored.txt\n");
    let ignored = fixture.write("ignored.txt", "ignored\n");
    assert_eq!(
        GitStatus::from_file(&ignored).unwrap().change_counts(),
        (0, 0, 0)
    );
}

#[test]
fn unstage_new_file_hunk_removes_the_index_entry() {
    let fixture = Fixture::new();
    fixture.write("base.txt", "base\n");
    fixture.commit_all("base");
    let file = fixture.write("new.txt", "new\n");
    let file = std::path::Path::new(&file);
    ovim_core::git::ops::stage_file(file).unwrap();
    assert!(ovim_core::git::ops::unstage_hunk(file, 0).unwrap());
    let statuses = ovim_core::git::ops::status(&fixture.root).unwrap();
    assert_eq!(statuses[0].code(), "??");
    assert_eq!(fs::read_to_string(file).unwrap(), "new\n");
}

#[test]
fn unstage_hunk_preserves_a_tracked_empty_file() {
    let fixture = Fixture::new();
    let file = fixture.write("empty.txt", "");
    fixture.commit_all("base");
    fixture.write("empty.txt", "added\n");
    let file = std::path::Path::new(&file);
    ovim_core::git::ops::stage_file(file).unwrap();
    assert!(ovim_core::git::ops::unstage_hunk(file, 0).unwrap());
    assert_eq!(fixture.staged("empty.txt"), "");
    assert_eq!(
        ovim_core::git::ops::status(&fixture.root).unwrap()[0].code(),
        " M"
    );
}

#[test]
fn stage_and_unstage_a_file_in_a_deleted_directory() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.root.join("nested")).unwrap();
    let file = fixture.write("nested/a.txt", "original\n");
    fixture.commit_all("base");
    fs::remove_dir_all(fixture.root.join("nested")).unwrap();
    let path = std::path::Path::new(&file);
    ovim_core::git::ops::stage_file(path).unwrap();
    assert_eq!(
        ovim_core::git::ops::status(&fixture.root).unwrap()[0].code(),
        "D "
    );
    ovim_core::git::ops::unstage_file(path).unwrap();
    assert_eq!(fixture.staged("nested/a.txt"), "original\n");
    assert_eq!(
        ovim_core::git::ops::status(&fixture.root).unwrap()[0].code(),
        " D"
    );
}

#[test]
fn stage_hunk_treats_brackets_in_a_filename_literally() {
    let fixture = Fixture::new();
    fixture.write("a[1].txt", "original\nsecond\n");
    fixture.write("a1.txt", "other\n");
    fixture.commit_all("base");
    let file = fixture.write("a[1].txt", "changed\nSECOND\n");
    fixture.write("a1.txt", "OTHER\n");
    assert!(ovim_core::git::ops::stage_hunk(std::path::Path::new(&file), 0).unwrap());
    assert_eq!(fixture.staged("a[1].txt"), "changed\nSECOND\n");
    assert_eq!(fixture.staged("a1.txt"), "other\n");
}

#[test]
fn unstage_file_does_not_match_other_paths_as_a_glob() {
    let fixture = Fixture::new();
    let file = fixture.write("a[1].txt", "original\n");
    fixture.write("a1.txt", "other\n");
    fixture.commit_all("base");
    fixture.write("a[1].txt", "changed\n");
    fixture.write("a1.txt", "OTHER\n");
    ovim_core::git::ops::stage_all(&fixture.root).unwrap();
    ovim_core::git::ops::unstage_file(std::path::Path::new(&file)).unwrap();
    assert_eq!(fixture.staged("a[1].txt"), "original\n");
    assert_eq!(fixture.staged("a1.txt"), "OTHER\n");
}

#[test]
fn unstage_file_restores_deletions_and_executable_mode() {
    let fixture = Fixture::new();
    let file = fixture.write("run.sh", "echo old\n");
    fixture.commit_all("base");
    let mut index = fixture.repo.index().unwrap();
    let mut entry = index.get_path(std::path::Path::new("run.sh"), 0).unwrap();
    entry.mode = 0o100755;
    index.add(&entry).unwrap();
    index.write().unwrap();
    let tree = fixture.repo.find_tree(index.write_tree().unwrap()).unwrap();
    let parent = fixture.repo.head().unwrap().peel_to_commit().unwrap();
    let signature = Signature::now("Test", "t@example.com").unwrap();
    fixture
        .repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "executable",
            &tree,
            &[&parent],
        )
        .unwrap();
    fs::remove_file(&file).unwrap();
    let path = std::path::Path::new(&file);
    ovim_core::git::ops::stage_file(path).unwrap();
    ovim_core::git::ops::unstage_file(path).unwrap();
    index.read(true).unwrap();
    assert_eq!(
        index
            .get_path(std::path::Path::new("run.sh"), 0)
            .unwrap()
            .mode,
        0o100755
    );
    assert_eq!(fixture.staged("run.sh"), "echo old\n");
    assert!(!path.exists(), "unstage must not restore the working file");
}

#[test]
fn unstage_file_in_an_unborn_repository_removes_only_the_selected_path() {
    let fixture = Fixture::new();
    let file = fixture.write("a[1].txt", "new\n");
    fixture.write("a1.txt", "other\n");
    ovim_core::git::ops::stage_all(&fixture.root).unwrap();
    ovim_core::git::ops::unstage_file(std::path::Path::new(&file)).unwrap();
    assert_eq!(fixture.staged("a1.txt"), "other\n");
    assert!(fixture
        .repo
        .index()
        .unwrap()
        .get_path(std::path::Path::new("a[1].txt"), 0)
        .is_none());
    assert_eq!(fs::read_to_string(file).unwrap(), "new\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn opening_a_file_computes_signs_in_the_background() {
    let fixture = Fixture::new();
    let file = fixture.write("a.txt", "one\n");
    fixture.commit_all("base");
    fixture.write("a.txt", "ONE\n");
    let mut test = EditorTest::new("");
    test.editor.load_file(&file).unwrap();
    assert!(test.editor.git_refresh_pending());
    test.settle_git();
    assert_eq!(
        test.editor.buffer().git_status().get_line_status(0),
        Some(ovim_core::git::LineStatus::Modified)
    );
}
