# Phase 2: Action Dispatch

**Goal:** User-triggered LSP actions don't block each other. Each action type manages its own lifecycle. No silent overwrites.

**Fixes:** Hover blocking goto-definition. Actions lost during fast input. Format/code-actions blocking the event loop for 500ms.

**Risk:** Low-Medium. Changes how actions are queued and processed, but each action's `_impl()` method is unchanged internally.

## The Problems

### Problem 1: The single-slot bottleneck

`pending_lsp_action` is a single `Option<LspAction>`. All 16 action types share one slot. `queue_lsp_action()` does a silent overwrite:

```rust
fn queue_lsp_action(&mut self, action: LspAction) {
    self.lsp.state.pending_lsp_action = Some(action);  // overwrites previous
    self.lsp.state.lsp_action_retry_count = 0;
}
```

If the user triggers two actions before the first is processed, the first is silently lost.

### Problem 2: The global gate

```rust
// event_loop.rs:474 and :919
if !editor.has_pending_lsp_response() {
    editor.process_pending_lsp_actions().await;
}
```

`has_pending_lsp_response()` checks four response slots (hover, definition, implementation, type_definition). If any slot is occupied, no new actions can be dispatched. Hover in flight blocks goto-definition.

### Problem 3: Mixed sync and async _impl methods

Some `_impl()` methods spawn a background task and return immediately:
- `goto_definition_impl()` -- spawns task, stores in `pending_lsp_responses.definition`
- `hover_impl()` -- spawns task, stores in `pending_lsp_responses.hover`

Others await the full LSP round-trip inline:
- `format_document_impl()` -- awaits `format_document()` (100-500ms)
- `code_actions_impl()` -- awaits `code_actions()` (100-300ms)
- `find_references_impl()` -- awaits `find_references()` (50-200ms)
- `rename_impl()` -- awaits `rename()` (100-500ms)

The inline-await ones block `process_pending_lsp_actions()`. During that time, no other action can start, the event loop doesn't process input (within the select arm), and status updates don't render.

All of them also call `prepare_lsp_request()` which includes `ensure_lsp_document_synced()` + a hardcoded `sleep(10ms)` at line 1795. Every action pays 10ms+ of unnecessary latency.

### What completion gets right

Completion has its own `pending_completion` slot and a monotonic `completion_request_seq`. Multiple requests can be in flight; only the latest is used. This is the right pattern.

## The Design

### Observation: Actions fall into two categories

**Navigate-and-show** actions spawn a background task and show results in a popup/picker:
- Hover, goto-definition/implementation/type, find-references, document-symbols, workspace-symbols, call/type-hierarchy

**Mutate-and-confirm** actions change the buffer and need the result before continuing:
- Format, code-actions (apply), rename, organize-imports

The first category can safely overlap -- hover and goto can be in flight simultaneously. The second category is naturally sequential -- you don't format while renaming.

### Per-slot dispatch (remove the single slot)

Replace `pending_lsp_action: Option<LspAction>` with direct dispatch. When the user presses a key, the action dispatches immediately into its own slot:

```rust
// Before:
pub fn request_goto_definition(&mut self) {
    self.queue_lsp_action(LspAction::GoToDefinition);  // put in single slot
}

// After:
pub fn request_goto_definition(&mut self) {
    self.pending_goto_definition = true;  // flag for this specific action
}
```

Or simpler -- just process the action inline in the input handler and remove the queue entirely. The `_impl()` methods that spawn background tasks already return immediately:

```rust
// In the input handler (where the key is processed):
EditorEvent::Key('g', 'd') => {
    editor.goto_definition_impl().await;  // spawns task, returns Ok(false) immediately
}
```

This is already how it works for the input path at `event_loop.rs:920`:

```rust
// Immediately process LSP actions triggered by input
if !editor.has_pending_lsp_response() {  // ← remove this gate
    editor.process_pending_lsp_actions().await;
}
```

The fix: **remove the gate**. The `_impl()` methods that spawn tasks already cancel previous requests in the same slot (goto_definition_common at line 93 does `definition.take()` + `old.task.abort()`). The per-slot design handles concurrency correctly.

### Per-slot cancellation

Each slot already does cancellation -- goto at line 93:

```rust
if let Some((_, old)) = self.lsp.state.pending_lsp_responses.definition.take() {
    old.task.abort();
}
```

The user presses `gd`, then quickly `gd` again: the first request is cancelled, the second starts. This is correct behavior.

The pattern should be uniform: every action's `_impl()` method cancels any previous pending request in its slot before starting a new one.

### Make inline-await actions non-blocking

The `_impl()` methods that await inline need to become spawn-and-poll, following the goto/hover pattern:

```rust
// Before (format_document_impl):
pub async fn format_document_impl(&mut self) -> Result<bool> {
    let ctx = self.prepare_lsp_request("format").await?;  // 10ms+ sleep
    let result = ctx.lsp.format_document(...).await;       // 100-500ms block
    self.apply_lsp_edits(result);
    Ok(true)
}

// After:
pub async fn format_document_impl(&mut self) -> Result<bool> {
    self.ensure_lsp_document_synced().await;  // no gratuitous sleep
    
    let (tx, rx) = oneshot::channel();
    let lsp = self.lsp_manager().clone();
    let task = tokio::spawn(async move {
        let result = lsp.format_document(...).await;
        let _ = tx.send(result);
        Ok(None)
    });
    
    self.pending_format = Some(PendingLspRequest { task, receiver: rx, started: Instant::now() });
    self.set_lsp_status("Formatting...".to_string());
    Ok(false)  // result arrives via polling
}
```

The response is processed in a `poll_pending_format()` method called from the tick, same as hover and goto.

For actions that mutate the buffer (format, rename), the poll handler applies the edits when the result arrives. The user sees "Formatting..." in the status line and can keep navigating while it runs.

### Remove the hardcoded sleep

`prepare_lsp_request()` at line 1795 has:

```rust
let did_flush = self.ensure_lsp_document_synced().await;
if did_flush {
    tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;
}
tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;  // why?
```

The 10ms sleep exists to "give the server time to process the didChange." This is cargo-cult -- the LSP protocol is request/response; the server will either respond with results based on the new content or with a ContentModified error. The sleep adds latency to every single LSP action.

Remove both sleeps. If a server returns stale results, the retry mechanism (or the user pressing the key again) handles it.

## Migration Steps

### Step 1: Remove the `has_pending_lsp_response()` gate

Delete the guard at `event_loop.rs:474` and `:919`. Let `process_pending_lsp_actions()` run unconditionally. The per-slot cancellation in each `_impl()` method prevents conflicts.

### Step 2: Convert inline-await actions to spawn-and-poll

Start with `format_document_impl` (most commonly hit). Then `code_actions_impl`, `rename_impl`, `find_references_impl`. Each gets a pending slot and a poll method.

### Step 3: Remove `pending_lsp_action` single slot

Once all actions either dispatch directly or use their own pending slot, the single `Option<LspAction>` slot is unused. Delete it, along with `queue_lsp_action()`, `lsp_action_retry_count`, and the `LspAction` enum.

Input handlers call `_impl()` methods directly instead of queueing.

### Step 4: Remove `prepare_lsp_request` sleeps

Delete the hardcoded sleeps. Keep `ensure_lsp_document_synced()` (flushing pending content is correct).

## Files Changed

| File | Change |
|------|--------|
| `ovim/src/event_loop.rs` | Remove `has_pending_lsp_response()` gates (lines 474, 919) |
| `ovim-core/src/editor/lsp_integration.rs` | Remove `queue_lsp_action`, `process_pending_lsp_actions`, sleeps in `prepare_lsp_request` |
| `ovim-core/src/editor/lsp_state.rs` | Remove `pending_lsp_action`, `lsp_action_retry_count`, eventually `LspAction` enum |
| `ovim-core/src/editor/lsp_modules/actions.rs` | Convert format/code-actions to spawn-and-poll |
| `ovim-core/src/editor/lsp_modules/references.rs` | Convert find-references to spawn-and-poll |
| `ovim-core/src/editor/input/normal/pending_commands.rs` | Call `_impl()` methods directly (requires async input handler or spawn) |

## Open Question: Input Handler Async Boundary

Currently, key handlers are sync (`fn handle_key(...) -> Result<Option<KeyEvent>>`). The `_impl()` methods that spawn tasks need async context. Currently this works because `process_pending_lsp_actions()` is called from the async event loop.

If we remove the single slot and dispatch directly from input handlers, we need async input handling. Two options:

**Option A:** Keep the single slot temporarily, but remove the gate and process it unconditionally every tick. This is the smallest change -- `queue_lsp_action` still exists but is never blocked.

**Option B:** Make input processing async. The event loop already calls `process_input_events()` in an async context. The key handlers can return an `Option<LspAction>` that the event loop processes immediately.

Option A is pragmatic and gets 90% of the benefit. Option B is cleaner but touches more code. Recommend A first, B later if the slot becomes a bottleneck again.

## Verification

1. **Concurrent actions test:** Trigger hover (`K`) and immediately goto-definition (`gd`). Both should complete independently.
2. **Fast input test:** Rapid `gd` `gd` `gd` -- each should cancel the previous. Final result should be the last one.
3. **Format latency test:** Measure time from `gq` keypress to "Formatting..." status. Should be < 5ms (no sleep).
4. **No regression:** All existing LSP tests pass.
