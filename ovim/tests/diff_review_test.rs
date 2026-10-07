//! Branch diff review (`<Space>gd`): open, navigate hunks, jump into files,
//! resume, refresh and close.

mod helpers;

use git2::{IndexAddOption, Oid, Repository, Signature};
use helpers::EditorTest;
use ovim_core::native_diff::{
    review_patch, review_snapshot, ChangeRef, DiffPairing, PatchLineKind, ReviewBase,
    ReviewSnapshot,
};
use ovim_core::{KeyCode, Mode};
use std::fs;
use std::path::Path;

fn commit_all(repo: &Repository, message: &str) -> Oid {
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let signature = Signature::now("Ovim", "ovim@example.com").unwrap();
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &parents,
    )
    .unwrap()
}

/// A repo with `main` (a.txt = one/two/three) and a checked-out `feature`
/// branch that committed a change to a.txt and has an untracked b.txt.
struct Fixture {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let repo = Repository::init(&root).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        commit_all(&repo, "c1");

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &head, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        fs::write(root.join("a.txt"), "one\n2\nthree\nfour\n").unwrap();
        commit_all(&repo, "edit a");
        fs::write(root.join("b.txt"), "new file\n").unwrap();
        Self { _dir: dir, root }
    }

    fn path(&self, name: &str) -> String {
        self.root.join(name).to_string_lossy().to_string()
    }
}

fn open_editor_on(fixture: &Fixture, name: &str) -> EditorTest {
    let mut test = EditorTest::new("");
    test.editor
        .open_file(Path::new(&fixture.path(name)))
        .unwrap();
    test.settle_git();
    test
}

fn current_line(test: &EditorTest) -> String {
    let line = test.editor.buffer().cursor().line();
    test.editor
        .buffer()
        .line_text(line)
        .map(|text| text.to_string())
        .unwrap_or_default()
}

fn line_index_of(test: &EditorTest, needle: &str) -> usize {
    let buffer = test.editor.buffer();
    (0..buffer.line_count())
        .find(|&index| buffer.line_text(index).as_deref() == Some(needle))
        .unwrap_or_else(|| panic!("no line {needle:?} in review"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn saved_moves_follow_exact_live_diff_and_recover_after_undo() {
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    let snapshot = ReviewSnapshot::from_patch(
        review_patch(&fixture.root, &ReviewBase::explicit("main")).unwrap(),
    )
    .unwrap();
    let removed = snapshot
        .blocks
        .iter()
        .find(|block| block.kind == PatchLineKind::Removed)
        .unwrap();
    let added = snapshot
        .blocks
        .iter()
        .find(|block| {
            block.kind == PatchLineKind::Added && snapshot.patch.files[block.file].path == "b.txt"
        })
        .unwrap();
    let custom = snapshot
        .reassign(&[DiffPairing {
            message: None,
            label: Some("Move line".into()),
            old: ChangeRef {
                block_id: removed.id.clone(),
                offset: None,
                count: None,
            }
            .into(),
            new: ChangeRef {
                block_id: added.id.clone(),
                offset: None,
                count: None,
            }
            .into(),
            related_to: None,
        }])
        .unwrap();

    test.editor
        .open_custom_diff_review("Move line", custom)
        .unwrap();
    assert_eq!(
        test.editor.diff_review_overlay_state().unwrap().mode,
        "saved"
    );
    test.editor.return_to_live_diff_review().unwrap();
    assert_eq!(
        test.editor.diff_review_overlay_state().unwrap().mode,
        "active"
    );
    assert!(test.editor.diff_review().unwrap().custom().is_some());

    fs::write(fixture.root.join("a.txt"), "one\nchanged\nthree\nfour\n").unwrap();
    test.editor.refresh_diff_review();
    assert_eq!(
        test.editor.diff_review_overlay_state().unwrap().mode,
        "stale"
    );
    assert!(test.editor.diff_review().unwrap().custom().is_none());
    test.editor.open_saved_diff_overlay().unwrap();
    assert_eq!(
        test.editor.diff_review_overlay_state().unwrap().mode,
        "saved"
    );
    test.editor.return_to_live_diff_review().unwrap();
    assert_eq!(
        test.editor.diff_review_overlay_state().unwrap().mode,
        "stale"
    );

    fs::write(fixture.root.join("a.txt"), "one\n2\nthree\nfour\n").unwrap();
    test.editor.refresh_diff_review();
    assert_eq!(
        test.editor.diff_review_overlay_state().unwrap().mode,
        "active"
    );
    test.editor.toggle_diff_review_overlay().unwrap();
    assert_eq!(
        test.editor.diff_review_overlay_state().unwrap().mode,
        "available"
    );
    assert!(test.editor.diff_review().unwrap().custom().is_none());
    test.editor.toggle_diff_review_overlay().unwrap();
    assert_eq!(
        test.editor.diff_review_overlay_state().unwrap().mode,
        "active"
    );

    test.editor.close_diff_review();
    test.keys(" gd");
    assert_eq!(
        test.editor.diff_review_overlay_state().unwrap().mode,
        "active"
    );

    test.editor.open_diff_review(Some("main")).unwrap();
    assert_eq!(
        test.editor.diff_review_overlay_state().unwrap().mode,
        "active"
    );
    let visible = test.editor.buffer().rope().to_string();
    Repository::open(&fixture.root)
        .unwrap()
        .find_reference("refs/heads/main")
        .unwrap()
        .delete()
        .unwrap();
    assert!(test.editor.toggle_diff_review_overlay().is_err());
    assert_eq!(
        test.editor.diff_review_overlay_state().unwrap().mode,
        "active"
    );
    assert!(test.editor.diff_review().unwrap().custom().is_some());
    assert_eq!(test.editor.buffer().rope().to_string(), visible);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn custom_review_keeps_cross_file_sources_and_a_frozen_layout() {
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    let snapshot = ReviewSnapshot::from_patch(
        review_patch(&fixture.root, &ReviewBase::explicit("main")).unwrap(),
    )
    .unwrap();
    let removed = snapshot
        .blocks
        .iter()
        .find(|block| {
            block.kind == PatchLineKind::Removed && snapshot.patch.files[block.file].path == "a.txt"
        })
        .unwrap();
    let added = snapshot
        .blocks
        .iter()
        .find(|block| {
            block.kind == PatchLineKind::Added && snapshot.patch.files[block.file].path == "b.txt"
        })
        .unwrap();
    let custom = snapshot
        .reassign(&[DiffPairing {
            message: Some("The code now lives in the new file.\nCheck all callers.".into()),
            label: Some("Move old line".to_string()),
            old: ChangeRef {
                block_id: removed.id.clone(),
                offset: None,
                count: None,
            }
            .into(),
            new: ChangeRef {
                block_id: added.id.clone(),
                offset: None,
                count: None,
            }
            .into(),
            related_to: None,
        }])
        .unwrap();

    test.editor.set_mode(Mode::AiChat);
    test.editor
        .open_custom_diff_review("Move old line", custom.clone())
        .unwrap();
    assert_eq!(test.editor.mode(), Mode::Normal);
    assert!(test.editor.is_diff_review_buffer());
    assert!(test
        .editor
        .buffer()
        .rope()
        .to_string()
        .contains("a.txt → b.txt"));
    assert!(test
        .editor
        .buffer()
        .rope()
        .to_string()
        .contains("Agent note:"));
    let note_line = (0..test.editor.buffer().line_count())
        .find(|&line| {
            test.editor
                .buffer()
                .line_text(line)
                .is_some_and(|text| text.contains("Check all callers."))
        })
        .unwrap();
    test.editor
        .buffer_mut()
        .cursor_mut()
        .set_position(note_line, ovim_core::unicode::GraphemeCol(0));
    test.keys("a");
    assert!(!test
        .editor
        .buffer()
        .rope()
        .to_string()
        .contains("Check all callers."));
    assert!(current_line(&test).contains("Move old line"));
    test.keys("a");
    assert!(test
        .editor
        .buffer()
        .rope()
        .to_string()
        .contains("Check all callers."));
    test.editor
        .set_diff_review_layout(ovim_core::editor::DiffLayout::Split);
    let split = test.editor.buffer().rope().to_string();
    assert!(split.contains("Check all callers."));
    assert!(
        split.lines().any(|line| line.matches("three").count() == 2),
        "{split}"
    );

    test.keys("w");
    let filtered = test.editor.buffer().rope().to_string();
    assert!(filtered.contains("a.txt → b.txt"));
    assert!(filtered
        .lines()
        .any(|line| line.matches("three").count() == 2));
    test.keys("w");

    fs::write(fixture.root.join("a.txt"), "changed\n").unwrap();
    fs::write(fixture.root.join("b.txt"), "changed\n").unwrap();
    test.editor.refresh_diff_review();
    assert_eq!(test.editor.buffer().rope().to_string(), split);

    test.editor
        .diff_review_open_source("a.txt", 2, "old")
        .unwrap();
    assert!(test
        .editor
        .buffer()
        .display_name()
        .unwrap()
        .contains("Before excerpt"));
    assert!(test.editor.buffer().rope().to_string().contains("2 │ two"));

    test.keys(" gd");
    assert!(test.editor.is_diff_review_buffer());
    assert!(test.editor.diff_review().unwrap().custom().is_none());

    test.editor
        .open_custom_diff_review("Move old line", custom)
        .unwrap();
    test.editor
        .diff_review_open_source("b.txt", 1, "new")
        .unwrap();
    assert!(test
        .editor
        .buffer()
        .display_name()
        .unwrap()
        .contains("After excerpt"));
    assert!(test
        .editor
        .buffer()
        .rope()
        .to_string()
        .contains("1 │ new file"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn saved_review_opens_context_outside_the_patch_after_source_deletion() {
    let fixture = Fixture::new();
    let repo = Repository::open(&fixture.root).unwrap();
    let original = (1..=80)
        .map(|line| format!("source line {line}\n"))
        .collect::<String>();
    fs::write(fixture.root.join("context.txt"), &original).unwrap();
    commit_all(&repo, "add context source");
    fs::write(
        fixture.root.join("context.txt"),
        original.replace("source line 40\n", "changed line 40\n"),
    )
    .unwrap();
    let snapshot = review_snapshot(&fixture.root, &ReviewBase::explicit("HEAD")).unwrap();
    assert!(!snapshot.patch.text.contains("source line 10\n"));
    let custom = snapshot.reassign(&[]).unwrap();
    let mut test = open_editor_on(&fixture, "a.txt");
    fs::remove_file(fixture.root.join("context.txt")).unwrap();
    for side in ["old", "new"] {
        test.editor
            .open_custom_diff_review("Saved context", custom.clone())
            .unwrap();
        test.editor
            .diff_review_open_source("context.txt", 10, side)
            .unwrap();
        assert!(current_line(&test).contains("10 │ source line 10"));
        assert!(test
            .editor
            .buffer()
            .rope()
            .to_string()
            .contains("80 │ source line 80"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn leader_gd_opens_a_highlighted_review_in_a_new_tab() {
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    assert_eq!(test.editor.tab_count(), 1);

    test.keys(" gd");

    assert!(test.editor.is_diff_review_buffer());
    assert_eq!(test.editor.tab_count(), 2);
    assert!(test.editor.buffer().is_read_only());
    assert_eq!(
        test.editor.buffer().display_name(),
        Some("Diff · feature → main")
    );
    assert_eq!(current_line(&test), "feature → main");

    let text = test.editor.buffer().rope().to_string();
    assert!(text.contains("2 files · +3 −1"), "{text}");
    assert!(text.contains("  M  a.txt  +2 −1"), "{text}");
    assert!(text.contains("  A  b.txt  +1  [ ]"), "{text}");
    assert!(text.contains("@@ -1,3 +1,4 @@"), "{text}");
    assert!(text.contains("\n+new file\n"), "{text}");
    assert!(text.contains("1 commit ahead"), "{text}");

    // The pathless buffer still gets the diff grammar.
    assert!(
        test.editor.buffer().has_syntax_highlighting(),
        "review buffer should be highlighted as a diff"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn bracket_c_walks_hunks_and_enter_opens_the_source_line() {
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    test.keys(" gd");

    test.keys("]c");
    assert_eq!(current_line(&test), "@@ -1,3 +1,4 @@");
    test.keys("]c");
    assert_eq!(current_line(&test), "@@ -0,0 +1 @@");
    test.keys("]c");
    assert_eq!(
        current_line(&test),
        "@@ -0,0 +1 @@",
        "stays on the last hunk"
    );
    assert_eq!(test.editor.status_message(), "Last hunk");
    test.keys("[c");
    assert_eq!(current_line(&test), "@@ -1,3 +1,4 @@");

    // Land on the added `+four` line, column 3 → column 2 in the file.
    let four = line_index_of(&test, "+four");
    test.set_cursor(four, 3);
    test.press_key(KeyCode::Enter);

    assert!(!test.editor.is_diff_review_buffer());
    assert_eq!(
        test.editor.tab_count(),
        2,
        "file opens in the originating tab"
    );
    assert_eq!(test.editor.current_tab_index(), 0);
    assert!(test.editor.buffer().file_path().unwrap().ends_with("a.txt"));
    let cursor = test.editor.buffer().cursor();
    assert_eq!((cursor.line(), cursor.col().0), (3, 2));
    assert_eq!(current_line(&test), "four");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn enter_on_a_removed_line_lands_where_the_removal_happened() {
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    test.keys(" gd");

    let removed = line_index_of(&test, "-two");
    test.set_cursor(removed, 0);
    test.press_key(KeyCode::Enter);

    assert!(test.editor.buffer().file_path().unwrap().ends_with("a.txt"));
    assert_eq!(test.editor.buffer().cursor().line(), 1);
    assert_eq!(current_line(&test), "2");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn enter_on_a_file_row_jumps_to_that_file_and_opens_new_files() {
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    test.keys(" gd");

    let row = line_index_of(&test, "  A  b.txt  +1  [ ]");
    test.set_cursor(row, 0);
    test.press_key(KeyCode::Enter);
    assert!(test.editor.is_diff_review_buffer());
    assert_eq!(current_line(&test), "diff --git a/b.txt b/b.txt  [ ]");

    // Enter on a file header opens the file at its first hunk.
    test.press_key(KeyCode::Enter);
    assert!(test.editor.buffer().file_path().unwrap().ends_with("b.txt"));
    assert_eq!(test.editor.buffer().cursor().line(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn leader_gd_resumes_the_review_at_the_same_hunk_after_editing() {
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    test.keys(" gd");

    let four = line_index_of(&test, "+four");
    test.set_cursor(four, 0);
    test.press_key(KeyCode::Enter);
    assert_eq!(current_line(&test), "four");

    // Edit the file on disk (as if saved) so the refreshed review differs.
    fs::write(fixture.path("a.txt"), "zero\none\n2\nthree\nfour\n").unwrap();

    test.keys(" gd");
    assert!(test.editor.is_diff_review_buffer());
    assert_eq!(test.editor.current_tab_index(), 1);
    assert_eq!(
        current_line(&test),
        "+four",
        "cursor follows the source line through the refresh"
    );
    let text = test.editor.buffer().rope().to_string();
    assert!(
        text.contains("+zero"),
        "review picked up the new change: {text}"
    );

    // Leaving the review returns to the file tab.
    test.keys(" gd");
    assert!(!test.editor.is_diff_review_buffer());
    assert_eq!(test.editor.current_tab_index(), 0);
    assert!(test.editor.buffer().file_path().unwrap().ends_with("a.txt"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn q_closes_the_review_and_drops_its_buffer() {
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    let buffers_before = test.editor.buffer_count();
    test.keys(" gd");
    assert_eq!(test.editor.buffer_count(), buffers_before + 1);

    test.keys("q");

    assert!(test.editor.diff_review().is_none());
    assert_eq!(test.editor.tab_count(), 1);
    assert_eq!(test.editor.buffer_count(), buffers_before);
    assert!(test.editor.buffer().file_path().unwrap().ends_with("a.txt"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn gitdiff_command_accepts_an_explicit_base() {
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");

    let result = ovim_core::commands::execute_command(&mut test.editor, "GitDiff HEAD");
    assert!(
        matches!(result, ovim_core::CommandResult::Success(_)),
        "{result:?}"
    );

    assert!(test.editor.is_diff_review_buffer());
    let text = test.editor.buffer().rope().to_string();
    assert!(text.starts_with("feature → HEAD\n"), "{text}");
    assert!(
        text.contains("+new file"),
        "uncommitted b.txt is included: {text}"
    );
    assert!(
        !text.contains("+four"),
        "committed work is excluded: {text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn on_the_default_branch_the_review_shows_uncommitted_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let repo = Repository::init(&root).unwrap();
    repo.set_head("refs/heads/main").unwrap();
    fs::write(root.join("a.txt"), "one\n").unwrap();
    commit_all(&repo, "c1");
    fs::write(root.join("a.txt"), "one\nuncommitted\n").unwrap();

    let mut test = EditorTest::new("");
    test.editor.open_file(root.join("a.txt").as_path()).unwrap();
    test.keys(" gd");

    let text = test.editor.buffer().rope().to_string();
    assert!(text.starts_with("main → main\n"), "{text}");
    assert!(
        text.contains("On main: uncommitted changes against HEAD"),
        "{text}"
    );
    assert!(text.contains("+uncommitted"), "{text}");
}

/// A repo whose feature branch rewrites one line of a Rust file and adds a
/// tab-indented line, so the split layout has to align both sides.
struct RustFixture {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
}

impl RustFixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let repo = Repository::init(&root).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        fs::write(
            root.join("a.rs"),
            "fn main() {\n    let x = 1;\n    println!(\"hi\");\n}\n",
        )
        .unwrap();
        commit_all(&repo, "c1");

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &head, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        fs::write(
            root.join("a.rs"),
            "fn main() {\n    let x = 42;\n\tlet y = \"two\";\n    println!(\"hi\");\n}\n",
        )
        .unwrap();
        commit_all(&repo, "edit a");
        Self { _dir: dir, root }
    }

    fn open(&self) -> EditorTest {
        let mut test = EditorTest::new("");
        test.editor
            .open_file(Path::new(
                &self.root.join("a.rs").to_string_lossy().to_string(),
            ))
            .unwrap();
        test
    }
}

fn buffer_text(test: &EditorTest) -> String {
    test.editor.buffer().rope().to_string()
}

/// First buffer line whose text contains `needle`.
fn line_containing(test: &EditorTest, needle: &str) -> usize {
    let buffer = test.editor.buffer();
    (0..buffer.line_count())
        .find(|&index| {
            buffer
                .line_text(index)
                .is_some_and(|text| text.contains(needle))
        })
        .unwrap_or_else(|| panic!("no line containing {needle:?} in review"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn the_patch_is_highlighted_with_each_file_s_own_grammar() {
    let fixture = RustFixture::new();
    let mut test = fixture.open();
    test.keys(" gd");

    // `let` inside an added line is a Rust keyword, not "an added line".
    let added = line_index_of(&test, "+    let x = 42;");
    let highlights = test.editor.buffer().highlights_for_line(added);
    assert!(
        highlights.iter().any(|(range, group)| *group
            == ovim_core::syntax::HighlightGroup::Keyword
            && range.start == 5),
        "expected a keyword span over `let`: {highlights:?}"
    );
    // The marker column still carries the diff colour.
    assert!(highlights
        .iter()
        .any(|(range, group)| *range == (0..1)
            && *group == ovim_core::syntax::HighlightGroup::DiffAdded));

    // Removed lines are highlighted from the old side of the patch.
    let removed = line_index_of(&test, "-    let x = 1;");
    assert!(test
        .editor
        .buffer()
        .highlights_for_line(removed)
        .iter()
        .any(|(_, group)| *group == ovim_core::syntax::HighlightGroup::Keyword));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn changed_rows_carry_a_background_tint_and_context_rows_do_not() {
    let fixture = RustFixture::new();
    let mut test = fixture.open();
    test.keys(" gd");

    let added = line_index_of(&test, "+    let x = 42;");
    let removed = line_index_of(&test, "-    let x = 1;");
    let context = line_index_of(&test, " fn main() {");
    let review = test.editor.diff_review().unwrap();
    assert_eq!(review.line_tints(added).len(), 1);
    assert!(review.line_tints(added)[0].1, "added rows tint green");
    assert!(!review.line_tints(removed)[0].1, "removed rows tint red");
    assert!(review.line_tints(context).is_empty());

    // The band runs to the end of the row so the renderer can carry it
    // through the padding to the right edge.
    assert_eq!(review.line_trailing_tint(added), Some(true));
    assert_eq!(review.line_trailing_tint(removed), Some(false));
    assert_eq!(review.line_trailing_tint(context), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn only_the_side_that_changed_is_tinted_in_the_split_layout() {
    let fixture = RustFixture::new();
    let mut test = fixture.open();
    test.keys(" gds");

    // A rewritten line pairs a removal on the left with an addition on the
    // right, so the row carries both tints and the band ends on the new side.
    let row = line_containing(&test, "let x = 1;");
    let review = test.editor.diff_review().unwrap();
    let tints = review.line_tints(row);
    assert_eq!(tints.len(), 2, "{tints:?}");
    assert!(!tints[0].1, "the old column is a removal");
    assert!(tints[1].1, "the new column is an addition");
    assert!(
        tints[0].0.end < tints[1].0.start,
        "the separator is untinted"
    );
    assert_eq!(review.line_trailing_tint(row), Some(true));

    // A row that only adds leaves the old column plain.
    let added_only = line_containing(&test, "let y = \"two\";");
    let tints = test.editor.diff_review().unwrap().line_tints(added_only);
    assert_eq!(tints.len(), 1, "{tints:?}");
    assert!(tints[0].1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn s_switches_between_the_unified_and_split_layouts() {
    let fixture = RustFixture::new();
    let mut test = fixture.open();
    test.keys(" gd");
    assert!(buffer_text(&test).contains("diff --git a/a.rs b/a.rs"));
    assert!(!buffer_text(&test).contains('│'));

    test.keys("s");

    let text = buffer_text(&test);
    assert!(
        text.contains('│'),
        "side-by-side rows are separated: {text}"
    );
    assert!(
        text.contains("── a.rs "),
        "the raw file header becomes a banner: {text}"
    );
    assert!(
        !text.contains("diff --git"),
        "the git header is folded into the banner: {text}"
    );
    // Both sides of the rewritten line share a row, with their line numbers.
    let row = line_containing(&test, "let x = 1;");
    let row_text = test.editor.buffer().line_text(row).unwrap().to_string();
    assert!(row_text.contains("let x = 42;"), "{row_text}");
    assert!(row_text.starts_with("  2 -"), "{row_text}");

    test.keys("s");
    assert!(buffer_text(&test).contains("diff --git a/a.rs b/a.rs"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn split_rows_align_after_expanding_tabs() {
    let fixture = RustFixture::new();
    let mut test = fixture.open();
    test.keys(" gds");

    // The added line is indented with a tab; both sides must still line up on
    // the separator column.
    let rows: Vec<String> = (0..test.editor.buffer().line_count())
        .filter_map(|index| {
            test.editor
                .buffer()
                .line_text(index)
                .map(|text| text.to_string())
        })
        .filter(|text| text.contains('│'))
        .collect();
    assert!(rows.len() >= 4, "{rows:?}");
    let columns: Vec<usize> = rows
        .iter()
        .map(|text| text.chars().position(|c| c == '│').unwrap())
        .collect();
    assert!(
        columns.windows(2).all(|pair| pair[0] == pair[1]),
        "every row separates at the same column: {columns:?}"
    );
    assert!(
        rows.iter()
            .any(|text| text.contains("    let y = \"two\";")),
        "the tab is expanded, not passed through: {rows:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn split_separator_tracks_the_rendered_viewport_center() {
    let fixture = RustFixture::new();
    let mut test = fixture.open();
    test.editor.options.textwidth = None;
    test.keys(" gds");

    let render_at = |test: &mut EditorTest, columns| {
        ovim::ui::render_editor_to_ansi(&mut test.editor, columns, 40).unwrap();
        assert!(
            test.editor.relayout_diff_review(),
            "split review did not relayout at {columns} columns; cached area: {:?}, text width: {}",
            test.editor.render_cache.last_buffer_area,
            test.editor.render_cache.last_text_width,
        );
    };
    render_at(&mut test, 180);

    let separator = |test: &EditorTest| {
        let row = line_containing(test, "let x = 1;");
        test.editor
            .buffer()
            .line_text(row)
            .unwrap()
            .chars()
            .position(|character| character == '│')
            .unwrap()
    };
    let first = separator(&test);
    let first_center = (test.editor.render_cache.last_text_width - 1) / 2;
    assert!(first.abs_diff(first_center) <= 1, "separator at {first}");

    render_at(&mut test, 240);
    let resized = separator(&test);
    let resized_center = (test.editor.render_cache.last_text_width - 1) / 2;
    assert!(
        resized.abs_diff(resized_center) <= 1,
        "separator at {resized}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn split_review_uses_its_source_numbers_instead_of_an_outer_gutter() {
    let fixture = RustFixture::new();
    let mut test = fixture.open();
    test.editor.options.number = true;
    test.editor.options.textwidth = None;
    test.keys(" gds");

    ovim::ui::render_editor_to_ansi(&mut test.editor, 120, 30).unwrap();
    assert_eq!(test.editor.render_cache.last_gutter_width, 0);
    assert_eq!(
        test.editor.render_cache.last_text_width,
        test.editor.render_cache.last_buffer_area.unwrap().width as usize,
        "the text occupies the buffer area after the diff scrollbar",
    );
    let changed = line_containing(&test, "let x = 1;");
    let row = test.editor.buffer().line_text(changed).unwrap();
    assert!(
        row.contains('│'),
        "source line numbers remain in the split row"
    );

    test.keys("s");
    ovim::ui::render_editor_to_ansi(&mut test.editor, 120, 30).unwrap();
    assert!(
        test.editor.render_cache.last_gutter_width > 0,
        "unified review keeps ordinary buffer line numbers"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn enter_in_the_split_layout_opens_the_column_under_the_cursor() {
    let fixture = RustFixture::new();
    let mut test = fixture.open();
    test.keys(" gds");

    let row = line_containing(&test, "let x = 42;");
    let text = test.editor.buffer().line_text(row).unwrap().to_string();
    // Land on the `4` of `42` on the new side. Columns are graphemes, and the
    // row's separator is multi-byte.
    let col = text[..text.find("42;").unwrap()].chars().count();
    test.set_cursor(row, col);
    test.press_key(KeyCode::Enter);

    assert!(test.editor.buffer().file_path().unwrap().ends_with("a.rs"));
    let cursor = test.editor.buffer().cursor();
    assert_eq!(cursor.line(), 1, "the new side's line 2");
    assert_eq!(
        current_line(&test)
            .chars()
            .nth(cursor.col().0)
            .unwrap_or(' '),
        '4',
        "cursor keeps its column: {:?}",
        current_line(&test)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn enter_on_the_old_column_lands_where_the_removal_happened() {
    let fixture = RustFixture::new();
    let mut test = fixture.open();
    test.keys(" gds");

    let row = line_containing(&test, "let x = 1;");
    let text = test.editor.buffer().line_text(row).unwrap().to_string();
    let col = text[..text.find("let x = 1;").unwrap()].chars().count();
    test.set_cursor(row, col);
    test.press_key(KeyCode::Enter);

    assert!(test.editor.buffer().file_path().unwrap().ends_with("a.rs"));
    assert_eq!(test.editor.buffer().cursor().line(), 1);
    assert_eq!(current_line(&test), "    let x = 42;");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn clicking_the_toolbar_switches_layout_without_moving_the_cursor() {
    let fixture = RustFixture::new();
    let mut test = fixture.open();
    test.keys(" gd");

    let toolbar = line_containing(&test, "[ Split ]");
    let text = test.editor.buffer().line_text(toolbar).unwrap().to_string();
    let split_col = text.find("[ Split ]").unwrap() + 2;
    let unified_col = text.find("[ Unified ]").unwrap() + 2;
    let hint_col = text.find("· click").unwrap();

    assert!(test.editor.diff_review_click(toolbar, split_col));
    assert!(buffer_text(&test).contains('│'));

    assert!(test.editor.diff_review_click(toolbar, unified_col));
    assert!(buffer_text(&test).contains("diff --git"));

    assert!(
        !test.editor.diff_review_click(toolbar, hint_col),
        "clicks outside the buttons fall through to the cursor"
    );
    assert!(
        !test.editor.diff_review_click(toolbar + 1, split_col),
        "only the toolbar row is clickable"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn switching_layout_keeps_the_cursor_on_the_same_change() {
    let fixture = RustFixture::new();
    let mut test = fixture.open();
    test.keys(" gd");

    let added = line_index_of(&test, "+\tlet y = \"two\";");
    test.set_cursor(added, 0);
    test.keys("s");
    assert!(
        current_line(&test).contains("let y = \"two\";"),
        "cursor followed the change into the split view: {:?}",
        current_line(&test)
    );

    test.keys("s");
    assert_eq!(current_line(&test), "+\tlet y = \"two\";");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn hunk_and_file_navigation_work_in_the_split_layout() {
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    test.keys(" gds");

    test.keys("]f");
    assert!(
        current_line(&test).contains("── a.txt "),
        "{}",
        current_line(&test)
    );
    test.keys("]f");
    assert!(
        current_line(&test).contains("── b.txt "),
        "{}",
        current_line(&test)
    );
    test.keys("[f");
    assert!(
        current_line(&test).contains("── a.txt "),
        "{}",
        current_line(&test)
    );

    test.keys("]c");
    assert_eq!(current_line(&test), "@@ -1,3 +1,4 @@");
    test.keys("]c");
    assert_eq!(current_line(&test), "@@ -0,0 +1 @@");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn the_layout_choice_survives_closing_and_reopening_the_review() {
    let fixture = RustFixture::new();
    let mut test = fixture.open();
    test.keys(" gds");
    test.keys("q");
    assert!(test.editor.diff_review().is_none());

    test.keys(" gd");
    assert!(
        buffer_text(&test).contains('│'),
        "the next review opens in the layout you last chose"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn gitdifflayout_command_sets_the_layout_explicitly() {
    let fixture = RustFixture::new();
    let mut test = fixture.open();
    test.keys(" gd");

    let result = ovim_core::commands::execute_command(&mut test.editor, "GitDiffLayout split");
    assert!(
        matches!(result, ovim_core::CommandResult::Success(_)),
        "{result:?}"
    );
    assert!(buffer_text(&test).contains('│'));

    let result = ovim_core::commands::execute_command(&mut test.editor, "GitDiffLayout sideways");
    assert!(
        matches!(result, ovim_core::CommandResult::Error(_)),
        "{result:?}"
    );
    assert!(
        buffer_text(&test).contains('│'),
        "an invalid name changes nothing"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn the_split_layout_handles_a_branch_with_no_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let repo = Repository::init(&root).unwrap();
    repo.set_head("refs/heads/main").unwrap();
    fs::write(root.join("a.txt"), "one\n").unwrap();
    commit_all(&repo, "c1");
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("feature", &head, false).unwrap();
    repo.set_head("refs/heads/feature").unwrap();

    let mut test = EditorTest::new("");
    test.editor.open_file(root.join("a.txt").as_path()).unwrap();
    test.keys(" gds");

    let text = buffer_text(&test);
    assert!(text.contains("No changes"), "{text}");
    // Navigation on an empty review reports rather than panics.
    test.keys("]c");
    assert_eq!(test.editor.status_message(), "Last hunk");
    test.keys("]f");
    assert_eq!(test.editor.status_message(), "Last file");
    test.press_key(KeyCode::Enter);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn the_split_layout_shows_deletions_and_binaries() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let repo = Repository::init(&root).unwrap();
    repo.set_head("refs/heads/main").unwrap();
    fs::write(root.join("gone.txt"), "one\ntwo\n").unwrap();
    fs::write(root.join("keep.txt"), "keep\n").unwrap();
    commit_all(&repo, "c1");

    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("feature", &head, false).unwrap();
    repo.set_head("refs/heads/feature").unwrap();
    fs::remove_file(root.join("gone.txt")).unwrap();
    fs::write(root.join("blob.bin"), [0u8, 159, 146, 150, 0]).unwrap();
    commit_all(&repo, "delete and add a binary");

    let mut test = EditorTest::new("");
    test.editor
        .open_file(root.join("keep.txt").as_path())
        .unwrap();
    test.keys(" gds");

    let text = buffer_text(&test);
    assert!(text.contains("── blob.bin "), "{text}");
    assert!(text.contains("── gone.txt "), "{text}");
    assert!(text.contains("D  +0 −2"), "{text}");

    // Enter on a deleted file refuses instead of opening a missing path.
    let row = line_containing(&test, "- one");
    test.set_cursor(row, 8);
    test.press_key(KeyCode::Enter);
    assert!(test.editor.is_diff_review_buffer());
    assert!(test
        .editor
        .buffer()
        .file_path()
        .is_none_or(|path| !path.ends_with("gone.txt")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_long_line_wraps_inside_its_split_column() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let repo = Repository::init(&root).unwrap();
    repo.set_head("refs/heads/main").unwrap();
    fs::write(root.join("a.txt"), "short\n").unwrap();
    commit_all(&repo, "c1");
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("feature", &head, false).unwrap();
    repo.set_head("refs/heads/feature").unwrap();
    let long = "x".repeat(1200);
    fs::write(root.join("a.txt"), format!("short\n{long}\n")).unwrap();
    commit_all(&repo, "long line");

    let mut test = EditorTest::new("");
    test.editor.open_file(root.join("a.txt").as_path()).unwrap();
    test.keys(" gds");

    let rows: Vec<String> = (0..test.editor.buffer().line_count())
        .filter_map(|index| test.editor.buffer().line_text(index).map(|t| t.to_string()))
        .filter(|text| text.contains("xxx"))
        .collect();
    assert!(rows.len() > 1, "the long line wraps: {}", rows.len());
    // Wrapping is capped, and the clipped row says so.
    assert!(rows.len() <= 12, "wrapping is bounded: {}", rows.len());
    assert!(
        rows.last().unwrap().contains('…'),
        "the clipped row is marked: {:?}",
        rows.last()
    );
    // Every wrapped row still ends at the same separator column.
    let columns: Vec<usize> = rows
        .iter()
        .map(|text| text.chars().position(|c| c == '│').unwrap())
        .collect();
    assert!(
        columns.windows(2).all(|pair| pair[0] == pair[1]),
        "{columns:?}"
    );

    // Enter from a continuation row lands on the same source line.
    let row = line_containing(&test, "xxx");
    test.set_cursor(row + 1, 70);
    test.press_key(KeyCode::Enter);
    assert_eq!(test.editor.buffer().cursor().line(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn pullbase_controls_review_and_unset_restores_auto() {
    use ovim_core::command_result::CommandResult;
    use ovim_core::commands::execute_command;
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    for command in ["set pullbase=feature", "GitDiff"] {
        assert!(matches!(
            execute_command(&mut test.editor, command),
            CommandResult::Success(_)
        ));
    }
    assert_eq!(test.editor.diff_review().unwrap().base().name, "feature");
    execute_command(&mut test.editor, "set pullbase=main");
    assert_eq!(test.editor.diff_review().unwrap().base().name, "main");
    execute_command(&mut test.editor, "GitDiff feature");
    execute_command(&mut test.editor, "unset pullbase");
    assert_eq!(test.editor.diff_review().unwrap().base().name, "feature");
    execute_command(&mut test.editor, "GitDiff");
    assert_eq!(test.editor.diff_review().unwrap().base().name, "main");
    for unset in [
        "set pullbase=",
        "set pullbase&",
        "set nopullbase",
        "unset pullbase",
    ] {
        execute_command(&mut test.editor, "set pullbase=feature");
        assert!(matches!(
            execute_command(&mut test.editor, unset),
            CommandResult::Success(_)
        ));
        assert_eq!(test.editor.options.pullbase, None);
    }
    assert!(matches!(
        execute_command(&mut test.editor, "set pullbase=main..feature"),
        CommandResult::Error(_)
    ));
    assert_eq!(test.editor.options.pullbase, None);
    let result = execute_command(&mut test.editor, "set pullbase?");
    assert!(
        matches!(result, CommandResult::Success(ref response) if response.message.as_deref() == Some("  pullbase="))
    );
}

#[test]
fn pullbase_resolves_remote_metadata_and_reports_missing_branches() {
    let fixture = Fixture::new();
    let repo = Repository::open(&fixture.root).unwrap();
    let oid = repo.refname_to_id("refs/heads/main").unwrap();
    repo.reference("refs/remotes/origin/release/stable", oid, true, "test")
        .unwrap();
    let base =
        ovim_core::native_diff::resolve_pullbase(&fixture.root, Some("origin/release/stable"))
            .unwrap();
    assert_eq!(
        base.remote,
        Some(("origin".into(), "release/stable".into()))
    );
    assert_eq!(base.spec, "origin/release/stable...WORKTREE");
    assert!(ovim_core::native_diff::resolve_pullbase(&fixture.root, Some("missing")).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn pullbase_path_override_refreshes_and_unsets_without_changing_global() {
    use ovim_core::command_result::CommandResult;
    use ovim_core::commands::execute_command;
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    execute_command(&mut test.editor, "set pullbase=main");
    execute_command(&mut test.editor, "GitDiff");
    let path = fixture.root.display();
    assert!(matches!(
        execute_command(
            &mut test.editor,
            &format!("set pullbase=feature path={path}")
        ),
        CommandResult::Success(_)
    ));
    assert_eq!(test.editor.diff_review().unwrap().base().name, "feature");
    assert_eq!(test.editor.options.pullbase.as_deref(), Some("main"));
    test.editor.refresh_diff_review();
    assert_eq!(test.editor.diff_review().unwrap().base().name, "feature");
    let query = execute_command(&mut test.editor, &format!("set pullbase? path={path}"));
    assert!(matches!(query, CommandResult::Success(ref response)
        if response.message.as_deref() == Some(format!("  pullbase=feature path={path}").as_str())));
    execute_command(&mut test.editor, "GitDiff main");
    assert_eq!(test.editor.diff_review().unwrap().base().name, "main");
    execute_command(&mut test.editor, "GitDiff");
    assert_eq!(test.editor.diff_review().unwrap().base().name, "feature");
    execute_command(&mut test.editor, &format!("unset pullbase path={path}"));
    assert_eq!(test.editor.diff_review().unwrap().base().name, "main");
    assert!(test.editor.options.pullbase_paths.is_empty());
    assert_eq!(test.editor.options.pullbase.as_deref(), Some("main"));
}

#[test]
fn pullbase_path_matching_uses_repo_root_and_closest_directory() {
    use ovim_core::native_diff::pullbase_for_path;
    use std::collections::BTreeMap;
    let fixture = Fixture::new();
    let nested = fixture.root.join("source directory");
    fs::create_dir(&nested).unwrap();
    let mut overrides = BTreeMap::new();
    overrides.insert(
        fixture.root.parent().unwrap().to_path_buf(),
        "parent".into(),
    );
    overrides.insert(fixture.root.clone(), "project".into());
    overrides.insert(nested.clone(), "source".into());
    assert_eq!(
        pullbase_for_path(&nested, Some("global"), &overrides).unwrap(),
        Some("project")
    );
    overrides.remove(&fixture.root);
    assert_eq!(
        pullbase_for_path(&nested, Some("global"), &overrides).unwrap(),
        Some("parent")
    );
    overrides.remove(fixture.root.parent().unwrap());
    assert_eq!(
        pullbase_for_path(&nested, Some("global"), &overrides).unwrap(),
        Some("global")
    );
    assert_eq!(pullbase_for_path(&nested, None, &overrides).unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn pullbase_controls_gutter_signs_but_keeps_head_as_the_unset_default() {
    use ovim_core::commands::execute_command;
    use ovim_core::git::GitStatus;
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    assert_eq!(test.editor.buffer().git_status().change_counts(), (0, 0, 0));
    // The review still defaults to main, even though the gutter defaults to HEAD.
    execute_command(&mut test.editor, "GitDiff");
    assert_eq!(test.editor.diff_review().unwrap().base().name, "main");
    test.editor.close_diff_review();
    execute_command(&mut test.editor, "set pullbase=main");
    test.settle_git();
    let changes = test.editor.buffer().git_status().change_counts();
    assert_ne!(changes, (0, 0, 0));
    let path = fixture.root.display();
    execute_command(
        &mut test.editor,
        &format!("set pullbase=feature path={path}"),
    );
    test.settle_git();
    assert_eq!(test.editor.buffer().git_status().change_counts(), (0, 0, 0));
    execute_command(&mut test.editor, &format!("unset pullbase path={path}"));
    test.settle_git();
    assert_eq!(test.editor.buffer().git_status().change_counts(), changes);

    // An editor configured before opening a file computes signs against the pull base.
    let mut other = ovim_core::editor::Editor::new();
    execute_command(&mut other, "set pullbase=main");
    other.open_file(Path::new(&fixture.path("a.txt"))).unwrap();
    helpers::settle_git_refresh(&mut other);
    assert_eq!(other.buffer().git_status().change_counts(), changes);

    // Background save refresh uses the override, and a later unset invalidates it.
    test.editor.spawn_git_refresh(&fixture.path("a.txt"), false);
    let mut refreshed = false;
    for _ in 0..100 {
        if test.editor.poll_git_refresh() {
            refreshed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(refreshed, "background gutter refresh must complete");
    assert_eq!(test.editor.buffer().git_status().change_counts(), changes);
    test.editor.spawn_git_refresh(&fixture.path("a.txt"), false);
    execute_command(&mut test.editor, "unset pullbase");
    test.settle_git();
    assert_eq!(test.editor.buffer().git_status().change_counts(), (0, 0, 0));

    fs::write(fixture.root.join("a.txt"), "uncommitted\n").unwrap();
    assert_ne!(
        GitStatus::from_file(fixture.root.join("a.txt"))
            .unwrap()
            .change_counts(),
        (0, 0, 0)
    );
}

#[test]
fn pullbase_gutter_uses_merge_base_not_unrelated_changes_on_target() {
    use ovim_core::git::GitStatus;
    let fixture = Fixture::new();
    let repo = Repository::open(&fixture.root).unwrap();
    repo.set_head("refs/heads/main").unwrap();
    fs::write(fixture.root.join("a.txt"), "unrelated target changes\n").unwrap();
    commit_all(&repo, "advance target independently");
    repo.set_head("refs/heads/feature").unwrap();
    fs::write(fixture.root.join("a.txt"), "one\n2\nthree\nfour\n").unwrap();
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.force();
    repo.checkout_head(Some(&mut checkout)).unwrap();
    let signs =
        GitStatus::from_file_with_pullbase(fixture.root.join("a.txt"), Some("main")).unwrap();
    assert_eq!(
        signs.get_line_status(0),
        None,
        "unchanged first line must not be marked"
    );
    assert!(signs.get_line_status(3).is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn blame_mouse_hover_renders_details_without_moving_cursor_or_taking_keyboard() {
    use ovim::editor::handle_mouse_event;
    use ovim::ui::Renderer;
    use ovim_core::{MouseEvent, MouseEventKind, Rect};
    use ratatui::{backend::TestBackend, Terminal};
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    test.editor.options.blame = true;
    test.editor.options.wrap = false;
    test.editor.buffer_mut().load_git_blame();
    test.editor.render_cache.last_buffer_area = Some(Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 25,
    });
    test.editor.render_cache.last_blame_width = 20;
    let mouse = |row, column| MouseEvent {
        kind: MouseEventKind::Moved,
        row,
        column,
    };
    handle_mouse_event(&mut test.editor, mouse(1, 1)).unwrap();
    assert_eq!(test.editor.mode(), ovim_core::mode::Mode::Normal);
    assert_eq!(test.editor.buffer().cursor().line(), 0);
    assert_eq!(test.editor.hover_position(), Some((1, 0)));
    assert!(test.editor.hover_info().unwrap().contains("edit a"));
    test.editor.mark_clean();
    handle_mouse_event(&mut test.editor, mouse(1, 2)).unwrap();
    assert!(
        !test.editor.is_dirty(),
        "moving within the same annotation must not redraw"
    );
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut Default::default()))
        .unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("Author:"));
    assert!(rendered.contains("edit a"));
    assert!(
        !terminal.backend().cursor_visible(),
        "the hardware caret must not cover blame details"
    );
    handle_mouse_event(&mut test.editor, mouse(1, 99)).unwrap();
    assert!(test.editor.hover_info().is_none());
    terminal
        .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut Default::default()))
        .unwrap();
    assert!(
        terminal.backend().cursor_visible(),
        "leaving the popup restores the caret"
    );
    test.editor.show_blame_info();
    terminal
        .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut Default::default()))
        .unwrap();
    assert!(
        !terminal.backend().cursor_visible(),
        "keyboard blame popovers hide the caret too"
    );
    test.press_key(ovim_core::KeyCode::Esc);
    handle_mouse_event(&mut test.editor, mouse(1, 1)).unwrap();
    test.keys("j");
    assert_eq!(
        test.editor.buffer().cursor().line(),
        1,
        "j must move the editing cursor"
    );
    assert!(test.editor.hover_info().is_none());
    handle_mouse_event(&mut test.editor, mouse(20, 1)).unwrap();
    assert!(
        test.editor.hover_info().is_none(),
        "blank gutter rows have no annotation"
    );
    test.keys("i");
    handle_mouse_event(&mut test.editor, mouse(1, 1)).unwrap();
    assert_eq!(test.editor.mode(), ovim_core::mode::Mode::Insert);
    assert!(test.editor.hover_info().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn clicking_blame_opens_that_commit_and_preserves_source_cursor() {
    use ovim::editor::handle_mouse_event;
    use ovim_core::{MouseButton, MouseEvent, MouseEventKind, Rect};
    let fixture = Fixture::new();
    let repo = Repository::open(&fixture.root).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    for (row, oid) in [(0, head.parent_id(0).unwrap()), (1, head.id())] {
        let mut test = open_editor_on(&fixture, "a.txt");
        test.editor.options.blame = true;
        test.editor.options.wrap = false;
        test.editor.buffer_mut().load_git_blame();
        let source_id = test.editor.buffer().id();
        let source_cursor = *test.editor.buffer().cursor();
        test.editor.render_cache.last_buffer_area = Some(Rect {
            x: 2,
            y: 3,
            width: 100,
            height: 25,
        });
        test.editor.render_cache.last_blame_width = 20;
        handle_mouse_event(
            &mut test.editor,
            MouseEvent {
                kind: MouseEventKind::Moved,
                row: row + 3,
                column: 3,
            },
        )
        .unwrap();
        assert!(test.editor.blame_mouse_hover_active());
        handle_mouse_event(
            &mut test.editor,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                row: row + 3,
                column: 3,
            },
        )
        .unwrap();
        assert_eq!(test.editor.tab_count(), 2);
        assert_eq!(test.editor.mode(), ovim_core::mode::Mode::Normal);
        assert!(test.editor.hover_info().is_none());
        assert!(test.editor.buffer().is_read_only());
        assert!(test.editor.buffer().file_path().is_none());
        let diff = test.editor.buffer().rope().to_string();
        assert!(diff.starts_with(&format!("commit {oid}\n")), "{diff}");
        assert!(diff.contains("diff --git a/a.txt b/a.txt"));
        assert!(
            !diff.contains("b.txt"),
            "uncommitted files do not belong to a commit patch"
        );
        let added = diff
            .lines()
            .position(|line| line == if row == 0 { "+two" } else { "+2" })
            .unwrap();
        assert!(!test.editor.buffer().highlights_for_line(added).is_empty());
        test.keys("gT");
        assert_eq!(test.editor.buffer().id(), source_id);
        assert_eq!(*test.editor.buffer().cursor(), source_cursor);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn blame_bands_color_every_row_in_a_commit_group_and_ignore_empty_wrap_rows() {
    use ovim::editor::handle_mouse_event;
    use ovim::ui::Renderer;
    use ovim_core::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{backend::TestBackend, Terminal};
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    test.editor.options.blame = true;
    test.editor.options.wrap = true;
    test.editor.buffer_mut().load_git_blame();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut Default::default()))
        .unwrap();
    let area = test.editor.render_cache.last_buffer_area.unwrap();
    let screen = terminal.backend().buffer();
    // Lines 1 and 3 are from the same original commit; both have a colored band.
    let first = &screen[(area.x, area.y)];
    let third = &screen[(area.x, area.y + 2)];
    assert_eq!(first.bg, third.bg);
    assert_eq!(first.fg, third.fg);
    assert_ne!(first.bg, screen[(area.x + 90, area.y)].bg);
    let tabs = test.editor.tab_count();
    handle_mouse_event(
        &mut test.editor,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            row: area.y + 20,
            column: area.x + 1,
        },
    )
    .unwrap();
    assert_eq!(test.editor.tab_count(), tabs);
    assert!(test.editor.hover_info().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn wrapped_blame_continuations_keep_the_commit_band_and_click_target() {
    use ovim::editor::handle_mouse_event;
    use ovim::ui::Renderer;
    use ovim_core::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{backend::TestBackend, Terminal};
    let fixture = Fixture::new();
    fs::write(
        fixture.root.join("a.txt"),
        format!("{}\nsecond\n", "long text ".repeat(30)),
    )
    .unwrap();
    let repo = Repository::open(&fixture.root).unwrap();
    let oid = commit_all(&repo, "long line");
    let mut test = open_editor_on(&fixture, "a.txt");
    test.editor.options.blame = true;
    test.editor.options.wrap = true;
    test.editor.buffer_mut().load_git_blame();
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    terminal
        .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut Default::default()))
        .unwrap();
    let area = test.editor.render_cache.last_buffer_area.unwrap();
    let screen = terminal.backend().buffer();
    assert_eq!(screen[(area.x, area.y)].bg, screen[(area.x, area.y + 1)].bg);
    handle_mouse_event(
        &mut test.editor,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            row: area.y + 1,
            column: area.x + 1,
        },
    )
    .unwrap();
    assert!(test
        .editor
        .buffer()
        .rope()
        .to_string()
        .starts_with(&format!("commit {oid}\n")));
}

#[test]
fn diff_scroll_indicator_tracks_wrapped_and_unwrapped_views_without_covering_text() {
    use ovim::ui::Renderer;
    use ratatui::{backend::TestBackend, Terminal};
    for wrap in [false, true] {
        let patch = (0..100)
            .map(|index| format!("+{index} {}\n", "content ".repeat(15)))
            .collect::<String>();
        let mut test = EditorTest::new(&patch);
        test.editor
            .buffer_mut()
            .enable_syntax_highlighting_for_path("commit.diff");
        test.editor.options.wrap = wrap;
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let draw = |test: &mut EditorTest, terminal: &mut Terminal<TestBackend>| {
            terminal
                .draw(|frame| {
                    Renderer::render_to_frame(frame, &mut test.editor, &mut Default::default())
                })
                .unwrap();
        };
        draw(&mut test, &mut terminal);
        let area = test.editor.render_cache.last_buffer_area.unwrap();
        let rail = area.x + area.width;
        assert_eq!(rail, 79);
        assert_eq!(terminal.backend().buffer()[(rail, area.y)].symbol(), "┃");
        assert_eq!(
            terminal.backend().buffer()[(rail, area.y + area.height - 1)].symbol(),
            "│"
        );
        assert_eq!(
            test.editor.render_cache.last_text_width,
            ovim::frontend::compute_text_width(&test.editor, 80)
        );
        test.keys("G$");
        draw(&mut test, &mut terminal);
        assert_eq!(terminal.backend().buffer()[(rail, area.y)].symbol(), "│");
        assert_eq!(
            terminal.backend().buffer()[(rail, area.y + area.height - 1)].symbol(),
            "┃"
        );
        assert!(terminal.get_cursor_position().unwrap().x < rail);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn w_hides_equal_same_file_pairs_in_both_terminal_layouts_without_changing_the_snapshot() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("a.txt"), "one\n two \nTHREE\n").unwrap();
    fs::remove_file(fixture.root.join("b.txt")).unwrap();
    let snapshot = ReviewSnapshot::from_patch(
        review_patch(&fixture.root, &ReviewBase::explicit("main")).unwrap(),
    )
    .unwrap();
    let old = snapshot
        .blocks
        .iter()
        .find(|block| block.kind == PatchLineKind::Removed)
        .unwrap();
    let new = snapshot
        .blocks
        .iter()
        .find(|block| block.kind == PatchLineKind::Added)
        .unwrap();
    let custom = snapshot
        .reassign(&[DiffPairing {
            message: None,
            label: Some("Same-file pair".into()),
            old: ChangeRef {
                block_id: old.id.clone(),
                offset: None,
                count: None,
            }
            .into(),
            new: ChangeRef {
                block_id: new.id.clone(),
                offset: None,
                count: None,
            }
            .into(),
            related_to: None,
        }])
        .unwrap();
    let mut test = open_editor_on(&fixture, "a.txt");
    test.editor
        .open_custom_diff_review("Equal and real changes", custom.clone())
        .unwrap();
    assert!(test.editor.buffer().rope().to_string().contains("two"));
    test.press_with(KeyCode::Char('w'), ovim_core::Modifiers::CONTROL);
    test.press_esc();
    assert!(test.editor.buffer().rope().to_string().contains("two"));
    test.keys("w");
    for columns in [100, 80] {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(columns, 40)).unwrap();
        terminal
            .draw(|frame| {
                ovim::ui::Renderer::render_to_frame(
                    frame,
                    &mut test.editor,
                    &mut Default::default(),
                )
            })
            .unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(screen.contains("Hide equal changes: on"), "{screen}");
        assert!(screen.contains("THREE"), "{screen}");
        let text = test.editor.buffer().rope().to_string();
        assert!(!text.contains("two"), "{text}");
        assert!(text.contains("three") && text.contains("THREE"), "{text}");
        let retained = test.editor.diff_review().unwrap().custom().unwrap();
        assert_eq!(retained, &custom);
        retained.validate_coverage().unwrap();
        let mapping = test.editor.diff_review().unwrap().patch_review_lines();
        for line in retained.sections.iter().flat_map(|section| &section.lines) {
            if line.text.trim() == "two" {
                assert!(mapping[line.source_patch_line].is_none());
            }
            if line.text == "THREE" {
                let row = mapping[line.source_patch_line].expect("real edit has a source mapping");
                assert!(text.lines().nth(row).unwrap().contains("THREE"));
            }
        }

        test.keys("]c");
        test.editor.refresh_diff_review();
        assert!(!test.editor.buffer().rope().to_string().contains("two"));
        test.keys("s");
    }
    test.keys("w");
    assert!(test.editor.buffer().rope().to_string().contains("two"));
    test.keys("wq");
    test.editor
        .open_custom_diff_review("Replay", custom)
        .unwrap();
    assert!(!test.editor.buffer().rope().to_string().contains("two"));
}

#[tokio::test(flavor = "multi_thread")]
async fn w_hides_equal_cross_file_pairs_in_both_terminal_layouts_without_changing_the_snapshot() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("moved.txt"), "one\n two \nTHREE\n").unwrap();
    fs::remove_file(fixture.root.join("a.txt")).unwrap();
    fs::remove_file(fixture.root.join("b.txt")).unwrap();
    let snapshot = ReviewSnapshot::from_patch(
        review_patch(&fixture.root, &ReviewBase::explicit("main")).unwrap(),
    )
    .unwrap();
    let old = snapshot
        .blocks
        .iter()
        .find(|block| block.kind == PatchLineKind::Removed)
        .unwrap();
    let new = snapshot
        .blocks
        .iter()
        .find(|block| block.kind == PatchLineKind::Added)
        .unwrap();
    let custom = snapshot
        .reassign(&[DiffPairing {
            message: None,
            label: Some("Cross-file pair".into()),
            old: ChangeRef {
                block_id: old.id.clone(),
                offset: None,
                count: None,
            }
            .into(),
            new: ChangeRef {
                block_id: new.id.clone(),
                offset: None,
                count: None,
            }
            .into(),
            related_to: None,
        }])
        .unwrap();
    let mut test = open_editor_on(&fixture, "moved.txt");
    test.editor
        .open_custom_diff_review("Equal and real changes", custom.clone())
        .unwrap();
    assert!(test.editor.buffer().rope().to_string().contains("two"));
    test.press_with(KeyCode::Char('w'), ovim_core::Modifiers::CONTROL);
    test.press_esc();
    assert!(test.editor.buffer().rope().to_string().contains("two"));
    test.keys("w");
    for columns in [100, 80] {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(columns, 40)).unwrap();
        terminal
            .draw(|frame| {
                ovim::ui::Renderer::render_to_frame(
                    frame,
                    &mut test.editor,
                    &mut Default::default(),
                )
            })
            .unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(screen.contains("Hide equal changes: on"), "{screen}");
        assert!(screen.contains("THREE"), "{screen}");
        let text = test.editor.buffer().rope().to_string();
        assert!(!text.contains("two"), "{text}");
        assert!(text.contains("three") && text.contains("THREE"), "{text}");
        let retained = test.editor.diff_review().unwrap().custom().unwrap();
        assert_eq!(retained, &custom);
        retained.validate_coverage().unwrap();
        let mapping = test.editor.diff_review().unwrap().patch_review_lines();
        for line in retained.sections.iter().flat_map(|section| &section.lines) {
            if line.text.trim() == "two" {
                assert!(mapping[line.source_patch_line].is_none());
            }
            if line.text == "THREE" {
                let row = mapping[line.source_patch_line].expect("real edit has a source mapping");
                assert!(text.lines().nth(row).unwrap().contains("THREE"));
            }
        }

        test.keys("]c");
        test.editor.refresh_diff_review();
        assert!(!test.editor.buffer().rope().to_string().contains("two"));
        test.keys("s");
    }
    test.keys("w");
    assert!(test.editor.buffer().rope().to_string().contains("two"));
    test.keys("wq");
    test.editor
        .open_custom_diff_review("Replay", custom)
        .unwrap();
    assert!(!test.editor.buffer().rope().to_string().contains("two"));
}

#[tokio::test(flavor = "multi_thread")]
async fn entirely_equal_terminal_review_has_no_phantom_navigation_and_can_be_restored() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("a.txt"), "one\n two \nthree\n").unwrap();
    fs::remove_file(fixture.root.join("b.txt")).unwrap();
    let snapshot = ReviewSnapshot::from_patch(
        review_patch(&fixture.root, &ReviewBase::explicit("main")).unwrap(),
    )
    .unwrap();
    let custom = snapshot.reassign(&[]).unwrap();
    let mut test = open_editor_on(&fixture, "a.txt");
    test.editor
        .open_custom_diff_review("Equal pair", custom)
        .unwrap();
    test.keys("w");
    assert!(test
        .editor
        .buffer()
        .rope()
        .to_string()
        .contains("No unequal changes"));
    assert!(!test.editor.buffer().rope().to_string().contains("two"));
    test.keys("]c]f[c[f");
    assert!(test.editor.buffer().cursor().line() < test.editor.buffer().line_count());
    test.keys("sw");
    assert!(test.editor.buffer().rope().to_string().contains("two"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn captured_context_expands_in_both_layouts_without_duplicates_or_moving_the_cursor() {
    use ovim_core::native_diff::context::DiffContext;
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let repo = Repository::init(&root).unwrap();
    repo.set_head("refs/heads/main").unwrap();
    let original: Vec<_> = (1..=100).map(|i| format!("source_{i:03}();")).collect();
    fs::write(root.join("code.rs"), original.join("\n") + "\n").unwrap();
    commit_all(&repo, "base");
    let mut changed = original.clone();
    changed[29] = "changed_030();".into();
    changed[59] = "changed_060();".into();
    fs::write(root.join("code.rs"), changed.join("\n") + "\n").unwrap();
    let snapshot = review_snapshot(&root, &ReviewBase::explicit("HEAD")).unwrap();
    let mut context = DiffContext::new(&snapshot, None);
    let ids: Vec<_> = context.regions(false).map(|r| r.id.clone()).collect();
    assert_eq!(ids.len(), 2);
    assert!(context.expand(&snapshot, &ids[0], false));
    assert!(context.expand(&snapshot, &ids[1], true));
    for _ in 0..20 {
        context.expand(&snapshot, &ids[0], false);
        context.expand(&snapshot, &ids[1], true);
    }
    let first = context.view(&snapshot, &ids[0]).unwrap();
    let second = context.view(&snapshot, &ids[1]).unwrap();
    assert!(!first.can_expand_down && !second.can_expand_up);
    let left: Vec<_> = first.after.old.iter().map(|l| l.number).collect();
    assert!(second.before.old.iter().all(|l| !left.contains(&l.number)));
    let mut unavailable = snapshot.clone();
    unavailable.sources.clear();
    let missing = DiffContext::new(&unavailable, None);
    assert!(!missing.view(&unavailable, &ids[0]).unwrap().can_expand_up);

    for curated in [false, true] {
        for split in [false, true] {
            // Recreate the captured source before opening the live comparison.
            fs::write(root.join("code.rs"), changed.join("\n") + "\n").unwrap();
            let mut test = EditorTest::new("");
            test.editor.open_file(root.join("code.rs")).unwrap();
            let custom = snapshot.reassign(&[]).unwrap();
            if curated {
                test.editor
                    .open_custom_diff_review("Context", custom.clone())
                    .unwrap();
            } else {
                test.editor.open_diff_review(Some("HEAD")).unwrap();
            }
            if split {
                test.keys("s");
            }
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 40)).unwrap();
            terminal
                .draw(|frame| {
                    ovim::ui::Renderer::render_to_frame(
                        frame,
                        &mut test.editor,
                        &mut Default::default(),
                    )
                })
                .unwrap();
            let text = test.editor.buffer().rope().to_string();
            assert!(!text.contains("source_020"));
            let line = text
                .lines()
                .position(|line| line.contains("changed_030"))
                .unwrap();
            test.editor
                .buffer_mut()
                .cursor_mut()
                .set_position(line, ovim_core::unicode::GraphemeCol(0));
            let row = line.saturating_sub(test.editor.scroll_offset());
            // Expansion must never reread this now-different worktree.
            fs::write(root.join("code.rs"), "unrelated live content\n").unwrap();
            test.keys("K");
            assert!(test
                .editor
                .buffer()
                .rope()
                .to_string()
                .contains("source_020"));
            assert!(current_line(&test).contains("changed_030"));
            assert_eq!(
                test.editor
                    .buffer()
                    .cursor()
                    .line()
                    .saturating_sub(test.editor.scroll_offset()),
                row
            );
            test.keys("J");
            let expanded = test.editor.buffer().rope().to_string();
            assert!(expanded.contains("source_040"));
            assert!(!expanded.contains("unrelated live content"));
            terminal
                .draw(|frame| {
                    ovim::ui::Renderer::render_to_frame(
                        frame,
                        &mut test.editor,
                        &mut Default::default(),
                    )
                })
                .unwrap();
            let screen: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(screen.contains("changed_030"), "{screen}");
            if curated {
                assert_eq!(
                    test.editor.diff_review().unwrap().custom().unwrap(),
                    &custom
                );
            }
            let context_row = expanded
                .lines()
                .position(|line| line.contains("source_020"))
                .unwrap();
            test.editor
                .buffer_mut()
                .cursor_mut()
                .set_position(context_row, ovim_core::unicode::GraphemeCol(0));
            test.editor.diff_review_open_at_cursor();
            assert!(test
                .editor
                .buffer()
                .rope()
                .to_string()
                .contains("source_020"));
            assert!(!test
                .editor
                .buffer()
                .rope()
                .to_string()
                .contains("unrelated live content"));
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn paired_context_stays_with_each_file_and_stops_at_capture_gaps() {
    use ovim_core::native_diff::{context::DiffContext, SourceWindow};
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let repo = Repository::init(&root).unwrap();
    repo.set_head("refs/heads/main").unwrap();
    let old: Vec<_> = (1..=60).map(|i| format!("old_source_{i:03}();")).collect();
    let new: Vec<_> = (1..=80).map(|i| format!("new_source_{i:03}();")).collect();
    fs::write(root.join("old.rs"), old.join("\n") + "\n").unwrap();
    fs::write(root.join("new.rs"), new.join("\n") + "\n").unwrap();
    commit_all(&repo, "base");
    let mut old_after = old.clone();
    old_after.remove(29);
    let mut new_after = new.clone();
    new_after.insert(49, "old_source_030();".into());
    fs::write(root.join("old.rs"), old_after.join("\n") + "\n").unwrap();
    fs::write(root.join("new.rs"), new_after.join("\n") + "\n").unwrap();
    let snapshot = review_snapshot(&root, &ReviewBase::explicit("HEAD")).unwrap();
    let old_block = snapshot
        .blocks
        .iter()
        .find(|b| b.kind == PatchLineKind::Removed)
        .unwrap();
    let new_block = snapshot
        .blocks
        .iter()
        .find(|b| b.kind == PatchLineKind::Added)
        .unwrap();
    let custom = snapshot
        .reassign(&[DiffPairing {
            message: None,
            label: Some("Moved statement".into()),
            old: ChangeRef {
                block_id: old_block.id.clone(),
                offset: None,
                count: None,
            }
            .into(),
            new: ChangeRef {
                block_id: new_block.id.clone(),
                offset: None,
                count: None,
            }
            .into(),
            related_to: None,
        }])
        .unwrap();
    let id = format!("section:{}", custom.sections[0].id);
    let mut partial = snapshot.clone();
    for source in partial
        .sources
        .iter_mut()
        .flat_map(|file| [&mut file.old, &mut file.new])
        .flatten()
    {
        source.complete = false;
        source.windows = vec![SourceWindow {
            start_line: 28,
            lines: source.windows[0].lines[27..32].to_vec(),
        }];
    }
    let mut context = DiffContext::new(&partial, Some(&custom));
    assert!(context.expand(&partial, &id, true));
    let view = context.view(&partial, &id).unwrap();
    assert_eq!(
        view.before.old.iter().map(|l| l.number).collect::<Vec<_>>(),
        [28, 29]
    );
    assert!(view.before.new.is_empty());
    assert!(!view.can_expand_up);
    assert!(!context.expand(&partial, "not-a-region", true));

    for split in [false, true] {
        let mut test = EditorTest::new("");
        test.editor.open_file(root.join("old.rs")).unwrap();
        test.editor
            .open_custom_diff_review("Move", custom.clone())
            .unwrap();
        if split {
            test.keys("s");
        }
        assert!(test.editor.expand_diff_context(&id, true));
        assert!(test.editor.expand_diff_context(&id, false));
        let text = test.editor.buffer().rope().to_string();
        assert!(text.contains("old_source_020"));
        assert!(text.contains("new_source_040"));
        assert_eq!(
            test.editor.diff_review().unwrap().custom().unwrap(),
            &custom
        );
        if !split {
            assert!(
                text.find("old_source_040").unwrap() < text.find("new_source_040").unwrap(),
                "old surrounding code must finish before new source begins: {text}"
            );
        }
        for _ in 0..20 {
            test.editor.expand_diff_context(&id, true);
            test.editor.expand_diff_context(&id, false);
        }
        let view = test
            .editor
            .diff_review()
            .unwrap()
            .context_view(&id)
            .unwrap();
        assert!(!view.can_expand_up && !view.can_expand_down);
        assert_eq!(view.before.old.first().unwrap().number, 1);
        assert_eq!(view.after.old.last().unwrap().number, 60);
        assert_eq!(view.after.new.last().unwrap().number, 81);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn refined_diffs_survive_independent_editors_only_for_verified_comparisons() {
    use ovim_core::native_diff::store::{same_comparison, ReviewStore};
    let fixture = Fixture::new();
    let storage = tempfile::tempdir().unwrap();
    let store = ReviewStore::new(storage.path().join("reviews"));
    let first = review_snapshot(&fixture.root, &ReviewBase::explicit("main")).unwrap();
    let custom = first.reassign(&[]).unwrap();
    {
        let mut terminal = open_editor_on(&fixture, "a.txt");
        terminal.editor.set_diff_review_store(Some(store.clone()));
        terminal
            .editor
            .open_custom_diff_review("Saved refinement", custom.clone())
            .unwrap();
    } // no exit hook or live instance required
    let mut gui = open_editor_on(&fixture, "a.txt");
    gui.editor.set_diff_review_store(Some(store.clone()));
    gui.editor.open_diff_review(Some("main")).unwrap();
    assert_eq!(gui.editor.diff_review().unwrap().custom(), Some(&custom));
    assert_eq!(
        gui.editor.diff_review_overlay_state().unwrap().mode,
        "active"
    );
    assert!(gui
        .editor
        .buffer()
        .rope()
        .to_string()
        .contains("Saved refinement"));
    gui.editor.toggle_diff_review_overlay().unwrap();
    assert_eq!(
        gui.editor.diff_review_overlay_state().unwrap().mode,
        "available"
    );
    gui.editor.refresh_diff_review();
    assert_eq!(
        gui.editor.diff_review_overlay_state().unwrap().mode,
        "available",
        "disk reload must respect the current instance's toggle"
    );

    fs::write(fixture.root.join("a.txt"), "another comparison\n").unwrap();
    let second = review_snapshot(&fixture.root, &ReviewBase::explicit("main")).unwrap();
    assert!(!same_comparison(&first, &second));
    let mut changed = open_editor_on(&fixture, "a.txt");
    changed.editor.set_diff_review_store(Some(store.clone()));
    changed.editor.open_diff_review(Some("main")).unwrap();
    assert!(changed.editor.diff_review().unwrap().custom().is_none());
    assert_eq!(
        changed.editor.diff_review_overlay_state().unwrap().mode,
        "stale"
    );
    changed.editor.open_saved_diff_overlay().unwrap();
    assert_eq!(
        changed.editor.diff_review().unwrap().custom(),
        Some(&custom)
    );
    assert_eq!(
        changed.editor.diff_review_overlay_state().unwrap().mode,
        "saved"
    );
    let second_custom = second.reassign(&[]).unwrap();
    changed
        .editor
        .open_custom_diff_review("Second refinement", second_custom)
        .unwrap();
    // Returning to an older comparison recovers its own refinement, not just the latest.
    fs::write(fixture.root.join("a.txt"), "one\n2\nthree\nfour\n").unwrap();
    let mut returned = open_editor_on(&fixture, "a.txt");
    returned.editor.set_diff_review_store(Some(store.clone()));
    returned.editor.open_diff_review(Some("main")).unwrap();
    assert_eq!(
        returned.editor.diff_review().unwrap().custom(),
        Some(&custom)
    );
    // A different comparison selector must not silently reuse this view.
    returned.editor.open_diff_review(Some("HEAD")).unwrap();
    assert!(returned.editor.diff_review().unwrap().custom().is_none());

    let other = Fixture::new();
    let mut isolated = open_editor_on(&other, "a.txt");
    isolated.editor.set_diff_review_store(Some(store));
    isolated.editor.open_diff_review(Some("main")).unwrap();
    assert!(isolated.editor.diff_review().unwrap().custom().is_none());
    assert_eq!(
        isolated.editor.diff_review_overlay_state().unwrap().mode,
        "none"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn persistent_diff_verification_covers_bytes_outside_excerpts_and_legacy_reviews_fail_closed()
{
    use ovim_core::native_diff::store::{same_comparison, ReviewStore};
    let fixture = Fixture::new();
    let storage = tempfile::tempdir().unwrap();
    let store = ReviewStore::new(storage.path().join("reviews"));
    fs::write(fixture.root.join("binary.bin"), b"\0binary one").unwrap();
    let first = review_snapshot(&fixture.root, &ReviewBase::explicit("main")).unwrap();
    assert!(first.content_fingerprint.is_some());
    let mut changed = first.clone();
    fs::write(fixture.root.join("binary.bin"), b"\0binary two").unwrap();
    let second = review_snapshot(&fixture.root, &ReviewBase::explicit("main")).unwrap();
    // Even a coincidentally equal presentation is insufficient for binary content.
    changed.content_fingerprint = second.content_fingerprint.clone();
    assert!(!same_comparison(&first, &changed));
    assert!(!same_comparison(&first, &second));
    fs::write(fixture.root.join("binary.bin"), b"\0binary one").unwrap();
    fs::write(fixture.root.join("a.txt"), "one\r\n2\r\nthree\r\nfour\r\n").unwrap();
    let crlf = review_snapshot(&fixture.root, &ReviewBase::explicit("main")).unwrap();
    assert_ne!(first.content_fingerprint, crlf.content_fingerprint);
    assert!(!same_comparison(&first, &crlf));
    fs::write(fixture.root.join("a.txt"), "one\n2\nthree\nfour\n").unwrap();
    let mut legacy = first.clone();
    legacy.content_fingerprint = None;
    store
        .save("Older saved diff", &legacy.reassign(&[]).unwrap())
        .unwrap();
    let mut editor = open_editor_on(&fixture, "a.txt");
    editor.editor.set_diff_review_store(Some(store));
    editor.editor.open_diff_review(Some("main")).unwrap();
    assert!(editor.editor.diff_review().unwrap().custom().is_none());
    assert_eq!(
        editor.editor.diff_review_overlay_state().unwrap().mode,
        "stale"
    );
    editor.editor.open_saved_diff_overlay().unwrap();
    assert!(editor.editor.diff_review().unwrap().custom().is_some());
}

#[test]
fn comparison_proof_covers_large_source_beyond_captured_context() {
    use ovim_core::native_diff::store::same_comparison;
    let fixture = Fixture::new();
    let repo = Repository::open(&fixture.root).unwrap();
    let mut lines: Vec<_> = (0..90_000)
        .map(|i| format!("original source line {i}"))
        .collect();
    fs::write(fixture.root.join("large.txt"), lines.join("\n")).unwrap();
    commit_all(&repo, "large source");
    lines[100] = "first actual change".into();
    fs::write(fixture.root.join("large.txt"), lines.join("\n")).unwrap();
    let first = review_snapshot(&fixture.root, &ReviewBase::explicit("HEAD")).unwrap();
    let source = first.source_for("large.txt", "new").unwrap();
    assert!(!source.complete);
    assert!(source.line(50_001).is_none());
    assert!(first.content_fingerprint.is_some());
    lines[50_000] = "change outside previously captured context".into();
    fs::write(fixture.root.join("large.txt"), lines.join("\n")).unwrap();
    let second = review_snapshot(&fixture.root, &ReviewBase::explicit("HEAD")).unwrap();
    assert_ne!(first.content_fingerprint, second.content_fingerprint);
    assert!(!same_comparison(&first, &second));
}

#[test]
fn truncated_diff_still_opens_as_a_display_snapshot_without_restoration_proof() {
    let fixture = Fixture::new();
    fs::write(
        fixture.root.join("large.txt"),
        "added source line\n".repeat(300_000),
    )
    .unwrap();
    let snapshot = ovim_core::native_diff::review_display_snapshot(
        &fixture.root,
        &ReviewBase::explicit("HEAD"),
    )
    .unwrap();
    assert!(snapshot.patch.truncated);
    assert!(snapshot.content_fingerprint.is_none());
    assert!(review_snapshot(&fixture.root, &ReviewBase::explicit("HEAD")).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn checking_files_hides_only_review_content_and_survives_refresh_and_reopen() {
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    test.keys(" gd");
    let original = test.editor.diff_review().unwrap().patch().clone();
    test.keys("]fx");
    assert!(test
        .editor
        .diff_review()
        .unwrap()
        .checked_items
        .contains("file:a.txt"));
    assert!(!buffer_text(&test).contains("diff --git a/a.txt"));
    assert!(buffer_text(&test).contains("diff --git a/b.txt"));
    assert_eq!(test.editor.diff_review().unwrap().patch(), &original);
    test.keys("sr");
    assert!(!buffer_text(&test).contains("one"));
    test.keys("q gd");
    assert!(test
        .editor
        .diff_review()
        .unwrap()
        .checked_items
        .contains("file:a.txt"));
    test.keys("X");
    assert!(buffer_text(&test).contains("[x] checked"));
    test.editor.toggle_diff_review_check("file:a.txt").unwrap();
    assert!(test.editor.diff_review().unwrap().checked_items.is_empty());
    assert_eq!(
        fs::read_to_string(fixture.root.join("a.txt")).unwrap(),
        "one\n2\nthree\nfour\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn changed_checked_file_reappears_while_unchanged_file_stays_checked() {
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    test.keys(" gd");
    for path in ["a.txt", "b.txt"] {
        test.editor
            .toggle_diff_review_check(&format!("file:{path}"))
            .unwrap();
    }
    assert!(buffer_text(&test).contains("All visible changes checked"));
    fs::write(fixture.root.join("a.txt"), "one\nnew change\nthree\n").unwrap();
    test.keys("r");
    let checked = &test.editor.diff_review().unwrap().checked_items;
    assert!(!checked.contains("file:a.txt"));
    assert!(checked.contains("file:b.txt"));
    assert!(buffer_text(&test).contains("+new change"));
    assert!(!buffer_text(&test).contains("diff --git a/b.txt"));
    test.keys("]c");
    assert!(current_line(&test).starts_with("@@"));
    assert!(test
        .editor
        .toggle_diff_review_check("section:missing")
        .is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn checked_custom_sections_keep_context_and_source_mapping_in_both_layouts() {
    let fixture = Fixture::new();
    let snapshot = review_snapshot(&fixture.root, &ReviewBase::explicit("main")).unwrap();
    let custom = snapshot.reassign(&[]).unwrap();
    assert!(custom.sections.len() >= 2);
    let first = format!("section:{}", custom.sections[0].id);
    for layout in [
        ovim_core::editor::DiffLayout::Unified,
        ovim_core::editor::DiffLayout::Split,
    ] {
        let mut test = open_editor_on(&fixture, "a.txt");
        test.editor
            .open_custom_diff_review("Review", custom.clone())
            .unwrap();
        test.editor.set_diff_review_layout(layout);
        test.keys("]cx");
        assert!(test
            .editor
            .diff_review()
            .unwrap()
            .checked_items
            .contains(&first));
        assert_eq!(test.editor.diff_review().unwrap().custom(), Some(&custom));
        test.keys("rX");
        assert!(buffer_text(&test).contains("[x] checked"));
        test.editor.toggle_diff_review_check(&first).unwrap();
        assert!(test.editor.diff_review().unwrap().checked_items.is_empty());
        test.keys("gg]cJ");
        assert!(!buffer_text(&test).contains("All visible changes checked"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn checks_agree_between_file_and_guided_views() {
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    let custom = review_snapshot(&fixture.root, &ReviewBase::explicit("main"))
        .unwrap()
        .reassign(&[])
        .unwrap();
    let section = custom
        .sections
        .iter()
        .find(|section| section.new_path.as_deref() == Some("a.txt"))
        .unwrap();
    let id = format!("section:{}", section.id);
    test.editor
        .open_custom_diff_review("Review", custom)
        .unwrap();
    test.editor.toggle_diff_review_check("file:a.txt").unwrap();
    assert!(test
        .editor
        .diff_review()
        .unwrap()
        .checked_items
        .contains(&id));
    test.editor.toggle_diff_review_check(&id).unwrap();
    assert!(!test
        .editor
        .diff_review()
        .unwrap()
        .checked_items
        .contains("file:a.txt"));
    test.editor.toggle_diff_review_check(&id).unwrap();
    assert!(test
        .editor
        .diff_review()
        .unwrap()
        .checked_items
        .contains("file:a.txt"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn diff_definition_uses_source_column_and_rejects_stale_and_unknown_lines() {
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    test.keys(" gd");
    assert!(test
        .editor
        .diff_review_goto_definition("../outside", 1, "new", 0)
        .is_err());
    assert!(test
        .editor
        .diff_review_goto_definition("a.txt", 999, "new", 0)
        .is_err());
    assert!(test
        .editor
        .diff_review_goto_definition("a.txt", 2, "old", 1)
        .is_err());
    test.editor
        .diff_review_goto_definition("a.txt", 4, "new", 2)
        .unwrap();
    assert!(!test.editor.is_diff_review_buffer());
    assert_eq!(test.editor.buffer().cursor().line(), 3);
    assert_eq!(test.editor.buffer().cursor().col().0, 2);
    test.keys(" gd");
    fs::write(fixture.root.join("a.txt"), "other\nnew\ncontent\nchanged\n").unwrap();
    assert!(test
        .editor
        .diff_review_goto_definition("a.txt", 4, "new", 2)
        .is_err());
    assert!(test.editor.is_diff_review_buffer());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn section_checks_survive_removing_the_custom_overlay() {
    let fixture = Fixture::new();
    let mut test = open_editor_on(&fixture, "a.txt");
    let custom = review_snapshot(&fixture.root, &ReviewBase::explicit("main"))
        .unwrap()
        .reassign(&[])
        .unwrap();
    let id = format!(
        "section:{}",
        custom
            .sections
            .iter()
            .find(|section| section.new_path.as_deref() == Some("a.txt"))
            .unwrap()
            .id
    );
    test.editor
        .open_custom_diff_review("Review", custom)
        .unwrap();
    test.editor.toggle_diff_review_check(&id).unwrap();
    test.editor.return_to_live_diff_review().unwrap();
    test.editor.toggle_diff_review_overlay().unwrap();
    assert!(test.editor.diff_review().unwrap().custom().is_none());
    assert!(test
        .editor
        .diff_review()
        .unwrap()
        .checked_items
        .contains("file:a.txt"));
    assert!(!buffer_text(&test).contains("diff --git a/a.txt"));
    test.editor.toggle_diff_review_check("file:a.txt").unwrap();
    test.editor.toggle_diff_review_overlay().unwrap();
    assert!(!test
        .editor
        .diff_review()
        .unwrap()
        .checked_items
        .contains(&id));
}
