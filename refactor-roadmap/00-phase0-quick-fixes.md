# Phase 0: Quick Fixes

**Goal:** Fix the acute symptoms with surgical changes that are correct regardless of what happens in later phases. No double work -- each fix is either the final fix or a prerequisite for the structural work.

**Risk:** Low. Each change is small, independent, and testable in isolation.

## Fix 1: Register before send (server.rs)

**Symptom:** LSP stops working after first requests.

**Change:** Swap the order of `outgoing_tx.send()` and `pending_requests.insert()` in `send_request()`. Add cleanup on send failure. Add logging for unmatched responses.

**File:** `ovim-core/src/lsp/server.rs:1183-1202`

```rust
// Before:
self.inner.outgoing_tx.send(msg).await?;
let mut pending = self.inner.pending_requests.lock().await;
pending.insert(request_id.clone(), PendingRequest { ... });

// After:
{
    let mut pending = self.inner.pending_requests.lock().await;
    pending.insert(request_id.clone(), PendingRequest { ... });
}
if let Err(e) = self.inner.outgoing_tx.send(msg).await {
    let mut pending = self.inner.pending_requests.lock().await;
    pending.remove(&request_id);
    return Err(anyhow!("Channel closed: {}", e));
}
```

Also add logging at line ~562 for the `else` case (response arrives for unknown request).

**Why no double work:** This IS the fix. Phase 1 says the same thing. There's no "better" version later.

## Fix 2: Remove the action gate (event_loop.rs)

**Symptom:** One slow/stuck LSP response blocks all other actions (hover blocks goto-definition).

**Change:** Remove the `if !editor.has_pending_lsp_response()` guard at two call sites.

**File:** `ovim/src/event_loop.rs:474` and `ovim/src/event_loop.rs:919`

```rust
// Before:
if !editor.has_pending_lsp_response() {
    editor.process_pending_lsp_actions().await;
}

// After:
editor.process_pending_lsp_actions().await;
```

**Why this is safe now:** The `_impl()` methods that spawn background tasks (goto, hover) already cancel previous requests in the same slot before starting new ones. The gate was preventing this cancellation from happening.

**Why no double work:** Phase 2 removes this gate as step 1. Whether or not we later restructure the action dispatch, this gate needs to go.

## Fix 3: Debouncer old_text staleness (notifications.rs)

**Symptom:** Wrong diagnostics after undo. LSP server's view of the document diverges from the editor.

**Change:** Always update `old_text` when the caller provides a baseline.

**File:** `ovim-core/src/lsp/notifications.rs:300-302`

```rust
// Before:
if debouncer.old_text.is_none() {
    debouncer.old_text = old_text;
}

// After:
if let Some(new_old) = old_text {
    debouncer.old_text = Some(new_old);
}
```

**Why this works:** The caller (`send_lsp_changes_if_modified`) always passes `last_flushed_content` as `old_text` -- what the server actually has. Between flushes, this value doesn't change, so rapid typing passes the same baseline repeatedly (correct). After undo, it passes the same baseline (also correct -- the server hasn't changed). The current guard is only wrong because it prevents updating `old_text` when the debouncer already has a stale value from a previous change cycle.

**Test to add:**

```rust
#[tokio::test]
async fn undo_sends_correct_content_to_lsp() {
    // Setup: buffer="hello\n", synced to LSP
    // Edit: type "x" -> "hellox\n", flush to LSP
    // Edit: type "y" -> "helloxy\n" (NOT flushed)
    // Undo: -> "hellox\n"
    // Flush: server should receive content matching "hellox\n"
    // Verify: the diff sent is against "hellox\n" (last flushed),
    //         producing no change (content is identical to what server has)
}
```

**Why no double work:** Phase 3's deeper cleanup simplifies `DocumentSyncState`, but this line fix is a prerequisite regardless. The structural simplification builds on correct baseline behavior.

## Fix 4: Git operations off the event loop (file_io.rs + commands.rs)

**Symptom:** Editor freezes on `:w` for 200ms-3s.

**Change:** Move `refresh_git_status()` and `load_git_blame()` to `spawn_blocking`. Add a channel for results. Drain results in the tick.

**Files:**
- `ovim-core/src/buffer/file_io.rs:254-256` -- remove `refresh_git_status()` from `save_as()`
- `ovim-core/src/commands.rs:57-63` -- spawn git ops in background after save
- `ovim-core/src/editor/mod.rs` -- add `git_refresh_tx/rx` channel
- `ovim/src/event_loop.rs` -- drain `git_refresh_rx` in tick

**Why no double work:** Phase 4 says the same thing. The full `CommandOutcome::Async` for the file write itself is optional and independent of this change.

## Fix 5: Remove gratuitous sleep (lsp_integration.rs)

**Symptom:** Every LSP action (hover, goto, format, etc.) pays 10ms+ latency for no reason.

**Change:** Remove the hardcoded sleep in `prepare_lsp_request`.

**File:** `ovim-core/src/editor/lsp_integration.rs:1795`

```rust
// Before:
let did_flush = self.ensure_lsp_document_synced().await;
if did_flush {
    tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;
}
tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

// After:
self.ensure_lsp_document_synced().await;
```

**Why no double work:** There's no scenario where this sleep is reintroduced. It's dead code.

## Order of Implementation

1. **Fix 1** (register-before-send) -- highest impact, fixes the most visible symptom
2. **Fix 2** (remove gate) -- immediately benefits from Fix 1 (no more stuck responses blocking the gate)
3. **Fix 3** (debouncer old_text) -- fixes the content corruption, add the test
4. **Fix 5** (remove sleep) -- one line, free latency win
5. **Fix 4** (git spawn_blocking) -- slightly more plumbing, independent of 1-3

Fixes 1-3 and 5 can be done in a single session. Fix 4 needs a channel and tick handler, so it's a bit more work but still small.

## What This Leaves for Later

After Phase 0, the remaining phases are all *structural improvement*, not *bug fixes*:

- **Phase 2 remainder:** Convert inline-await actions (format, code-actions) to spawn-and-poll. Remove the single `Option<LspAction>` slot.
- **Phase 3 remainder:** Simplify `DocumentSyncState`, reduce reconciliation complexity.
- **Phase 5:** Decoration projection (edit log + immutable versioned decorations). Fixes inlay hint drift structurally.
- **Phase 6:** `LspState` decomposition. Pure maintainability refactor.

These are "make it pleasant to work in" changes, not "fix the thing that's broken" changes. They can be done at whatever pace makes sense.
