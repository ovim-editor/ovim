# Phase 4: Async Save

**Goal:** Save never blocks the event loop. Git operations run on background threads.

**Fixes:** Editor freezes on `:w`.

**Risk:** Low. The file write itself is simple. The complexity is in the edge cases (save-and-quit, concurrent edits).

## The Bug

The save path blocks the event loop thread:

```
commands.rs:save_buffer()              -- sync fn
  file_io.rs:save_as()                -- sync wrapper
    block_in_place(block_on(save_as_async()))   -- blocks tokio thread
    refresh_git_status()              -- sync git2 diff (100-1000ms)
  commands.rs:load_git_blame()        -- sync git2 blame (100-1000ms, conditional)
```

## The Fix

### Priority: git operations first

The file write via `block_in_place` is typically fast (< 10ms for normal files). The git operations are the slow part. The quickest win with the smallest blast radius:

**Move git operations to `spawn_blocking`, keep save synchronous for now.**

```rust
// In save_buffer() (commands.rs):
match editor.buffer_mut().save_as(&resolved) {
    Ok(_) => {
        editor.handle_file_path_transition_after_save(old_path, new_path);
        editor.mark_saved();
        editor.mark_buffer_saved();

        // Git refresh is low-priority background work.
        // Don't block the editor for it.
        let path = resolved.clone();
        let blame_enabled = editor.options.blame;
        let git_tx = editor.git_refresh_tx.clone();
        tokio::task::spawn_blocking(move || {
            let status = ovim_core::git::GitStatus::from_file(&path).ok();
            let blame = if blame_enabled {
                ovim_core::git::GitBlame::from_file(&path).ok()
            } else {
                None
            };
            let _ = git_tx.blocking_send(GitRefreshResult { path, status, blame });
        });

        // ... return success message
    }
}
```

The event loop drains `git_refresh_rx` on each tick and applies the results:

```rust
// In process_editor_tick():
while let Ok(result) = git_refresh_rx.try_recv() {
    if let Some(status) = result.status {
        editor.buffer_mut().set_git_status(status);
    }
    if let Some(blame) = result.blame {
        editor.buffer_mut().set_git_blame(blame);
    }
    editor.mark_dirty();
}
```

This alone eliminates 80-90% of the save freeze. Ship it and see if the remaining `block_in_place` for file I/O is noticeable. On most systems, writing a source file to an SSD takes < 5ms. The `fsync` in `save_as_async` can take 10-50ms on some filesystems, but that's still much better than 1-3s from git.

### If file I/O latency matters: async save

If profiling shows the file write itself is a problem (network filesystems, large files), then make save fully async with `CommandOutcome`:

```rust
pub enum CommandOutcome {
    Done(CommandResult),
    Pending { status: String },
}
```

But this is a bigger change to the command system and adds complexity for save-and-quit. Don't do it unless the `spawn_blocking` for git isn't enough.

### Save-and-quit

`:wq` and `ZZ` need the save to complete before quitting. With synchronous save + background git:

```rust
// :wq just saves (sync) and quits. Git refresh is abandoned.
fn cmd_wq(editor: &mut Editor) -> CommandOutcome {
    let result = save_buffer(editor, SaveOpts { quit_after: true, .. });
    // save_buffer already calls editor.quit() on success
    CommandOutcome::Done(result)
}
```

The git refresh task is spawned but the editor quits before it completes. This is fine -- we don't need updated git status after quitting.

## Files Changed

| File | Change |
|------|--------|
| `ovim-core/src/commands.rs` | Move git refresh to `spawn_blocking` after save |
| `ovim-core/src/editor/mod.rs` | Add `git_refresh_tx` / `git_refresh_rx` channels |
| `ovim/src/event_loop.rs` | Drain `git_refresh_rx` in tick, apply results |
| `ovim-core/src/buffer/file_io.rs` | Remove `refresh_git_status()` from `save_as()` |

## Verification

1. **Responsiveness test:** Open a file in a large git repo. `:w`. Verify input is accepted immediately (cursor blinks, keys echo).
2. **Git status test:** After save, wait 1-2 seconds. Verify gutter signs update (git diff markers).
3. **Save-and-quit test:** `:wq` on a modified file. File is written. Editor exits. (Git refresh doesn't block exit.)
4. **Blame test:** With blame enabled, save. Verify blame annotations update after background refresh completes.
