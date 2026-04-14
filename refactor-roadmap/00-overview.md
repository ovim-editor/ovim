# LSP Subsystem Refactor Roadmap

## The Problems

Four user-facing symptoms, three confirmed root causes, plus structural debt:

1. **LSP stops responding after initial requests.** Race condition in `server.rs`: request sent before registered in pending map. Fast LSP servers respond before registration, response is silently dropped, caller waits 10s for timeout. Compounded by a global gate (`has_pending_lsp_response()`) that blocks all new user-triggered actions while any response slot is occupied.

2. **Save freezes the editor.** Synchronous `block_in_place` for file I/O + synchronous `git2` operations (diff, blame) on the event loop thread. Blocks all input, rendering, and LSP communication for 200ms-3s.

3. **Undo sends wrong content to LSP.** The didChange debouncer at `notifications.rs:300` only sets `old_text` on the first change after a sync (`if debouncer.old_text.is_none()`). After undo, the debouncer still holds `old_text` from the pre-undo edit, not from the undone state. When the incremental diff is computed, it diffs against the wrong baseline. The LSP server receives a corrupted edit and its view of the document diverges from the editor's.

4. **Inlay hints drift left as you type.** Hints arrive from LSP computed against a stale buffer version. They're applied with positions calculated against the current rope (wrong), then `adjust_for_edits()` shifts them further on each keystroke. Error accumulates linearly until the next refresh.

## Root Cause Analysis

### The Debouncer Content Bug (Symptoms 3, partially 1)

The `ChangeDebouncer` in `LspManager` holds three things: `pending_text` (new content), `old_text` (baseline for incremental diff), and `pending_version`. On each `did_change()` call:

- `pending_text` and `pending_version` are always updated
- `old_text` is only set when `None` -- first change after a flush

This means: edit A sets `old_text` to the pre-A content. Undo (which is edit B) updates `pending_text` to post-undo content but leaves `old_text` pointing to the pre-A content. The diff sent to the LSP is `diff(pre-A, post-undo)` -- which might produce the right full replacement, but when the server supports incremental sync, `compute_simple_diff()` produces a minimal patch between the wrong baseline and the current state. This can produce an edit that, when applied to what the server actually has, yields wrong content.

The existing tests (`lsp_document_sync_undo_test.rs`) only verify that `buffer_modified` is set after undo -- they don't test what content actually reaches the LSP.

### The Request-Response Race (Symptom 1)

`server.rs:1183-1202`: message is sent via `outgoing_tx` before the request is inserted into `pending_requests`. The reader task at line 557 can receive and process the response before `insert` executes, silently dropping it. The comment at line 1190 claims lock ordering prevents this, but the reader acquires its own independent lock.

### The Action Gate Design (Symptom 1, interaction effects)

`pending_lsp_action` is a single `Option<LspAction>` -- one slot for all user-initiated actions. It's gated by `has_pending_lsp_response()` which checks four response slots. This creates two problems:

- A stuck/slow response blocks all new actions (hover blocks goto-definition)
- Rapid user input silently overwrites queued actions with no feedback

Meanwhile, some `_impl()` methods (format, code actions, rename, references) await the LSP response inline, blocking `process_pending_lsp_actions()` for the full round-trip (100-500ms). Others (goto, hover) spawn a background task and return immediately. This inconsistency means the system's responsiveness depends on which action the user happens to trigger.

Completion works correctly because it has its own separate slot with sequence-based stale rejection -- the right design, but not applied to other actions.

### The Save Blocking (Symptom 2)

`save_as()` uses `block_in_place(block_on(save_as_async()))` on the event loop thread. Then `refresh_git_status()` and optionally `load_git_blame()` run synchronous `git2` operations. The entire command dispatch path is `fn execute_command() -> CommandResult` -- synchronous, no way to express "this takes time."

## Design Principles

1. **Make invalid states unrepresentable.** A request that has been sent but isn't tracked shouldn't be expressible. The debouncer shouldn't hold stale baselines.

2. **One owner per piece of state.** Document content truth lives in the Buffer. The debouncer doesn't independently track "what the server has" -- it receives the content to send and the baseline to diff against, both from the same source.

3. **Acknowledge that some things take time.** Save, formatting, git operations are not instant. The type system distinguishes sync from async results. The UI stays responsive.

4. **One pattern for all LSP features.** The codebase already has the right pattern (fire into a slot, poll for result, cancel on replace) — it's just implemented five different ways. `Slot<T>` unifies goto, hover, completion, inlay hints, diagnostics, format, rename, and every future LSP feature into one generic abstraction.

## Phases

| Phase | Scope | Fixes | Risk |
|-------|-------|-------|------|
| Phase | Scope | Fixes | Risk |
|-------|-------|-------|------|
| [**00: Quick Fixes**](./00-phase0-quick-fixes.md) | 5 surgical changes | All four symptoms | **Done** |
| [01: Request Pipeline](./01-request-pipeline.md) | `server.rs` | Response race, silent drops | **Done** (shipped in Phase 0) |
| [**02: Unified Slot Architecture**](./02-action-dispatch.md) | `Slot<T>`, LspSlots, LspIntents | Action blocking, silent overwrite, inline awaits, ad-hoc polling | Medium -- the next structural step |
| [03: Document Sync](./03-document-sync.md) | Debouncer, `DocumentSyncState`, content flow | Reconciliation complexity | Medium -- simplify after Phase 0 fix |
| [04: Async Save](./04-async-save.md) | `file_io.rs`, commands, git | Save freezes | **Done** (git ops shipped in Phase 0) |
| [05: Decoration Projection](./05-decoration-projection.md) | `decoration.rs`, inlay hints | Hint drift, undo clearing hints | Medium -- changes rendering pipeline |
| [06: LspState Decomposition](./06-lsp-state-decomposition.md) | `lsp_state.rs` | Maintainability | Low -- largely subsumed by Phase 2 |

**Phase 0 is shipped.** The acute symptoms are fixed. Phases 1 and 4 are done (the core fixes were in Phase 0).

**Phase 2 is the next big move.** The `Slot<T>` architecture replaces 5 ad-hoc implementations of the same pattern, converts all inline-await actions to non-blocking, and eliminates the single-slot bottleneck. It's designed for incremental migration — each feature conversion is independent and shippable. Phase 6 (LspState decomposition) is largely achieved as a side effect of Phase 2, since the ad-hoc pending structs are replaced by `LspSlots`.

Phase ordering:
- Phase 2 is next -- highest impact structural improvement, incremental migration
- Phase 3 is independent -- simplifies DocumentSyncState now that Phase 0 fixed the baseline bug
- Phase 5 depends on an edit log (can be added standalone or as part of Phase 3)
- Phase 6 may be unnecessary after Phase 2 cleans up LspState

## Files Involved

```
ovim-core/src/lsp/server.rs              -- request/response lifecycle (Phase 1)
ovim-core/src/lsp/mod.rs                 -- LspManager, ChangeDebouncer (Phases 1, 3)
ovim-core/src/lsp/notifications.rs       -- didChange, debouncing, old_text bug (Phase 3)
ovim-core/src/lsp/utils.rs              -- compute_simple_diff (Phase 3)
ovim-core/src/editor/lsp_state.rs        -- LspState, DocumentSyncState, PendingLspResponses (Phases 2, 3, 6)
ovim-core/src/editor/lsp_integration.rs  -- sync, reconciliation, actions, ensure_synced (Phases 2, 3)
ovim-core/src/editor/lsp_modules/*.rs    -- individual _impl methods (Phase 2)
ovim-core/src/editor/decoration.rs       -- DecorationMap, adjust_for_edits (Phase 5)
ovim-core/src/editor/change_tracking.rs  -- undo/redo, edit recording (Phases 3, 5)
ovim-core/src/buffer/file_io.rs          -- save_as, block_in_place (Phase 4)
ovim-core/src/commands.rs                -- command dispatch (Phase 4)
ovim/src/event_loop.rs                   -- main loop, tick, gating logic (Phases 2, 4)
```
