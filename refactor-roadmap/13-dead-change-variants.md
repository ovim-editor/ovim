# 13: Remove Dead `Change` Variants

**Goal:** Delete four `Change` enum variants that are defined but never constructed, along with their unreachable `apply()`, `undo()`, `repeat()`, and accessor implementations.

**Fixes:** ~400 lines of dead code in `change.rs`. Reduces cognitive load when reading the undo/repeat system.

**Risk:** None. These variants are never instantiated — the compiler just doesn't know because they're reachable through enum match arms.

## The Evidence

The audit traced every call site of `push_change()`, `add_change()`, `push_recorded_undo()`, and `record_operation()`. Four `Change` variants have constructor methods that are **never called anywhere in the codebase**:

| Variant | Constructor | Lines in `change.rs` | Migrated to |
|---------|-------------|---------------------|-------------|
| `ChangeTextObject` | `change_text_object()` | 149–159 | `RepeatAction::Change` (delete phase) + insert-mode composite |
| `ChangeWord` | `change_word()` | 163–169 | `RepeatAction::Change` (delete phase) + insert-mode composite |
| `ChangeSearchMatch` | `change_search_match()` | 180–191 | `RepeatAction::Change` (delete phase) + insert-mode composite |
| `ReplaceMode` | `replace_mode()` | 372–386 | `RepeatAction::ReplaceMode` |

These variants were the original Pattern A implementation for change operators (`ci"`, `cw`, `cgn`, `R`). They've been fully replaced by Pattern B — the operations now use `record_operation()` / `push_recorded_undo()` for mechanical undo and `RepeatAction` for semantic repeat.

## What to delete

In `change.rs`:

1. **Enum variants**: `ChangeTextObject`, `ChangeWord`, `ChangeSearchMatch`, `ReplaceMode` (and their struct fields)
2. **Constructor methods**: `change_text_object()`, `change_word()`, `change_search_match()`, `replace_mode()`
3. **Match arms** in `apply()`, `undo()`, `repeat()`, `get_inserted_text()`, `cursor_before()`, `cursor_after()`, `set_cursor_before()`, `set_cursor_after()`, `edit_position()`
4. **Helper method**: `find_text_object()` — only called from `ChangeTextObject`'s `repeat()` arm
5. **Helper method**: `calculate_end_position()` — check if still used by remaining variants before removing

Also in `change.rs`:
6. **`TextObjectType` enum** — After removing `ChangeTextObject`, check whether any remaining code in this file references it. If only `RepeatAction` and `Buffer::delete_text_object()` use it, consider moving the enum to a shared location (roadmap 14 handles this).

## Verification

```bash
# Before: confirm no construction sites exist
cargo grep 'change_text_object\|change_word\|change_search_match\|Change::replace_mode' --include '*.rs'

# After: compiler will catch any missed match arms
cargo build 2>&1 | head -50
cargo test
```

## Files

- `ovim-core/src/change.rs` — all deletions happen here
