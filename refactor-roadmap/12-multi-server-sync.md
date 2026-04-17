# 12: Multi-Server Document Sync (RETIRED)

> **Carried forward as** [17-multi-server-sync.md](./17-multi-server-sync.md) with the same recommendation (Option B: periodic re-sync).

---

**Goal:** Document versions are tracked per-server so that server crashes, reconnections, or missed notifications are detected and recovered.

**Fixes:** Potential stale diagnostics or wrong completions when one server in a multi-server setup (e.g., TypeScript + Tailwind CSS) falls behind.

**Risk:** Medium. The fix is straightforward but touches the version tracking in `LspManager`.

## The Problem

`LspManager` tracks document versions in two shared maps (`lsp/mod.rs:164,170`):

```rust
document_versions: Mutex<HashMap<Uri, i32>>,      // per-file, shared across servers
last_sent_versions: Mutex<HashMap<Uri, i32>>,      // per-file, shared across servers
```

When `did_change_broadcast` sends updates, it loops through all servers for a language but uses a single version counter for the file. If one server misses a notification (crash, network issue, overloaded), the editor has no way to detect the divergence.

### Scenario

1. Open a TSX file — TypeScript and Tailwind CSS servers both receive `didOpen(v1)`
2. User edits — both receive `didChange(v2)`, `didChange(v3)`
3. Tailwind CSS server crashes silently (no notification to editor)
4. User edits — TypeScript receives `didChange(v4)`, Tailwind does not
5. Tailwind restarts and reconnects
6. Editor thinks both servers are at v4 — no re-sync triggered
7. Tailwind serves completions based on v3 content — wrong results

### Current mitigation

The `LanguageServer` struct has a `notification_listener` that detects when the server process exits. If the server is supervised, the supervisor restarts it and the LSP init path re-opens the document. This covers clean crashes but not silent hangs or dropped notifications over stdio.

## The Fix

### Option A: Per-server version tracking

Change version maps from `HashMap<Uri, i32>` to `HashMap<(Uri, ServerId), i32>`. `did_change_broadcast` tracks which servers successfully received each notification. `ensure_lsp_document_synced` can detect per-server divergence and re-send `didOpen` to lagging servers.

**Complexity:** Medium. The broadcast already loops through servers — it just needs to track success/failure per server.

### Option B: Periodic re-sync check

On each document sync tick, compare the server's last-known version with the editor's current version. If they diverge beyond a threshold, force a `didClose` + `didOpen` to re-sync.

**Complexity:** Low. Doesn't require per-server version tracking — just a periodic health check.

### Recommendation

Option B for now — simpler and covers the failure mode. Option A if multi-server setups become more common.

## Files

- `ovim-core/src/lsp/mod.rs` — `document_versions`, `last_sent_versions`
- `ovim-core/src/lsp/notifications.rs` — `did_change_broadcast`, `flush_pending_changes_broadcast`
- `ovim-core/src/editor/lsp_integration.rs` — `ensure_lsp_document_synced`
