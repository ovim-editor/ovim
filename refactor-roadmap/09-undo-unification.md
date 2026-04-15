# 09: Undo System Unification

**Goal:** One undo representation, not two. Eliminate the impedance mismatch between `Change` (line/col positions) and `Edit` (absolute char offsets).

**Fixes:** Latent risk of position misinterpretation in grouped undo operations, simplifies the path to a full edit log for decoration projection.

**Risk:** Medium-high. The undo system is load-bearing and touches every buffer mutation. Must be done incrementally with extensive testing.

## The Problem

Two parallel systems represent buffer mutations:

### `Change` enum (`change.rs`)

```rust
enum Change {
    InsertText { position: (usize, usize), text: String, cursor_before: (usize, usize) },
    DeleteText { range: Range, deleted_text: String, cursor_before: (usize, usize), ... },
    Composite { changes: Vec<Change>, cursor_before, cursor_after, ... },
}
```

- Uses **(line, col)** positions — ambiguous (col can be char or grapheme depending on caller)
- Drives the actual undo/redo via `ChangeManager`
- Created in `push_change()`, stored on undo/redo stacks
- `undo()` computes absolute char offset from line/col at undo time (position depends on current rope state)

### `Edit` enum (`edit.rs`)

```rust
enum Edit {
    Insert { offset: usize, text: String },
    Delete { offset: usize, text: String },
}
```

- Uses **absolute char offsets** — unambiguous
- Captured by `buffer.record()` during any operation
- Used for decoration adjustment (`adjust_for_edits`)
- Used for document sync (LSP incremental changes)
- Has `inverse()` and `apply()` — mechanically reversible

### The mismatch

When `buffer.undo()` runs, it wraps in `record()` to capture edits. But the inner `Change::undo()` uses line/col → char offset conversion at undo time. This means:

1. The `Change` stores `position: (5, 10)` at creation time
2. At undo time, line 5 col 10 maps to char offset X in the current rope
3. `delete_char_range(X, X+len)` is called
4. `record()` captures `Edit::Delete { offset: X, text }` — correct for the current rope

This works today because the conversion happens at undo time against the current rope. But it means the `Change`'s stored position is not the same as the `Edit`'s offset — they're different representations of the same operation, computed at different times, against different rope states.

### Two active undo paths (by design)

Both `push_change()` and `push_recorded_undo()` are actively used:
- `push_change()` — sets the dot-repeat template (`last_change`). Used for user-initiated insert mode composites.
- `push_recorded_undo()` — preserves the existing repeat template. Used for LSP edits, workspace operations, operators, visual mode changes — anything that shouldn't override dot-repeat.

These serve different purposes and shouldn't be collapsed into one.

### Where it could break

- **Grouped undo with position-dependent sub-changes:** Each `Change::undo()` in a `Composite` mutates the rope, so the next sub-change's line/col is interpreted against the mutated state. If a sub-change deletes a line, the next sub-change's line number refers to a different line than when the change was recorded. The arithmetic is correct because `undo()` runs sub-changes in reverse order and the `InsertText` path clamps invalid lines — but the invariant is implicit, not enforced by the type system.

- **Future edit log:** A persistent edit log (for Phase 5 full projection) would need edits keyed by buffer version. `buffer.undo()` wraps in `record()` to capture the edits applied during undo — this is intentional and the captured edits are correct (they reflect the actual mutations in order). But the `Change` and `Edit` remain separate representations of the same operation. The `Change` is the undo record; the `Edit` is the mutation record. They are always consistent because the `Edit` is captured from the `Change`'s execution, not computed independently.

## The Design

### Option A: Migrate `ChangeManager` to use `Edit` + `UndoEntry`

`edit.rs` already has `UndoEntry` (Single or Group of Edits with cursor positions). Replace `Change` with `UndoEntry` as the undo stack element. `push_recorded_undo` in `change_tracking.rs` already creates `UndoEntry::Group` from recorded edits — this is the natural undo record.

**Migration path:**
1. Make `push_change()` store `UndoEntry` instead of `Change`
2. `ChangeManager.undo()` calls `UndoEntry::undo()` which applies inverse edits
3. Delete `Change` enum entirely
4. The `record()` wrapper on `buffer.undo()` becomes unnecessary — undo directly produces edits from the `UndoEntry`

**Risk:** The `Change::Composite` grouping logic and `undo_group_id` need to be preserved. `UndoEntry::Group` already supports this, but the merge/coalesce behavior in `ChangeManager` needs adapting.

### Option B: Keep both, add a clear boundary

Document and enforce: `Change` is the undo record (stores what to undo and where), `Edit` is the sync/decoration record (stores what happened to the rope). They serve different purposes and shouldn't be unified.

The boundary: `Change::undo()` produces `Edit`s as a side effect (via `record()`). `Edit`s never produce `Change`s. The flow is always Change → buffer mutation → Edit capture.

**Risk:** Lower, but preserves the dual-system complexity. Future work (edit log, projection) builds on `Edit` and ignores `Change`.

### Recommendation

Option B for now. The dual system works and unification is high-risk with no user-visible benefit. Document the boundary clearly and add integration tests for composite undo + decoration adjustment to catch the subtle edge cases.

Option A becomes attractive if/when a persistent edit log is needed (Phase 5 full projection). At that point, the edit log IS the undo record, and `Change` becomes redundant.

## Files

- `ovim-core/src/change.rs` — `Change` enum, `ChangeManager`
- `ovim-core/src/edit.rs` — `Edit` enum, `UndoEntry`
- `ovim-core/src/editor/change_tracking.rs` — `record_operation`, `push_recorded_undo`, undo/redo
- `ovim-core/src/buffer/mod.rs` — `buffer.undo()`, `buffer.redo()`, `buffer.record()`
