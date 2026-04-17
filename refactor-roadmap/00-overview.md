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
| 05 | Decoration projection (partial) | `decoration.rs`, `change_tracking.rs` |
| 06 | LspState decomposition (superseded by 02) | — |
| 07 | Completion textEdit | completion code |
| 08 | Column coordinate correctness | buffer ops, LSP conversions |

These docs (00-phase0 through 08) are kept for historical reference. They describe solved problems and should not drive new work.

## Active roadmap

| # | Title | Type | Risk | Effort |
|---|-------|------|------|--------|
| [13](./13-dead-change-variants.md) | Remove dead `Change` variants | Dead code removal | **None** | Small |
| [14](./14-text-object-resolution.md) | Unify `TextObjectType` resolution | Deduplication | **Low** | Small |
| [15](./15-change-enum-simplification.md) | Simplify the `Change` enum | Architecture | **Low** | Medium |
| [16](./16-event-loop-grouping.md) | Event loop phase grouping | Readability | **None** | Small |
| [17](./17-multi-server-sync.md) | Multi-server document sync | Bug prevention | **Medium** | Medium |

### Recommended order

**13 → 14 → 15** form a natural sequence: remove dead code, extract the shared dispatch, then simplify the enum that's left. Each step is independently shippable and makes the next one cleaner.

**16** is independent — do it whenever you want a quick win.

**17** is the only one with user-facing impact. Prioritize it if you're expanding companion server support (e.g., Tailwind CSS + TypeScript).

## What was retired

Old roadmaps 09–12 are replaced by the active roadmap above:

- **09 (undo unification)** → Split into 13 (dead code) + 15 (simplification). The "don't unify yet" advice was correct — but the dead code should still go.
- **10 (editor decomposition)** → Remains background work. `LspSubsystem` is the template; follow the same pattern when touching other areas. No dedicated roadmap needed — the pattern is established.
- **11 (event loop ordering)** → Replaced by 16 with concrete phase inventory and grouping proposal.
- **12 (multi-server sync)** → Carried forward as 17 with the same recommendation (Option B: periodic re-sync).

## Architecture notes

### What's singing

**`Slot<T>` / `TrackedSlot<T>`** — Cancellation is structural (replacing the in-flight request *is* cancelling it). `TrackedSlot`'s generation counter can't lose an invalidation, can't consume it twice, and debounce composes orthogonally. This is the reference abstraction for the codebase.

**`DecorationMap` with char-offset anchoring** — Mutations use flat offsets (pure arithmetic in `adjust_for_edits`), queries use lines (derived from rope at call time). Two-level structure, right boundary between them.

**`LspSubsystem` grouping** — State, slots, intents, channels, UI — all one field access away, with a clear boundary.

**`Edit` enum** — Absolute char offsets, mechanically reversible, no ambiguity. The clean core of the undo system.

### Where the tension lives

**`Change` does three jobs** — undo record, repeat template, and semantic description. Pattern B (`Edit`-based undo + `RepeatAction` for semantic repeat) has already won for most operations, but the `Change` enum still carries the weight of its former roles.

**`TextObjectType` resolution** — Same 8-arm match block duplicated in three files. Adding a new text object type requires touching all three.

**Event loop readability** — Phases are more independent than they look (each `_impl()` syncs its own document state), but a reader can't know that without deep knowledge. Named groups would make the rhythm visible.
