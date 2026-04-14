# Phase 3: Document Sync

**Goal:** The LSP server always has the correct view of the document. The content pipeline from buffer mutation to didChange notification has no path where stale baselines corrupt the incremental diff.

**Fixes:** Wrong diagnostics after undo. Silent document desync. Reconciliation complexity.

**Risk:** Medium. Touches the debouncer, the content flow, and the sync tracking. Needs careful testing because bugs here are silent (wrong diagnostics, not crashes).

## The Bug

In `ovim-core/src/lsp/notifications.rs:300-302`:

```rust
// Update pending text, version, and old text
debouncer.pending_text = text;
debouncer.pending_version = assigned_version;
// Only set old_text if we don't already have it (first change after sync)
if debouncer.old_text.is_none() {
    debouncer.old_text = old_text;
}
```

`old_text` is only set when `None` -- the first change after a flush. Subsequent changes (including undo) update `pending_text` but leave `old_text` stale.

### Concrete Scenario

1. File starts as `"hello\n"`. LSP server has `"hello\n"`.
2. User types `x` at end: buffer becomes `"hellox\n"`.
   - `did_change()` called: `pending_text = "hellox\n"`, `old_text = "hello\n"` (set because `None`)
   - Debounce timer starts (150ms)
3. Timer fires. `flush_pending_changes_broadcast()` sends incremental diff:
   - `compute_simple_diff("hello\n", "hellox\n")` → insert `x` at line 0, col 5
   - Server now has `"hellox\n"`. Correct.
   - `old_text` reset to `None` (debouncer removed on flush).
4. User types `y`: buffer becomes `"helloxy\n"`.
   - `did_change()`: `pending_text = "helloxy\n"`, `old_text = "hellox\n"` (set because `None`)
5. User undoes: buffer becomes `"hellox\n"`.
   - `did_change()`: `pending_text = "hellox\n"`, `old_text` is **not updated** (still `"hellox\n"`)
   - `old_text == pending_text`!
6. Timer fires:
   - `compute_simple_diff("hellox\n", "hellox\n")` → returns `None` (identical)
   - **No didChange sent.** The server still thinks it has `"helloxy\n"`.
   - Server and editor now disagree. Diagnostics are computed against wrong content.

Step 5-6 is the worst case: the undo brings the buffer back to the same content as `old_text`, so no diff is detected and nothing is sent. The server is stuck with pre-undo content.

### A less severe but more common scenario

If the undo doesn't bring content back to exactly `old_text`, the diff IS sent, but it's computed against the wrong baseline. `compute_simple_diff()` does a line-level diff, so it might accidentally produce the right result for small single-line changes. But for multi-line undos or undos across refactoring, the diff will be wrong.

## Why The Guard Exists

The `if debouncer.old_text.is_none()` guard has a purpose: when the user types rapidly (multiple `did_change()` calls before the 150ms flush), we want `old_text` to stay as the **last flushed content** -- what the server actually has. Without the guard, each keystroke would overwrite `old_text` with the previous keystroke's content, and the diff would only cover the last keystroke, not all accumulated changes.

The problem is that undo is a different kind of change. Normal typing accumulates forward from a baseline. Undo jumps backward. The guard is correct for forward accumulation but wrong for undo.

## The Fix

### Approach: Always pass the correct baseline

The debouncer shouldn't be responsible for tracking what the server has. The editor already tracks this in `DocumentSyncState.last_flushed_content`. The fix is to always pass the correct baseline from the editor side:

```rust
// In send_lsp_changes_if_modified() (lsp_integration.rs:1366-1376):
let old_content = self.lsp.state.document_sync
    .get(&state_key)
    .and_then(|state| state.last_flushed_content.clone());

// This is passed to did_change_broadcast as old_text.
// It represents what the server actually has.
```

This is already what happens. The bug is that the debouncer ignores it on subsequent calls. The simplest fix:

**In the debouncer, always update `old_text` when the caller provides one:**

```rust
// notifications.rs:300-302
// Before:
if debouncer.old_text.is_none() {
    debouncer.old_text = old_text;
}

// After:
if let Some(new_old) = old_text {
    debouncer.old_text = Some(new_old);
}
```

Wait -- this would break the rapid-typing case. If the user types `a`, `b`, `c` before the flush:
1. `a`: `old_text = last_flushed` (correct baseline)
2. `b`: caller passes `old_text = last_flushed` (same, because `last_flushed` hasn't changed)
3. `c`: caller passes `old_text = last_flushed` (still the same)

This works! The caller always passes `last_flushed_content`, which doesn't change until the next flush. So for rapid typing, `old_text` stays as the last flushed content (correct). For undo, `old_text` gets updated to `last_flushed_content` (also correct, because last_flushed is what the server has).

**But only if `last_flushed_content` is accurate.** Let me check...

### Verifying last_flushed_content accuracy

`last_flushed_content` is set in `mark_change_flushed()` (lsp_state.rs:63-84), which is called:
1. In `reconcile_document_sync_with_manager()` when `sent_version >= target_version`
2. In `mark_document_flushed()` after explicit flush in `ensure_lsp_document_synced()`

The tricky case: what if the debouncer has flushed content to the server but `last_flushed_content` hasn't been updated yet? This can happen because:
1. `did_change()` is called, content queued in debouncer
2. Timer fires, `flush_pending_changes_broadcast()` sends to server
3. `last_sent_versions` is updated in LspManager
4. But `DocumentSyncState.last_flushed_content` is only updated on the next tick when `reconcile_document_sync_with_manager()` runs

During the gap between step 3 and step 4, `last_flushed_content` is stale. If a new `did_change()` call happens in this gap, the baseline is wrong.

**This is the real problem.** `last_flushed_content` and the debouncer's `old_text` are two independent attempts to track what the server has, and they can disagree.

### The Real Fix: Single Owner for the Baseline

The debouncer should own the baseline, but it should be updated correctly:

```rust
impl ChangeDebouncer {
    /// Update the pending change. The baseline (old_text) tracks
    /// what the server currently has.
    fn update(&mut self, text: Arc<str>, version: i32, baseline: Option<Arc<str>>) {
        self.pending_text = text;
        self.pending_version = version;
        // The baseline is what the server has. Only update it when
        // the caller provides a fresh one (after a flush completed).
        // Between flushes, the baseline stays constant.
        if let Some(b) = baseline {
            self.old_text = Some(b);
        }
    }
}
```

And on flush, the debouncer's `old_text` is consumed:

```rust
// In flush_pending_changes_broadcast():
let text = debouncer.pending_text.clone();
let old_text = debouncer.old_text.take();  // consumed on flush
// After successful send, old_text would be set to `text` (new baseline)
// But the debouncer is removed on flush, so this is moot.
```

The key insight: **the caller shouldn't pass `old_text` on every call.** It should pass it only when it knows the baseline has changed (after a flush). Between flushes, the debouncer keeps its own `old_text` constant.

But this is exactly what the current code tries to do with `if debouncer.old_text.is_none()` -- it just gets it wrong for undo because the debouncer's old_text hasn't been consumed.

### Simplest Correct Fix

Actually the simplest fix is: **don't try to track the baseline in two places.** The debouncer should always receive the definitive baseline from the caller:

```rust
// In send_lsp_changes_if_modified():
let old_content = self.get_lsp_server_content(&state_key);  // what the server has
lsp.did_change_broadcast(uri, language_id, content, old_content).await;
```

And in the debouncer:

```rust
// Always overwrite. The caller is authoritative.
debouncer.old_text = old_text;
debouncer.pending_text = text;
```

For rapid typing, `old_content` will be `last_flushed_content` on every call -- the same value. For undo, it's also `last_flushed_content`, which is still what the server has. The baseline doesn't change between flushes regardless of what edits happen in the buffer.

The only case where this would be wrong is if a flush happened asynchronously and `last_flushed_content` wasn't updated. To close this gap:

After `flush_pending_changes_broadcast` successfully sends, update `last_flushed_content` immediately (it already does this through the reconciliation path, but we should ensure it's synchronous with the flush).

### What to test

The existing test `undo_marks_lsp_document_modified` verifies the flag. We need a test that verifies the actual content:

```rust
#[tokio::test]
async fn undo_sends_correct_content_to_lsp() {
    let mut t = EditorTest::new("hello\n");
    t.set_file_path("test.rs");
    t.init_lsp_sync();

    // Simulate initial sync
    t.flush_lsp_changes().await;  // server has "hello\n"

    // Edit: add "x"
    t.keys("A").type_text("x").keys("<Esc>");
    t.flush_lsp_changes().await;  // server has "hellox\n"

    // Edit: add "y"
    t.keys("A").type_text("y").keys("<Esc>");
    // DON'T flush yet -- change is queued

    // Undo the "y"
    t.keys("u");
    
    // Now flush -- the server should receive content that produces "hellox\n"
    let sent_content = t.capture_next_did_change().await;
    assert_eq!(sent_content, "hellox\n",
        "After undo, LSP should receive the undone content");
}

#[tokio::test]
async fn undo_through_unflushed_edit_sends_correct_diff() {
    let mut t = EditorTest::new("hello\n");
    t.set_file_path("test.rs");
    t.init_lsp_sync();
    t.flush_lsp_changes().await;  // server has "hello\n"

    // Edit and undo WITHOUT flushing in between
    t.keys("A").type_text("x").keys("<Esc>");
    t.keys("u");  // back to "hello\n"

    // The diff should be against what the server has ("hello\n"),
    // which is identical to current content -- so either no change
    // or a full-content sync should be sent.
    let sent_content = t.capture_next_did_change().await;
    // Either no send (content identical) or correct content
    assert!(sent_content.is_none() || sent_content == Some("hello\n"));
}
```

## Simplifying DocumentSyncState (Opportunistic)

While fixing the baseline bug, simplify the sync state. The reconciliation protocol (`reconcile_document_sync_with_manager`) exists because `DocumentSyncState` and `LspManager` track versions independently. After the fix:

- `last_flushed_content` becomes the single source of truth for "what the server has"
- `last_queued_content` can be removed (the debouncer owns queued content)
- `target_lsp_version` can be simplified (version comparison between buffer version and last-synced version)

Don't do a full rewrite here -- just remove fields that are no longer needed after the `old_text` fix is in place.

## Files Changed

| File | Change |
|------|--------|
| `ovim-core/src/lsp/notifications.rs:300-302` | Always update `old_text` from caller |
| `ovim-core/src/editor/lsp_integration.rs` | Ensure `last_flushed_content` is updated synchronously after flush |
| `ovim-core/src/editor/lsp_state.rs` | Potentially remove `last_queued_content` |
| `ovim/tests/` | New tests: undo content correctness, edit-undo-flush sequence |

## Verification

1. **Undo content test:** Edit, flush, edit, undo, flush. Capture the didChange notification and verify content matches the buffer.
2. **Rapid edit-undo test:** Edit 5 times without flushing, undo 3 times, flush. Server content matches buffer.
3. **Full round-trip test:** Edit, undo, redo, undo, flush. Diagnostics are correct for the final content.
4. **Incremental diff test:** With a server that supports incremental sync, verify the diff range is correct after undo (not a stale range).
