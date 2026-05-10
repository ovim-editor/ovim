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
| [15](./15-change-enum-simplification.md) | Simplify the `Change` enum (all steps) | `23e6eeb` / `30142fb` / `cc813ea` / `beb850d` / `1e5f9b2` / `cb85572` / `31996ca` / `a005cf7` / step-4.3 |
| [16](./16-event-loop-grouping.md) | Event loop phase grouping | `443ffb4` |
| [18](./18-line-text-accessor.md) | Unified `Buffer::line_text` accessor | 2026-04-28 |
| [19](./19-per-window-wrap-map.md) | Per-window wrap map (OV-00209) | `2c38903` / `70c7f9c` / `2db5684` (+ 19.5 tests) |

These docs are kept with `(DONE)` banners so the "what was deleted and why"
trail stays discoverable. They should not drive new work.

## Active roadmap

| # | Title | Type | Risk | Effort |
|---|-------|------|------|--------|
| [17](./17-multi-server-sync.md) | Multi-server document sync | Bug prevention | **Medium** | Medium |

### Recommended order

**17** is the only remaining active item. It has user-facing impact —
prioritize it if you're expanding companion server support (Tailwind CSS
+ TypeScript).

Roadmap **18** (unified `Buffer::line_text` accessor) shipped 2026-04-28
including the cosmetic sweep. 186 dead `trim_end_matches('\n')` calls
were removed; the 10 that remain are all genuinely live (raw `Rope::line()`
results in renderer / motion / event_loop, AI extraction, `&mut String`
patterns) and pinned by a regression guard in
`buffer::line_ending::tests::trim_end_matches_n_count_is_bounded`. The
line-ending bug class is structurally closed.

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

**Multi-server document versions are shared, not per-server** — If one
server in a multi-server setup (TypeScript + Tailwind CSS) silently drops
a `didChange` or restarts, the editor can't detect divergence. Roadmap 17
addresses this with periodic re-sync.

**`scroll_offset` lives in two places** — `Window::scroll_offset` and
`ViewportState::scroll_offset`, resynced on focus change. (The wrap map used to
have the same problem; roadmap 19 moved it onto `Window`, with
`ViewportState::wrap_map` kept only as the headless fallback. `scroll_offset`
hasn't had the same treatment.) Works, but it's the "focused window's state is
the editor's state" pattern showing its seams. Unifying it is the
`editor/mod.rs` decomposition (ex-roadmap 10) — background work, no dedicated
roadmap.
