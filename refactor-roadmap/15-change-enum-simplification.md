# 15: Simplify the `Change` Enum

**Goal:** After removing dead variants (roadmap 13), clarify what remains. The `Change` enum should read as a clean description of what it actually does — undo records for two distinct editing patterns.

**Fixes:** Architectural clarity. Makes the undo/repeat boundary legible to new readers.

**Risk:** Low if done incrementally. Medium if attempted all at once.

## What remains after roadmap 13

After removing the four dead variants, `Change` has:

| Variant | Undo | Repeat | Used by |
|---------|------|--------|---------|
| `InsertText` | Line/col delete | Re-insert at cursor | Insert mode keystrokes |
| `DeleteText` | Re-insert at range start | Re-delete at cursor | Insert mode backspace/delete |
| `Composite` | Undo children in reverse | Replay children + entry_mode cursor positioning | Insert mode sessions (i/a/I/A), API operations |
| `Recorded` | Inverse edits in reverse | Replay edits forward | All Pattern B operations, LSP edits |
| `ResourceOp` | Restore file snapshots | Non-repeatable | LSP workspace resource operations |

Two patterns are visible:

**Pattern A** (`InsertText`, `DeleteText`, `Composite`): Line/col coordinates, semantic undo/repeat intertwined in the same type. Used exclusively for insert-mode keystroke batching, where the ChangeBuilder accumulates individual keystrokes and `Composite` groups them.

**Pattern B** (`Recorded`): Absolute char offsets via `Edit`, mechanical undo, repeat handled externally by `RepeatAction`. Used for everything else.

## The quiet asymmetry

`InsertText.apply()` uses `buffer.insert_text_at(line, col, text)` — line/col coordinates.
`InsertText.undo()` converts to absolute char offsets and uses `buffer.delete_char_range()`.

The comment in `undo()` explains why: line/col clamping via `line_len()` excludes newlines, but insertions can target the newline position. So `apply` and `undo` use different coordinate systems for the same operation. `Edit`/`Recorded` doesn't have this problem — both directions use absolute offsets.

`Composite.repeat()` takes `&mut self` and mutates the change in place (updating range, deleted_text, position). Then `repeat_last_change()` in `change_tracking.rs` clones the change, calls `repeat()`, and writes it back. The clone-mutate-write-back dance exists because repeat and undo are coupled in the same type. Compare with `RepeatAction::execute(&self, buffer)` — immutable, no side effects.

## The opportunity

`InsertText` and `DeleteText` are only created inside insert mode (via `add_change()` through `ChangeBuilder`). They represent individual keystrokes that get batched into a `Composite`. The `Composite` is what matters for undo and repeat.

If insert mode used `buffer.record()` to capture its keystrokes as `Edit`s, and stored the batch as `Change::Recorded`, then `InsertText` and `DeleteText` would also become dead code. The `Composite` would become a `Recorded` with `entry_mode` metadata for repeat cursor positioning.

This would leave `Change` with just `Recorded` and `ResourceOp` — at which point it's essentially `UndoEntry` from `edit.rs` plus filesystem snapshots. The dual representation problem from old roadmap 09 dissolves naturally.

## Incremental path

Don't do this all at once. The steps:

### Step 1: Document the boundary (immediate)

Add a module-level comment to `change.rs` that explains the two patterns and their mutual exclusion. The existing comment at the top is good but could be sharper about *why* both exist (insert-mode keystroke batching is the last holdout).

### Step 2: Audit `Composite.repeat()` mutation (investigation)

Map exactly what `repeat()` mutates on `InsertText` and `DeleteText` inside a `Composite`. The key question: can the same repeat behavior be achieved by recording the composite's buffer mutations as `Edit`s and replaying them? If yes, `Composite` can become `Recorded` + `entry_mode`.

The tricky part: insert-mode repeat isn't just "replay the same edits" — it positions the cursor based on `entry_mode` (I goes to first non-blank, A goes to end of line, etc.) and then replays the keystrokes. This cursor positioning is semantic, not mechanical. So the repeat side needs `entry_mode`, even if undo becomes mechanical.

### Step 3: Extract `entry_mode` into repeat metadata (if step 2 confirms feasibility)

Add an `InsertSession` variant to `RepeatAction` that carries `entry_mode` + the inserted text (or keystroke sequence). Insert mode finalization would then:
- Push a `Change::Recorded` (mechanical undo from `buffer.record()`)
- Set `RepeatAction::InsertSession { entry_mode, keystrokes }` (semantic repeat)

This fully separates undo from repeat for insert mode, matching how all other operations already work.

### Step 4: Remove `InsertText`, `DeleteText`, `Composite` (after step 3)

At this point, `Change` has only `Recorded` and `ResourceOp`. Consider renaming to `UndoRecord` or merging with `UndoEntry`.

## What NOT to do

- Don't remove `calculate_end_position()` until `InsertText` and `Composite` are gone — it's used by their `apply()`/`undo()`/`repeat()` methods.

## Background: what Pattern A does today

Collected from the module-doc investigation that previously lived at the top of `change.rs`. This is "here's what a future implementer needs to know," not specification.

### How ChangeBuilder works today

`ChangeBuilder` accumulates individual `Change::InsertText` / `DeleteText` entries via `add()`. On `build()`, if there's exactly one change and `entry_mode` is plain `Insert`, it unwraps the single change (avoiding a Composite wrapper). Otherwise it wraps in `Composite` with `entry_mode` and `cursor_before` / `cursor_after`. The builder is started when entering insert mode (`start_change_building`) and finalized on exit (`finalize_change_building`).

**There is NO per-keystroke undo during an active insert session.** The builder accumulates changes, but they're not on the undo stack until `finalize_building_at()` is called on Esc. Backspace during insert mode is handled by `delete_char_before_cursor()` adding a `DeleteText` to the builder — not by popping from undo. So the builder's per-change granularity is only used for replay ordering, not for mid-session undo.

### How Composite.repeat() works

`repeat(&mut self)` first repositions the cursor based on `entry_mode` (I→first non-blank, A→end of line, a→right by 1, etc.), then iterates `changes.iter_mut()` calling `repeat()` on each sub-change. Each `InsertText.repeat()` mutates its own `position` field to the current cursor, then applies. Each `DeleteText.repeat()` recalculates the deletion range from the current cursor and mutates `range` and `deleted_text` to match what was actually deleted. This mutation is critical: the repeated `Composite` becomes a valid undo entry because its sub-changes now reflect actual positions. Finally, cursor moves left by 1 to simulate Esc.

### How insert mode creates changes

In `helpers.rs`, `insert_char()`, `insert_newline()`, `delete_char_before_cursor()`, etc. each create a `Change::InsertText` or `Change::DeleteText` and call `editor.apply_change_and_record()`. When a builder is active (insert-mode session), `add_change()` routes to `builder.add()` instead of pushing directly to the undo stack.

### The entry_mode cursor positioning

`Composite.repeat()` handles cursor repositioning before replay. A `RepeatAction::InsertSession` could do the same — it just needs the `InsertEntryMode` enum value and would reposition before replaying keystrokes. This is straightforward.

### Migration path

1. Wrap the insert session in `buffer.record()` instead of using `ChangeBuilder`. Each `insert_char` / `insert_newline` / `delete_char` call would go through `buffer.insert_text_at()` / `buffer.delete_range()` directly (they already do — `Change.apply()` calls these). The `record()` closure would capture all `Edit`s.

2. On exit, push `Change::Recorded { edits, ... }` for undo.

3. For repeat, store `RepeatAction::InsertSession { entry_mode, keystrokes: Vec<KeyEvent> }`. Repeat would: reposition cursor per entry_mode, then replay each keystroke through the insert-mode handler (which re-derives indentation, completion, etc.).

### Tricky parts

- **Keystroke replay vs. edit replay**: The current `Composite.repeat()` replays *edits* (insert "x" at position, delete range, etc.), not keystrokes. This is simpler but loses context (auto-indent on Enter bakes in the indent string). A `RepeatAction` replaying keystrokes would be more correct (re-derive indent for the new context) but requires capturing the raw `KeyEvent` sequence.

- **buffer.record() scoping**: Currently `record()` takes a closure. An insert session spans many event-loop ticks. We'd need `buffer.start_recording()` / `buffer.stop_recording()` (a stateful recording mode) rather than the current closure-based API.

- **Completion and snippets**: `accept_completion()` does multi-step edits (delete prefix, insert completion text). These currently produce `InsertText` / `DeleteText` changes. Under Pattern B they'd just be recorded edits, which is fine for undo but means the keystroke log needs a "completion accepted" marker for faithful replay.

- **Visual block insert replay**: `exit_insert_mode()` replays the first line's changes on subsequent lines. This currently clones `Change` objects. Under Pattern B, it could replay the same keystrokes or the same edits (offset-adjusted) on each line.

- **Whitespace cleanup**: `cleanup_whitespace_only_line()` adds a `DeleteText` to the builder before finalize. Under Pattern B this would just be another recorded edit — simpler.

**Bottom line**: The migration is feasible and would unify the undo model. The main prerequisite is a stateful recording API on Buffer (start/stop instead of closure). The repeat side needs a keystroke capture mechanism. Neither is architecturally risky, but it touches insert mode, undo, repeat, completion, and visual block — so it should be its own focused sprint, not a drive-by refactor.

## Files

- `ovim-core/src/change.rs` — primary target
- `ovim-core/src/edit.rs` — `UndoEntry` may absorb `Change` in step 4
- `ovim-core/src/editor/change_tracking.rs` — `repeat_last_change()`, `push_recorded_undo()`
- `ovim-core/src/editor/input/insert_mode.rs` — insert mode finalization
- `ovim-core/src/editor/input/helpers.rs` — insert mode keystroke handling
- `ovim-core/src/repeat_action.rs` — potential `InsertSession` variant
