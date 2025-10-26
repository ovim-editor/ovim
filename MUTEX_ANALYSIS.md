# Mutex and Lock Usage Analysis for ovim

**Analysis Date:** 2025-10-26
**Focus Areas:** src/lsp/, src/editor/, src/main.rs, src/event_loop.rs

## Executive Summary

The codebase shows **generally good lock management** with minimal deadlock risk. The architecture uses:
- **DashMap** for lock-free concurrent access (servers in LspManager)
- **Tokio async Mutex** for async operations
- **AtomicBool** for cached capability flags (lock-free reads)
- Proper lock scope management with explicit `drop()` calls

### Risk Level: **LOW to MEDIUM**

## Critical Findings

### 1. **POTENTIAL DEADLOCK: health_check() with nested awaits** ⚠️

**Location:** `src/lsp/mod.rs:306-314`

```rust
pub async fn health_check(&self) -> Vec<LanguageServerHealth> {
    let mut health_infos = Vec::new();

    for entry in self.servers.iter() {
        health_infos.push(entry.value().health_check().await);  // NESTED AWAIT
    }

    health_infos
}
```

**Issue:**
- Iterates over `DashMap::iter()` (which holds internal read lock)
- Calls `server.health_check().await` which acquires multiple locks:
  - `pending_requests.lock().await`
  - `capabilities.lock().await`
  - `supervisor.health_check().await` → `tasks.lock().await`
  - `process.lock().await`

**Risk:** Low (DashMap's iter is lock-free per entry, but could cause contention)

**Recommendation:**
```rust
pub async fn health_check(&self) -> Vec<LanguageServerHealth> {
    let servers: Vec<_> = self.servers.iter()
        .map(|entry| entry.value().clone())
        .collect();

    let mut health_infos = Vec::new();
    for server in servers {
        health_infos.push(server.health_check().await);
    }
    health_infos
}
```

---

### 2. **LOCK ORDERING: No consistent order enforced** ⚠️

The codebase has multiple Mutexes but no documented lock ordering:

**Locks in LspManager:**
- `diagnostics: Mutex<HashMap<...>>`
- `document_versions: Mutex<HashMap<...>>`
- `notification_rx: Mutex<Receiver<...>>`
- `flush_rx: Mutex<Option<Receiver<...>>>`
- `current_progress: Mutex<HashMap<...>>`

**Locks in LanguageServer:**
- `process: Mutex<Option<Child>>`
- `stdin: Arc<Mutex<ChildStdin>>`
- `state: Arc<Mutex<ServerState>>`
- `capabilities: Mutex<Option<ServerCapabilities>>`
- `pending_requests: Mutex<HashMap<...>>`
- `incoming_rx: Mutex<Option<Receiver<...>>>`

**Locks in TaskSupervisor:**
- `tasks: Mutex<HashMap<...>>`

**Issue:** No consistent lock acquisition order documented. Potential for deadlock if locks are acquired in different orders.

**Current Mitigation:**
- Locks are typically short-lived
- Most code paths only acquire one lock at a time
- Good use of `drop()` to release locks early

**Recommendation:**
Document a strict lock ordering hierarchy:
```
Level 0: TaskSupervisor::tasks
Level 1: LanguageServer::state
Level 2: LanguageServer::pending_requests, capabilities, process
Level 3: LspManager::diagnostics, document_versions, current_progress
```

---

### 3. **GOOD PATTERN: Explicit lock release before async calls** ✅

**Location:** Multiple places

```rust
// GOOD: src/lsp/mod.rs:462-476
pub async fn flush_pending_changes(&self, uri: &Url) -> Result<()> {
    if let Some((_, debouncer_arc)) = self.change_debouncers.remove(uri) {
        let mut debouncer = debouncer_arc.lock().await;
        debouncer.cancel_timer();

        let language_id = debouncer.language_id.clone();
        let text = debouncer.pending_text.clone();
        let old_text = debouncer.old_text.clone();
        let uri = debouncer.uri.clone();
        drop(debouncer); // GOOD: Release lock before async call

        self.send_did_change_immediate(uri, &language_id, text, old_text).await?;
    }
    Ok(())
}
```

**Analysis:** This pattern is used consistently throughout the codebase. No locks held across await points.

---

### 4. **GOOD PATTERN: Lock-free capability checks** ✅

**Location:** `src/lsp/server.rs:1263-1371`

```rust
// Cached capability flags - lock-free reads
cap_goto_definition: AtomicBool,
cap_hover: AtomicBool,
cap_completion: AtomicBool,
// ... etc

pub async fn supports_hover(&self) -> bool {
    self.inner.cap_hover.load(Ordering::Relaxed)  // NO LOCK
}
```

**Analysis:** Excellent optimization. Capabilities are cached as `AtomicBool` during initialization, allowing lock-free reads.

---

### 5. **NO DEADLOCK: process_editor_tick is non-blocking** ✅

**Location:** `src/event_loop.rs:23-65`

```rust
async fn process_editor_tick(editor: &mut Editor, ...) {
    // All operations are non-blocking or properly await
    while let Ok(status) = java_status_rx.try_recv() { ... }

    if let Some(lsp_manager) = editor.lsp_manager() {
        lsp_manager.process_notifications().await;
        lsp_manager.process_flush_requests().await;
    }
    // ... etc
}
```

**Analysis:** Uses `try_recv()` to avoid blocking. All async operations properly await without holding locks.

---

### 6. **POTENTIAL ISSUE: pending_requests during cleanup** ⚠️

**Location:** `src/lsp/server.rs:346-390`

```rust
async move {
    loop {
        tokio::time::sleep(CLEANUP_INTERVAL).await;

        let mut pending = inner.pending_requests.lock().await;  // LOCK HELD
        let now = Instant::now();

        let stale_ids: Vec<RequestId> = pending
            .iter()
            .filter_map(|...| { ... })  // EXPENSIVE ITERATION
            .collect();

        for id in stale_ids {
            if let Some(req) = pending.remove(&id) { ... }
        }
        // LOCK HELD FOR ENTIRE CLEANUP CYCLE
    }
}
```

**Issue:** Lock held during iteration and removal. Could block incoming responses.

**Impact:** Low - cleanup runs every 10 seconds, iteration is fast

**Recommendation:**
```rust
let mut pending = inner.pending_requests.lock().await;
let stale_ids: Vec<_> = pending
    .iter()
    .filter_map(|(id, req)| {
        if now.duration_since(req.sent_at) > REQUEST_STALE_TIMEOUT {
            Some(id.clone())
        } else { None }
    })
    .collect();
drop(pending);  // RELEASE LOCK

// Re-acquire for removal
let mut pending = inner.pending_requests.lock().await;
for id in stale_ids {
    if let Some(req) = pending.remove(&id) {
        let _ = req.sender.send(Err(...));
    }
}
```

---

### 7. **NO DEADLOCK: try_lock used for non-critical reads** ✅

**Location:** `src/lsp/mod.rs:180-191`

```rust
pub fn get_progress_message(&self) -> Option<String> {
    if let Ok(progress) = self.current_progress.try_lock() {  // NON-BLOCKING
        if !progress.is_empty() {
            progress.values().next().cloned()
        } else {
            None
        }
    } else {
        None  // Returns None if lock contended
    }
}
```

**Analysis:** Good use of `try_lock()` for optional reads. Won't block if lock is held.

---

### 8. **ASYNC TASK SPAWNING: No lock held** ✅

**Location:** `src/lsp/mod.rs:522-530`

```rust
let handle = tokio::spawn(async move {
    tokio::time::sleep(Duration::from_millis(CHANGE_DEBOUNCE_MS)).await;

    // Timer expired - request flush via channel
    if let Err(e) = flush_tx.send(uri_clone).await {
        lsp_error!("Debounce", "Error sending flush request: {}", e);
    }
});
```

**Analysis:** Background tasks properly use channels instead of shared state. No locks held across task boundaries.

---

## Lock Contention Analysis

### High Contention Locks (used frequently):

1. **`pending_requests`** (src/lsp/server.rs)
   - **Reads:** Every response from LSP server (line 475)
   - **Writes:** Every request sent (line 859), timeouts (line 933), cleanup (line 346)
   - **Mitigation:** Short critical sections, proper lock release
   - **Risk:** LOW - operations are fast

2. **`state`** (src/lsp/server.rs)
   - **Reads:** `is_ready()`, `get_state()`
   - **Writes:** State transitions during initialization/shutdown
   - **Risk:** LOW - mostly read operations, writes are rare

3. **`diagnostics`** (src/lsp/mod.rs)
   - **Reads:** UI rendering, health checks
   - **Writes:** On LSP notification
   - **Risk:** LOW - DashMap for servers reduces contention

### Low Contention Locks:

- `document_versions` - Only modified on file open/close
- `capabilities` - Written once during initialization
- `process` - Only accessed during spawn/shutdown
- `current_progress` - Infrequent updates

---

## Lock Scope Analysis

### Good Practices Found:

1. **Early lock release with `drop()`:**
   ```rust
   let mut versions = self.document_versions.lock().await;
   versions.remove(&uri);
   drop(versions);  // Explicit release
   ```

2. **Short critical sections:**
   ```rust
   {
       let mut pending = self.inner.pending_requests.lock().await;
       pending.insert(request_id.clone(), ...);
   }  // Lock released immediately
   ```

3. **Data cloning before async:**
   ```rust
   let debouncer = debouncer_arc.lock().await;
   let text = debouncer.pending_text.clone();
   drop(debouncer);

   self.send_did_change_immediate(..., text, ...).await?;
   ```

---

## Async + Mutex Pitfalls

### ✅ GOOD: No `.await` while holding locks

Verified patterns:
- All locks released before `.await` calls
- Data cloned, locks dropped, then async operations performed
- Channels used for cross-task communication

### ✅ GOOD: No std::sync::Mutex in async code

All Mutexes are `tokio::sync::Mutex` - correct for async contexts.

### ⚠️ WATCH: DashMap iteration with async calls

Current usage is safe but could be improved (see Finding #1).

---

## Recommendations

### Priority 1 (High):

1. **Fix health_check() nested await pattern**
   - Collect servers first, then iterate
   - Prevents holding DashMap ref during async operations

### Priority 2 (Medium):

2. **Document lock ordering hierarchy**
   - Add comments in code
   - Create lock acquisition rules

3. **Optimize cleanup task lock scope**
   - Split iteration and removal into separate lock acquisitions

### Priority 3 (Low):

4. **Add lock contention metrics**
   - Track lock acquisition times
   - Monitor for performance issues

5. **Consider RwLock for read-heavy data**
   - `diagnostics` is read-heavy, could benefit from `RwLock`
   - `capabilities` is also read-heavy after initialization

---

## Testing Recommendations

### Deadlock Testing:

1. **Stress test health checks:**
   ```rust
   // Spawn 100 concurrent health_check() calls
   // Verify no deadlock or slowdown
   ```

2. **Race condition testing:**
   ```rust
   // Rapidly send LSP requests while processing responses
   // Verify pending_requests map doesn't corrupt
   ```

3. **Shutdown testing:**
   ```rust
   // Send requests while shutting down server
   // Verify graceful cleanup without panics
   ```

---

## Conclusion

The codebase demonstrates **solid lock management practices**:
- ✅ Proper async Mutex usage
- ✅ Short critical sections
- ✅ Lock-free optimizations where appropriate
- ✅ No await points while holding locks
- ⚠️ Minor improvements needed for health_check and cleanup

**Overall Assessment:** Production-ready with low deadlock risk. Recommended improvements are optimizations rather than critical fixes.

---

## Code Examples for Improvements

### Before (health_check):
```rust
pub async fn health_check(&self) -> Vec<LanguageServerHealth> {
    let mut health_infos = Vec::new();
    for entry in self.servers.iter() {
        health_infos.push(entry.value().health_check().await);
    }
    health_infos
}
```

### After (health_check):
```rust
pub async fn health_check(&self) -> Vec<LanguageServerHealth> {
    // Clone servers first to release DashMap refs
    let servers: Vec<_> = self.servers.iter()
        .map(|entry| entry.value().clone())
        .collect();

    // Now we can safely await without holding any DashMap refs
    let mut health_infos = Vec::new();
    for server in servers {
        health_infos.push(server.health_check().await);
    }
    health_infos
}
```

---

## Lock Hierarchy Diagram

```
┌─────────────────────────────────────┐
│     TaskSupervisor::tasks           │ Level 0 (Outermost)
└─────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────┐
│     LanguageServer::state           │ Level 1
└─────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────┐
│  LanguageServer::pending_requests   │ Level 2
│  LanguageServer::capabilities       │
│  LanguageServer::process            │
└─────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────┐
│  LspManager::diagnostics            │ Level 3 (Innermost)
│  LspManager::document_versions      │
│  LspManager::current_progress       │
└─────────────────────────────────────┘
```

**Rule:** Always acquire locks from outer to inner. Never acquire outer lock while holding inner lock.
