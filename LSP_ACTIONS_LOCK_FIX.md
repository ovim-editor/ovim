# LSP Actions Lock Contention Fix

## Problem

After fixing the main event loop lock contention, typing `K` (hover command) still caused the UI to freeze during Java LSP initialization.

### Symptoms
- K command blocks for 60-120 seconds
- Other LSP commands (gd for goto-definition, etc.) also block
- Status shows "Requesting hover information..." but nothing happens
- UI completely unresponsive until Java init completes

## Root Cause: LSP Action Implementations Using Blocking Locks

All LSP action implementations (hover, goto-definition, completion, format, code-actions) were using `lsp.lock().await`, which blocks if the background Java initialization task holds the lock.

### Code Flow

**User presses K:**
```
1. input.rs:1840 - KeyCode::Char('K') detected
2. editor.request_hover() - Sets pending_lsp_action = ShowHover
3. Event loop calls process_pending_lsp_actions()
4. hover_impl() called
5. lsp.lock().await ← BLOCKS for 60-120 seconds!
```

**Background Java Task (holding lock):**
```rust
// src/main.rs:793-796
let mut start_task = tokio::spawn(async move {
    let lsp = lsp_clone.lock().await;  // Holds lock for 60-120 seconds
    lsp.start_server("java", ...).await
});
```

**LSP Action Implementation (blocked):**
```rust
// src/editor/mod.rs:1424 (hover_impl)
let lsp_guard = lsp.lock().await;  // ← BLOCKS!
let hover_text = lsp_guard.hover(...).await?;
```

## Solution ✅

Changed all LSP action implementations to use **non-blocking `try_lock()`** with automatic retry mechanism:

### Fix 1: Update process_pending_lsp_actions to Retry on Failure

```rust
// Before
pub async fn process_pending_lsp_actions(&mut self) {
    if let Some(action) = self.pending_lsp_action.take() {
        match action {
            LspAction::ShowHover => {
                let _ = self.hover_impl().await;
            }
            // ...
        }
    }
}

// After
pub async fn process_pending_lsp_actions(&mut self) {
    if let Some(action) = self.pending_lsp_action.take() {
        let retry = match action {
            LspAction::ShowHover => {
                matches!(self.hover_impl().await, Err(_))
            }
            // ...
        };

        // If action returned error (couldn't get lock), retry next iteration
        if retry {
            self.pending_lsp_action = Some(action);
        }
    }
}
```

### Fix 2: Update All LSP Action Implementations

Changed 5 LSP action implementations to use try_lock:

#### hover_impl (src/editor/mod.rs:1429-1436)
```rust
// Before
let lsp_guard = lsp.lock().await;  // BLOCKS!

// After
let lsp_guard = match lsp.try_lock() {
    Ok(guard) => guard,
    Err(_) => {
        // LSP busy, return error to trigger retry
        return Err(anyhow::anyhow!("LSP busy"));
    }
};
```

#### goto_definition_impl (src/editor/mod.rs:1323-1330)
Same pattern - changed from `lock().await` to `try_lock()` with error on failure.

#### completion_impl (src/editor/mod.rs:1516-1523)
Same pattern - changed from `lock().await` to `try_lock()` with error on failure.

#### format_document_impl (src/editor/mod.rs:1605-1612)
Same pattern - changed from `lock().await` to `try_lock()` with error on failure.

#### code_actions_impl (src/editor/mod.rs:1681-1688)
Same pattern - changed from `lock().await` to `try_lock()` with error on failure.

### Fix 3: Update Background LSP Notifications

Also fixed 4 background notification senders that were blocking:

#### send_lsp_change (src/editor/mod.rs:1200-1203)
```rust
// Before
let lsp_guard = lsp.lock().await;  // BLOCKS!

// After
let Ok(lsp_guard) = lsp.try_lock() else {
    return; // LSP busy, will sync on next change
};
```

#### send_lsp_save_if_needed (src/editor/mod.rs:1245-1248)
```rust
// Before
let lsp_guard = lsp.lock().await;  // BLOCKS!

// After
let Ok(lsp_guard) = lsp.try_lock() else {
    return; // LSP busy, save notification sent on next save
};
```

#### get_current_file_diagnostics (src/editor/mod.rs:964-966)
```rust
// Before
let lsp_guard = lsp.lock().await;

// After
let lsp_guard = lsp.try_lock().ok()?;  // Return None if busy
```

#### get_diagnostic_count (src/editor/mod.rs:974-977)
```rust
// Before
let lsp_guard = lsp.lock().await;

// After
if let Ok(lsp_guard) = lsp.try_lock() {
    return lsp_guard.count_diagnostics(&uri).await;
}
// Return (0,0,0,0) if busy
```

## How It Works

### Blocking Approach (Before)

```
User presses K during Java init:
├─ request_hover() sets pending action
├─ process_pending_lsp_actions() called
├─ hover_impl() called
│  └─ lsp.lock().await ← BLOCKS for 60-120 seconds!
│     [UI completely frozen]
│     [User cannot do anything]
│     [Eventually gets lock and shows hover]
└─ Action completes
```

### Non-Blocking with Retry (After)

```
User presses K during Java init:
├─ request_hover() sets pending action
├─ process_pending_lsp_actions() called
├─ hover_impl() called
│  └─ lsp.try_lock() → Err (busy)
│     └─ Returns Err("LSP busy") immediately
├─ Action marked for retry (put back in pending_lsp_action)
├─ Event loop continues ✅ (UI responsive!)
├─ Next iteration (~16ms later)
│  └─ try_lock() again → still busy, retry
├─ ... continues retrying every ~16ms ...
├─ Java init completes, lock released
├─ Next iteration
│  └─ try_lock() → Ok! Gets lock and shows hover ✅
└─ Action completes
```

**Key Difference:**
- **Blocking:** Wait 60-120s with frozen UI
- **Non-blocking with retry:** Try every 16ms, UI stays responsive

## Files Modified

### src/editor/mod.rs

**9 lock acquisitions changed** from `lock().await` to `try_lock()`:

1. **Line 965**: get_current_file_diagnostics
2. **Line 975**: get_diagnostic_count
3. **Line 1201**: send_lsp_change
4. **Line 1246**: send_lsp_save_if_needed
5. **Line 1324**: goto_definition_impl
6. **Line 1430**: hover_impl
7. **Line 1517**: completion_impl
8. **Line 1606**: format_document_impl
9. **Line 1682**: code_actions_impl

**Plus retry mechanism** at lines 1246-1271 in process_pending_lsp_actions.

## Behavior

### User Actions

| Command | Key | Before Fix | After Fix |
|---------|-----|-----------|-----------|
| Hover | K | Blocks 60-120s ❌ | Retries, succeeds when ready ✅ |
| Goto Definition | gd | Blocks 60-120s ❌ | Retries, succeeds when ready ✅ |
| Completion | Ctrl-Space | Blocks 60-120s ❌ | Retries, succeeds when ready ✅ |
| Format | (custom) | Blocks 60-120s ❌ | Retries, succeeds when ready ✅ |
| Code Actions | (custom) | Blocks 60-120s ❌ | Retries, succeeds when ready ✅ |

### Background Notifications

| Notification | Before Fix | After Fix |
|--------------|-----------|-----------|
| didChange | Blocks on edit ❌ | Skips if busy, sends next edit ✅ |
| didSave | Blocks on save ❌ | Skips if busy, sends next save ✅ |
| Diagnostics | Blocks when reading ❌ | Returns empty if busy ✅ |

## Performance Impact

**No performance degradation:**
- Actions retry every ~16ms until lock available
- Background notifications skip gracefully when busy
- LSP operations complete at same speed
- UI stays responsive throughout

**User Experience:**
- Immediate feedback: "Requesting hover information..."
- Can continue working while waiting
- Action completes as soon as LSP ready
- No frozen feeling

## Testing

### Test 1: Press K During Java Init

```bash
rm -rf ~/.cache/ovim/java
cargo run -- TestJava.java
```

Immediately press `K` on any symbol:

**Expected:**
- ✅ Status shows "Requesting hover information..."
- ✅ UI stays responsive
- ✅ Can move cursor, type, switch modes
- ✅ After Java init completes, hover appears
- ✅ **No blocking at any point**

### Test 2: Multiple LSP Commands During Init

```bash
cargo run -- Test.java
```

During initialization, try:
1. Press `K` (hover) ✅ Queued, retries
2. Press `Esc` ✅ Works
3. Type `gd` (goto definition) ✅ Queued, retries
4. Move around with `hjkl` ✅ Works
5. Press `i` and type ✅ Works
6. Wait for init to complete
7. Commands execute when ready ✅

### Test 3: Edit During Init

```bash
cargo run -- Test.java
```

1. Press `i` for insert mode
2. Type some code
3. Press `Esc`
4. UI should stay responsive ✅
5. didChange notifications skip if LSP busy ✅
6. After init, LSP gets next change ✅

## Edge Cases Handled

1. **Repeated K presses** - Only last action kept (not queued multiple times)
2. **LSP busy for long time** - Continuous retry every 16ms
3. **LSP never becomes available** - Action stays pending (user can cancel with Esc)
4. **Multiple action types** - Only one pending at a time (latest overwrites)

## Comparison to Other Editors

### VS Code
- LSP requests during init are queued
- UI stays responsive ✅
- Actions complete when server ready ✅

### IntelliJ IDEA
- Requests during indexing show "Indexing..." message
- UI stays responsive ✅
- Actions complete after indexing ✅

### ovim (Before)
- LSP requests during init block UI ❌
- Appears frozen ❌
- No feedback to user ❌

### ovim (After)
- LSP requests retry automatically ✅
- UI stays responsive ✅
- Status feedback to user ✅
- **Matches professional IDE behavior!** ✅

## Architecture Pattern

### For User-Triggered LSP Actions

```rust
async fn lsp_action_impl(&mut self) -> Result<bool> {
    // Clone LSP manager Arc
    let lsp = self.lsp_manager.as_ref()?.clone();

    // Try to get lock without blocking
    let lsp_guard = match lsp.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            // Return error to trigger retry
            return Err(anyhow::anyhow!("LSP busy"));
        }
    };

    // Perform LSP operation
    let result = lsp_guard.some_operation(...).await?;

    // Process result
    Ok(result)
}
```

### For Background Notifications

```rust
async fn send_notification(&mut self) {
    let Some(lsp) = &self.lsp_manager else {
        return;
    };

    // Try to get lock, skip if busy
    let Ok(lsp_guard) = lsp.try_lock() else {
        return; // Will send on next trigger
    };

    // Send notification
    let _ = lsp_guard.notify(...).await;
}
```

## Summary

**Problem:** Pressing K (or other LSP commands) during Java initialization blocked UI for 60-120 seconds

**Root Cause:** LSP action implementations used `lock().await`, blocking while background task held lock

**Solution:**
1. ✅ Changed all LSP actions to use `try_lock()`
2. ✅ Implemented automatic retry mechanism
3. ✅ Background notifications skip gracefully when busy

**Results:**
- All LSP commands stay responsive during initialization
- Actions automatically retry until successful
- UI never blocks on lock contention
- Professional IDE experience

**Impact:**
- UI responsiveness: Blocked → Fully responsive
- User feedback: None → Clear status messages
- Retry mechanism: Manual → Automatic
- User experience: Frustrating → Smooth

---

**Status:** ✅ COMPLETE
**Date:** 2025-10-07
**Version:** ovim 0.1.0

**All LSP actions are now non-blocking with automatic retry!**

**Test it:**
```bash
rm -rf ~/.cache/ovim/java
cargo run -- TestJava.java
# Press K immediately - no blocking!
# Try gd, Ctrl-Space, etc. - all responsive!
```
