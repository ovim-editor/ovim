# Phase 5: Decoration Projection

**Goal:** Inlay hints don't drift. Undo doesn't wipe all decorations. Stale hints are displayed at corrected positions, not wrong ones.

**Fixes:** Inlay hints shifting left as you type. Hints disappearing on undo.

**Risk:** Medium. Changes the decoration storage model and the rendering pipeline.

## The Problems

### Problem 1: Stale positions on application

When inlay hints arrive at `lsp_integration.rs:763-780`, they're converted to absolute char offsets using the *current* rope. But the LSP computed these positions against a potentially older buffer version. The version check at line 786:

```rust
if result.buffer_version != self.buffer().version() {
    self.invalidate_inlay_hint_debounce();
}
```

...detects the mismatch and schedules a refresh, but still applies the stale hints. The positions are wrong from the moment they're created.

### Problem 2: Arithmetic drift

`adjust_for_edits()` in `decoration.rs:194-222` shifts decoration offsets on each edit. For decorations placed at correct positions, this works. For decorations placed at wrong positions (from Problem 1), each adjustment compounds the error. Type 5 characters before a hint: it shifts 5 positions right from an already-wrong base.

### Problem 3: Undo clears everything

`change_tracking.rs:44` does `self.decorations.clear()` on undo because the undo path doesn't expose the edit list for adjustment. All inlay hints vanish until the next refresh cycle (500ms debounce + round-trip).

## The Design

### Core change: store the source version, project at render time

Instead of converting LSP positions to absolute char offsets against the current rope, store them with the version they were computed against:

```rust
struct VersionedDecoration {
    /// Buffer version this position was computed against.
    source_version: u64,
    /// Absolute char offset in the source version's rope.
    char_offset: usize,
    /// The decoration content and style.
    content: DecorationContent,
    style: DecorationStyle,
}
```

At render time, the decoration is projected from `source_version` to the current version using the edit log from Phase 3:

```rust
fn project_offset(offset: usize, edits: &[Edit]) -> Option<usize> {
    let mut pos = offset;
    for edit in edits {
        match edit {
            Edit::Insert { offset: ins_off, text } => {
                if pos >= *ins_off {
                    pos += text.chars().count();
                }
            }
            Edit::Delete { offset: del_off, text } => {
                let len = text.chars().count();
                let end = del_off + len;
                if pos >= end {
                    pos -= len;
                } else if pos > *del_off {
                    return None;  // position was deleted
                }
            }
        }
    }
    Some(pos)
}
```

This is the same arithmetic as `adjust_for_edits`, but:
- It's computed fresh from the authoritative edit chain, not accumulated
- It's a pure function (no mutation)
- If the source version falls off the edit log, decorations are discarded (too stale)

### Why this fixes all three problems

**Problem 1 (stale positions):** Hints are stored with their source version's char offsets. When the buffer has advanced, projection applies the exact edits that happened since. No wrong baseline.

**Problem 2 (drift):** Projection is recomputed on each render, not accumulated. There's no chain of mutations that can drift. Each frame independently computes the correct position from (source_offset, edit_chain).

**Problem 3 (undo clears):** Undo produces a new buffer version with edits. Projection applies the undo's edits like any other. Decorations stay visible at their projected positions. No special case.

### Practical considerations

**Doesn't this need full rope snapshots?** No. The projection only needs the edit list, not the old rope. The `char_offset` is absolute (computed once when hints arrive), and the projection adjusts it through edits. No need to re-parse the old rope.

**What about the edit log from Phase 3?** This phase depends on having an edit log in the buffer -- a ring buffer of recent `Vec<Edit>` per version. Phase 3 introduces this for the document sync fix. If Phase 5 is done before Phase 3, we can add a minimal edit log here: just keep the last N edits in a `VecDeque`, keyed by version.

**Performance.** With 50 hints and 20 edits since the last refresh: 1000 offset comparisons. Sub-microsecond. The projection is cached per (source_version, current_version) pair, so between edits it's free.

### When to fall back to full refresh

If the source version has fallen off the edit log (the user made more edits than the log holds since the hints were fetched), we can't project. In this case, discard the decorations and trigger a refresh. This is strictly better than the current behavior (showing wrong positions).

The edit log should be sized for the expected latency: 500ms debounce + 200ms round-trip ≈ 700ms. At fast typing (10 chars/second with undo/redo), that's ~7-10 versions. A 64-entry ring buffer is more than enough.

## Migration Path

### Step 1: Add edit log to Buffer

```rust
struct EditLog {
    entries: VecDeque<(u64, Vec<Edit>)>,  // (version, edits)
    capacity: usize,
}

impl EditLog {
    fn push(&mut self, version: u64, edits: Vec<Edit>) { ... }
    fn edits_since(&self, version: u64) -> Option<Vec<&[Edit]>> { ... }
}
```

Populated from `record()` in `buffer/mod.rs`. This is useful for Phase 3 too.

### Step 2: Add `source_version` to Decoration

Add the field. Set it when creating decorations from LSP results. The current `adjust_for_edits()` still runs in parallel -- both systems coexist.

### Step 3: Implement `project()` and validate

Add the projection function. In debug builds, assert that `project()` agrees with `adjust_for_edits()` for a few frames. This validates the projection logic.

### Step 4: Switch renderer to projection

Replace `for_line()` to use projection. Remove `adjust_for_edits()` calls. Remove `decorations.clear()` from undo.

### Step 5: Handle the stale-application path

In `lsp_integration.rs:763-780`, when creating decorations from LSP results, use the *request*'s buffer version as `source_version`, not the current buffer version. The projection handles the rest.

## Files Changed

| File | Change |
|------|--------|
| `ovim-core/src/buffer/mod.rs` | Add `EditLog`, populate from `record()` |
| `ovim-core/src/editor/decoration.rs` | Add `source_version`, `project()`, caching |
| `ovim-core/src/editor/change_tracking.rs` | Remove `adjust_for_edits()` calls, remove `decorations.clear()` from undo |
| `ovim-core/src/editor/lsp_integration.rs` | Set `source_version` from request metadata when creating hint decorations |
| `ovim/src/ui/renderer/buffer.rs` | Update rendering to use projected positions |

## Verification

1. **Drift test:** Place a hint at column 10. Type 5 characters at column 0. Hint renders at column 15. Type 5 more. Hint at 20. No drift on any frame.
2. **Undo test:** Hints visible. Undo a change. Hints stay visible at correct positions (no flash).
3. **Stale hint test:** Request hints. Type rapidly. When stale hints arrive, they render at correct (projected) positions.
4. **Delete test:** Hint inside a region. Delete the region. Hint disappears. Undo. Hint reappears.
5. **Edit log overflow test:** Type 100 characters rapidly (more than log capacity since last hint refresh). Hints disappear cleanly (no wrong positions). Refresh triggers.
