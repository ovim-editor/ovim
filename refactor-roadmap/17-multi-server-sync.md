# 17: Multi-Server Document Sync

**Goal:** Detect and recover from silent server divergence in multi-server setups (e.g., TypeScript + Tailwind CSS).

**Fixes:** Wrong completions/diagnostics when a companion server silently crashes or drops a `didChange` notification.

**Risk:** Medium. Touches version tracking in `LspManager`.

## The Problem

`LspManager` tracks document versions per-file, not per-server:

```rust
document_versions: Mutex<HashMap<Uri, i32>>,      // shared across servers
last_sent_versions: Mutex<HashMap<Uri, i32>>,      // shared across servers
```

When `did_change_broadcast` sends updates, it loops through all servers for a language but uses a single version counter. If one server misses a notification (crash, network issue, overloaded), the editor can't detect the divergence.

### Failure scenario

1. Open TSX file — TypeScript and Tailwind CSS servers both receive `didOpen(v1)`
2. User edits — both receive `didChange(v2)`, `didChange(v3)`
3. Tailwind CSS server crashes silently (no exit notification)
4. User edits — TypeScript receives `didChange(v4)`, Tailwind does not
5. Tailwind restarts and reconnects
6. Editor thinks both servers are at v4 — no re-sync triggered
7. Tailwind serves completions based on v3 content — wrong results

### Current mitigation

`LanguageServer`'s `notification_listener` detects when the server process exits. If supervised, the supervisor restarts it and the LSP init path re-opens the document. This covers clean crashes but not silent hangs or dropped notifications over stdio.

## Recommendation: Periodic re-sync check (Option B)

On each document sync tick, compare each server's expected state with what we've sent. If divergence is detected (server was restarted, version mismatch), force a `didClose` + `didOpen` to re-sync.

This is simpler than per-server version tracking (Option A) and covers the failure mode. Option A becomes worthwhile if multi-server setups grow beyond two servers per file.

### Implementation sketch

In `ensure_lsp_document_synced()` or the sync tick:

```rust
// For each server registered for this file:
//   If server was restarted since last didOpen → re-send didOpen with current content
//   If server health check fails → mark for re-sync on next successful health check
```

The `LanguageServer` struct already has `is_alive()` — extend this with a `last_restart_epoch` or similar to detect restart-without-notification.

### What NOT to do

- Don't add per-server version maps unless you have three or more servers per file. The complexity isn't justified for the two-server case.
- Don't poll server health on every tick — a periodic check (every 5–10 seconds) is sufficient.

## Files

- `ovim-core/src/lsp/mod.rs` — `document_versions`, `last_sent_versions`
- `ovim-core/src/lsp/notifications.rs` — `did_change_broadcast`, `flush_pending_changes_broadcast`
- `ovim-core/src/editor/lsp_integration.rs` — `ensure_lsp_document_synced`
- `ovim-core/src/lsp/server.rs` — `LanguageServer` health tracking
