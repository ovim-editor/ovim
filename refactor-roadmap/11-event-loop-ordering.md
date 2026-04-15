# 11: Event Loop Clarity

**Goal:** Make the event loop's structure legible so that modifications don't accidentally break implicit invariants.

**Risk:** Low. This is documentation + optional structural grouping.

## Current State

The event loop tick in `event_loop.rs` runs ~15 phases. Each LSP `_impl()` method calls `ensure_lsp_document_synced()` internally before making its request, so the phases are more independent than they first appear.

### What IS order-dependent

- **LSP init before everything else:** `initialize_lsp_for_file()` must complete before any LSP request can succeed (servers must be registered in the index). Currently at line 85, before intent dispatch at line 476. This is correctly ordered.
- **Diagnostics sync:** `sync_lsp_and_refresh_diagnostics()` combines document sync with diagnostic polling. The sync must happen before diagnostics are checked — but this is internal to the function, not an event loop ordering concern.

### What is NOT order-dependent (contrary to original claim)

- **Document sync vs intent dispatch:** Each `_impl()` method (completion, hover, goto, format, etc.) calls `ensure_lsp_document_synced()` independently. Moving `dispatch_pending_intents()` earlier in the tick would NOT cause stale-content issues.
- **Slot polling vs intent dispatch:** Slots can be polled at any time — they return `None` until a result arrives. Polling before or after dispatch both work.

### Real risks

The event loop is 500+ lines with DAP handling, AI jobs, make operations, git refresh, Lua commands, LSP install progress, and picker ticks interleaved. The risk isn't wrong ordering — it's **comprehension**. Someone adding a new phase doesn't know which of the 15 existing phases matter and which are independent.

## The Fix

Group related phases with section comments. Optionally extract tight groups into named functions:

```rust
// === LSP lifecycle ===
process_lsp_notifications(editor).await;   // notifications, workspace edits
initialize_lsp_if_needed(editor).await;    // server startup

// === LSP sync + query cycle ===
editor.sync_lsp_and_refresh_diagnostics().await;
editor.dispatch_pending_intents().await;
editor.poll_pending_lsp_responses();

// === Background tasks ===
poll_background_tasks(editor);  // AI, make, git, installs
```

This makes the structure scannable without requiring deep knowledge of each phase's dependencies.
