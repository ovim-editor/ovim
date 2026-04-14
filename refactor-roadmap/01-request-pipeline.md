# Phase 1: Request Pipeline

**Goal:** Make it structurally impossible to send a request without tracking it. Log dropped responses.

**Fixes:** LSP stops working after first requests. Silent response drops.

**Risk:** Low. Contained to LSP transport internals. The `LanguageServer` public API is unchanged.

## The Bug

In `ovim-core/src/lsp/server.rs:1183-1202`:

```rust
// Line 1183: message leaves the building
self.inner.outgoing_tx.send(msg).await?;

// Lines 1194-1202: THEN we start tracking it
let mut pending = self.inner.pending_requests.lock().await;
pending.insert(request_id, PendingRequest { sender: tx, ... });
```

The reader task at line 557 can process the response before `insert` executes. The response is silently dropped (no `else` branch, no logging). The caller waits 10 seconds for a timeout that should never have happened.

## The Fix

### Register before send

The fix is 10 lines, not an architectural change:

```rust
async fn send_request(&self, method: &str, params: Value) -> Result<Value> {
    // ... state checks unchanged ...

    let request_id = RequestId::Number(self.inner.next_request_id.fetch_add(1, Ordering::SeqCst));
    let (tx, rx) = oneshot::channel();
    let msg = build_request_message(&request_id, method, &params);

    // Register FIRST -- the entry exists before the message hits the wire
    {
        let mut pending = self.inner.pending_requests.lock().await;
        pending.insert(request_id.clone(), PendingRequest {
            sender: tx,
            sent_at: Instant::now(),
            method: method.to_string(),
        });
    }

    // THEN send -- if send fails, clean up the registration
    if let Err(e) = self.inner.outgoing_tx.send(msg).await {
        let mut pending = self.inner.pending_requests.lock().await;
        pending.remove(&request_id);
        return Err(anyhow!("Channel closed: {}", e));
    }

    // Wait with timeout (unchanged)
    // ...
}
```

The invariant: **the pending map always has the entry before the message reaches the wire.** If send fails, we clean up. If send succeeds, the reader will always find the entry.

### Log when responses arrive for unknown requests

In the reader task at line 562, add logging for the `None` case:

```rust
if let Some(req) = pending_req {
    // ... handle response (unchanged) ...
} else {
    // This should only happen for requests that timed out and were cleaned up.
    // If this fires frequently, there's a bug in the pipeline.
    crate::lsp_warn!(
        &inner_clone.log_prefix(),
        "Response for unknown request ID {:?} (timed out or already handled)",
        id
    );
}
```

### No structural change needed

The first version of this plan proposed extracting a `RequestPipeline` struct. On reflection, that's over-engineering. The fix is: swap two operations (register before send) and add error handling for the send-failure path. The existing `pending_requests` map, the `outgoing_tx` channel, and the reader task all work correctly -- they just need to be called in the right order.

## Files Changed

| File | Lines | Change |
|------|-------|--------|
| `ovim-core/src/lsp/server.rs` | 1183-1202 | Swap: insert into pending_requests before outgoing_tx.send |
| `ovim-core/src/lsp/server.rs` | ~562 | Add logging for unmatched response IDs |

## Verification

1. **Race test:** Send 100 requests to a fast server in a tight loop. All 100 must receive responses.
2. **Failure test:** Close the outgoing channel. Verify the pending entry is cleaned up (no leak).
3. **Timeout test:** Send a request to a non-responsive server. Verify timeout fires and entry is removed.

## What This Doesn't Fix

- The `has_pending_lsp_response()` gate still blocks new actions while old ones are in flight. That's Phase 2.
- Wrong content after undo still corrupts the LSP view. That's Phase 3.
- Save still freezes. That's Phase 4.
