# 15: Simplify the `Change` Enum

> **Steps 1–3 + 4.1–4.2 DONE.** Insert sessions now push `Change::Recorded`
> from session-captured edits, with `RepeatAction::InsertSession` carrying
> the dot-repeat payload. `Change::Composite` and the `ChangeBuilder`
> accumulator are gone; the builder shrank to `cursor_before` + `entry_mode`.
> What's left (step 4.3) is removing the transient `InsertText` /
> `DeleteText` wrappers used by `apply_change_and_record` for cursor
> positioning. Tracked separately below.

**Goal:** After removing dead variants (roadmap 13), clarify what remains. The `Change` enum should read as a clean description of what it actually does — undo records for two distinct editing patterns.

**Fixes:** Architectural clarity. Makes the undo/repeat boundary legible to new readers.

**Risk:** Low if done incrementally. Medium if attempted all at once.

## Where `Change` is now (post step 4.2)

| Variant | Role | Used by |
|---------|------|---------|
| `InsertText` | Transient cursor-positioning wrapper inside `apply_change_and_record` | Insert/replace-mode helpers (see step 4.3) |
| `DeleteText` | Transient cursor-positioning wrapper inside `apply_change_and_record` | Insert/replace-mode helpers (see step 4.3) |
| `Recorded` | Mechanical undo + redo backed by a `Vec<Edit>` | All session and direct edits, LSP edits, API handlers |
| `ResourceOp` | Filesystem snapshot pair | LSP workspace create/rename/delete |

`Composite` is gone. Insert sessions and the API edit handler push
`Recorded` directly. Dot-repeat for insert sessions runs through
`RepeatAction::InsertSession` (offsets re-anchored by `delta = new_origin - origin_offset`).

## Step 1: Document the boundary (DONE — `23e6eeb` / `30142fb`)

The module-level comment at the top of `change.rs` originally named
both patterns and called out the apply/undo coordinate asymmetry. After
step 4.2 the comment was rewritten to describe the post-Composite
architecture; the historical pattern-A/B split is no longer load-bearing
in the source.

## Step 2: Audit `Composite.repeat()` mutation (DONE)

**Conclusion:** the in-place mutation that made step 2 look risky was
vestigial — load-bearing only before `repeat_last_change()` started
wrapping dot-repeat in `buffer.record()` and pushing a
`Change::Recorded` for mechanical undo. Once that was true, the mutated
Composite only lived as the re-repeat template in `last_change`, and
every `.repeat()` method read fresh cursor state rather than the
mutated fields.

The audit confirmed that:

1. `InsertText.repeat()`'s `*self_pos = new_pos` is written but never
   re-read — replay uses `buffer.cursor_char_col()` + `self.text`.
2. `DeleteText.repeat()`'s width is preserved across mutations (the new
   range has `end - start == old end - old start`), and replay reads
   only the width and the `backwards` flag.
3. `Composite.repeat()`'s `iter_mut()` only existed to thread (1) and
   (2); the only semantically load-bearing parts were the entry_mode
   reposition and the trailing `move_left(1)` — both ported cleanly to
   `RepeatAction::InsertSession`.

## Step 3: Replace `Composite` with `RepeatAction::InsertSession` (DONE)

Landed as `cc813ea` (buffer recording API), `beb850d`
(`RepeatAction::InsertSession` + execute), `1e5f9b2` (insert-mode
wiring), `cb65db8` (this doc's step-2 audit). At insert-mode finalize:

- Push `Change::Recorded` (mechanical undo from session edits).
- Set `RepeatAction::InsertSession { entry_mode, origin_offset, edits }`.

Payload chose **Option A** — a relative edit log carried as raw `Edit`s
plus the absolute `origin_offset` they were captured against. Replay
re-anchors via `delta = new_origin - origin_offset`. This mirrors the
old Composite's behavior and preserves the edge cases (backspace
across origin, `accept_completion` delete-then-insert, intra-session
arrow keys).

`accept_completion_item` uses `pause_recording` /
`resume_recording` around its own `record()` so its undo entry stays
isolated from the surrounding insert session — same semantics as
before step 3.

## Step 4: Remove `InsertText` / `DeleteText` / `Composite`

**4.1 — DONE (`cb85572`)** Insert sessions push `Change::Recorded`, not
`Composite`. Updated consumers: `open_line_repeat` reads
`InsertSession`'s entry_mode; `pending_change_repeat` merges delete +
insert as one Recorded; visual-block replay wraps siblings in
`record()` and combines into one Recorded. New `edit_start` field on
`Recorded` carries the post-entry-mode cursor for `g;` / changelist,
keeping `cursor_before` available for undo restore.

**4.2 — DONE (`31996ca` + `a005cf7`)** API edit handler refactored to
`record()` + `Recorded`. `Change::Composite` variant + constructor +
match arms removed. `ChangeBuilder` shrunk to `cursor_before` +
`entry_mode`. `ChangeManager::add_change` is a no-op while building;
`finalize_building_at` removed.

**4.3 — Open.** `Change::insert` / `Change::delete` are still
constructed by ~15 call sites (insert/replace-mode helpers) for the
side effect of `change.apply()`: rope mutation + automatic cursor
positioning. Replacing them requires inlining the cursor logic into
either each call site or a new buffer-level helper. Mechanical work
but touches a wide surface — own sprint.

When 4.3 lands, `Change` reduces to `Recorded` + `ResourceOp` and
`apply_change_and_record` collapses. At that point `calculate_end_position()`
also becomes dead.

## Files

- `ovim-core/src/change.rs` — primary target (much smaller after 4.2)
- `ovim-core/src/edit.rs` — `UndoEntry` may absorb `Change` once 4.3 lands
- `ovim-core/src/editor/change_tracking.rs` — `repeat_last_change()`, `push_recorded_undo()`
- `ovim-core/src/editor/input/insert_mode.rs` — insert mode finalization
- `ovim-core/src/editor/input/helpers.rs` — insert mode keystroke handling
- `ovim-core/src/repeat_action.rs` — `InsertSession` lives here
