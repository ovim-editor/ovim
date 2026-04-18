# Refactor Roadmap

## History

Phases 0–8 addressed user-facing bugs: LSP response races, save freezes, undo/LSP content divergence, inlay hint drift, column coordinate mismatches. The LSP subsystem was rebuilt around `Slot<T>` / `TrackedSlot<T>` with intent-based dispatch. All of that is done and working.

This roadmap covers what remains: structural cleanup that reduces cognitive load and prepares the codebase for safe extension.

## Completed (reference only)

| Phase | Scope | Files |
|-------|-------|-------|
| 00–01 | Quick fixes + request pipeline | `server.rs` |
| 02 | `Slot<T>`, `LspSlots`, `LspIntents` | `lsp_slot.rs`, `lsp_subsystem.rs`, `lsp_state.rs` |
| 03 | Document sync / debouncer fix | `notifications.rs`, `lsp/mod.rs` |
| 04 | Async save + git ops | `file_io.rs`, `commands.rs` |
| 05 | Decoration projection | `decoration.rs`, `change_tracking.rs` |
| 06 | LspState decomposition (superseded by 02) | — |
| 07 | Completion textEdit | completion code |
| 08 | Column coordinate correctness | buffer ops, LSP conversions |

These docs (00-phase0 through 08) are kept for historical reference. They describe solved problems and should not drive new work.

## Recently shipped

| # | Title | Landed in |
|---|-------|-----------|
| [13](./13-dead-change-variants.md) | Remove dead `Change` variants | `0a8af89` |
| [14](./14-text-object-resolution.md) | Unify `TextObjectType` resolution | `d6a114a` |
| [15](./15-change-enum-simplification.md) step 1 — document the boundary | `23e6eeb` / `30142fb` |
| [16](./16-event-loop-grouping.md) | Event loop phase grouping | `443ffb4` |

These docs are kept with `(DONE)` banners so the "what was deleted and why"
trail stays discoverable. They should not drive new work.

## Active roadmap

| # | Title | Type | Risk | Effort |
|---|-------|------|------|--------|
| [15](./15-change-enum-simplification.md) step 4.3 | Remove `InsertText` / `DeleteText` cursor-positioning wrappers | Architecture | Low | Medium |
| [17](./17-multi-server-sync.md) | Multi-server document sync | Bug prevention | **Medium** | Medium |

### Recommended order

**15 step 4.3** is the only undo-system cleanup still open. After steps
3 + 4.1 + 4.2 (insert-mode recording, `RepeatAction::InsertSession`,
`Composite` removal), `InsertText` / `DeleteText` survive only as
transient cursor-positioning wrappers used by `apply_change_and_record`.
Replacing them is mechanical (~15 call sites) but touches a wide
surface — own sprint.

**17** is the only roadmap item with user-facing impact. Prioritize it if
you're expanding companion server support (e.g., Tailwind CSS +
TypeScript).

## What was retired

Old roadmaps 09–12 are replaced by the active roadmap above:

- **09 (undo unification)** → Split into 13 (dead code) + 15 (simplification). The "don't unify yet" advice was correct — but the dead code should still go.
- **10 (editor decomposition)** → Remains background work. `LspSubsystem` is the template; follow the same pattern when touching other areas. No dedicated roadmap needed — the pattern is established.
- **11 (event loop ordering)** → Replaced by 16 with concrete phase inventory and grouping proposal.
- **12 (multi-server sync)** → Carried forward as 17 with the same recommendation (Option B: periodic re-sync).

## Architecture notes

### What's singing

**`Slot<T>` / `TrackedSlot<T>`** — Cancellation is structural (replacing the in-flight request *is* cancelling it). `TrackedSlot`'s generation counter can't lose an invalidation, can't consume it twice, and debounce composes orthogonally. This is the reference abstraction for the codebase.

**`DecorationMap` with versioned projection** — Each decoration stores its `source_version` and a char offset in that version's rope. At render time, `project_offset` replays the edits from `source_version` forward to get the current offset. No accumulated drift, no wrong-baseline errors on undo, and stale decorations from old buffer versions project onto current positions instead of rendering where they were first placed.

**`LspSubsystem` grouping** — State, slots, intents, channels, UI — all one field access away, with a clear boundary.

**`Edit` enum** — Absolute char offsets, mechanically reversible, no ambiguity. The clean core of the undo system.

### Where the tension lives

**`InsertText` / `DeleteText` survive as cursor-positioning wrappers** —
After roadmap 15 steps 3 + 4.1 + 4.2, undo and dot-repeat for insert
sessions go through `Recorded` + `RepeatAction::InsertSession`.
`InsertText` / `DeleteText` no longer reach the undo stack from
sessions, but `apply_change_and_record` still wraps each insert/delete
keystroke in one of those variants for the cursor-positioning side
effect of `change.apply()`. Roadmap 15 step 4.3 inlines that into
buffer-level helpers and lets the variants go.

**Multi-server document versions are shared, not per-server** — If one
server in a multi-server setup (TypeScript + Tailwind CSS) silently drops
a `didChange` or restarts, the editor can't detect divergence. Roadmap 17
addresses this with periodic re-sync.
