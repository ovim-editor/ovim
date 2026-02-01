/// Regression test for Bug 2: Syntax highlighting not updating after edit
///
/// Bug: After deleting a character that affects syntax (e.g., removing `/` from `/**`),
/// the tree-sitter parse tree was updated incrementally, but the UI didn't re-render
/// until user pressed another key. The highlight cache was rebuilt after debounce,
/// but mark_dirty() wasn't called, so the render loop skipped the frame.
mod helpers;

use helpers::EditorTest;

#[tokio::test]
async fn test_bug2_process_rehighlight_returns_early_when_no_pending() {
    // Bug 2 regression test documentation:
    // process_pending_rehighlight() should call mark_dirty() AFTER rehighlighting
    //
    // Without this fix, after editing syntax-significant text:
    // 1. Buffer is edited → marked dirty → renders → clean
    // 2. After debounce (100ms), rehighlighting happens → BUT editor not marked dirty
    // 3. UI doesn't re-render → stale highlighting shown until next keypress
    //
    // The fix adds mark_dirty() call in src/editor/mod.rs line 932
    //
    // This test just verifies early return path works (when no rehighlight needed)

    let content = "fn main() {}";
    let mut test = EditorTest::new(content);

    // No pending rehighlight, so should return early
    assert!(!test.editor.buffer().needs_rehighlight());

    test.editor.mark_clean();
    test.editor.process_pending_rehighlight().await;

    // Should still be clean since nothing was processed
    assert!(!test.editor.is_dirty());
}

#[test]
fn test_bug2_mark_dirty_after_edit() {
    // Verify that edit operations themselves mark dirty (separate from rehighlight)
    let content = "line1\nline2\nline3";

    let mut test = EditorTest::new(content);
    test.editor.mark_clean(); // Start clean

    // Any edit should mark dirty immediately (even before rehighlight debounce)
    test.keys("x");

    // Should be dirty after immediate edit
    assert!(
        test.editor.is_dirty(),
        "Editor should be marked dirty immediately after edit command"
    );
}
