# 15: Simplify the `Change` Enum

> **DONE.** After step 4.3 `Change` is just `Recorded` + `ResourceOp`.
> Every buffer-level edit call site now goes through a buffer helper
> (`insert_text_at_positioning_cursor` /
> `delete_range_positioning_cursor`), and `Editor::record_edit` provides
> the closure-based shim that used to live in `apply_change_and_record`.
> See the step-4.3 landing note below for the semantics-preserving
> tricks (`RepeatAction::InsertSession` for non-session direct-path
> keystrokes, `RepeatAction::PasteAfter` for Normal-mode bracketed
> paste).

**Goal:** After removing dead variants (roadmap 13), clarify what remains. The `Change` enum should read as a clean description of what it actually does — undo records for two distinct editing patterns.

**Fixes:** Architectural clarity. Makes the undo/repeat boundary legible to new readers.

**Risk:** Low if done incrementally. Medium if attempted all at once.

## Where `Change` is now (post step 4.3)

| Variant | Role | Used by |
|---------|------|---------|
| `Recorded` | Mechanical undo + redo backed by a `Vec<Edit>` | Insert/replace-mode sessions, Normal-mode operators, LSP edits, API handlers, direct-path buffer helpers |
| `ResourceOp` | Filesystem snapshot pair | LSP workspace create/rename/delete |

`InsertText`, `DeleteText`, and `Composite` are all gone. Dot-repeat for
insert sessions runs through `RepeatAction::InsertSession` (offsets
re-anchored by `delta = new_origin - origin_offset`). Direct-path
keystrokes (visual-`c` entering insert, Normal-mode bracketed paste)
also set a `RepeatAction` so `.` still re-anchors to the current cursor
the way `Change::InsertText::repeat` used to.

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

**4.3 — DONE.** Removed the `InsertText` / `DeleteText` variants, their
constructors (`Change::insert`, `Change::delete`,
`Change::delete_backward`), and their arms in `apply`, `undo`, `repeat`,
`cursor_before`, `cursor_after`, `set_cursor_before`, `set_cursor_after`,
`get_inserted_text`, `into_edits`. `Change::calculate_end_position`
also went away (the `helpers::calculate_end_position` copy lives on for
its other callers). `apply_change_and_record` was replaced by two
primitives:

1. `Buffer::insert_text_at_positioning_cursor` and
   `Buffer::delete_range_positioning_cursor` port the cursor-landing
   behavior (end of inserted text / start of deleted range) that the
   old `Change::InsertText/DeleteText::apply` provided.
2. `Editor::record_edit(cursor_before, |buf| …)` wraps those helpers
   in the recording-origin / undo-push logic `apply_change_and_record`
   used to carry. During an active insert session the ambient recording
   captures edits and the session-level `finalize_change_building`
   pushes a single `Recorded`. Outside a session it wraps the edit in
   `buffer.record()`, pushes a `Recorded`, AND sets
   `RepeatAction::InsertSession { entry_mode: Insert, … }` so `.`
   re-anchors to the current cursor — the behavior the old
   `Change::InsertText::repeat` baked into the undo variant itself.

The Normal-mode bracketed paste did NOT route through `record_edit`; it
needs `RepeatAction::PasteAfter { count: 1 }` to preserve the "paste-at-
cursor" semantics of `.`. The Insert-mode bracketed paste goes through
`record_edit` so the ambient insert session captures it.

The API edit handlers in `ovim/src/event_loop.rs` (`handle_insert_lines`,
`handle_delete_lines`) were converted from direct mutation + `Change::insert`
/ `Change::delete` to `buffer.record()` + `push_recorded_undo`, matching
the `handle_edit_line` shape that already existed.

Test surface touched: the `insert_mode.rs` hygiene test that previously
asserted `Some(Change::InsertText { .. })` now asserts
`Some(Change::Recorded { .. })` — the direct-path push genuinely
changed type, so the assertion update reflects real semantics rather
than weakening the test.

## Files

- `ovim-core/src/change.rs` — primary target (much smaller after 4.2)
- `ovim-core/src/edit.rs` — `UndoEntry` may absorb `Change` once 4.3 lands
- `ovim-core/src/editor/change_tracking.rs` — `repeat_last_change()`, `push_recorded_undo()`
- `ovim-core/src/editor/input/insert_mode.rs` — insert mode finalization
- `ovim-core/src/editor/input/helpers.rs` — insert mode keystroke handling
- `ovim-core/src/repeat_action.rs` — `InsertSession` lives here
