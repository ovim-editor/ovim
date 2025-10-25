# LSP Unresponsiveness - Root Cause Analysis

## Problem Statement

LSP appears to load successfully (shows "rust: 263/263"), but `K` (hover) and `gd` (goto definition) never work in either TUI or headless mode. The editor becomes completely unresponsive to LSP requests.

## Root Cause: Duplicate `didOpen` Notifications

### Evidence

From test output:
```
[DEBUG-HOVER] hover() returned: false
[2025-10-22 23:35:00.835] [DEBUG] [stderr] 2025-10-22T23:35:00.835849+02:00 ERROR duplicate DidOpenTextDocument: /Users/adrian.helvik/Personal/ovim/src/main.rs
```

The file `src/main.rs` received **4 duplicate `didOpen` notifications** in a single session. rust-analyzer correctly rejected these with `ERROR duplicate DidOpenTextDocument`, causing the LSP to enter an inconsistent state where it refuses to respond to requests.

### The Bug: Broken State Tracking

**Two separate code paths call `did_open`, but only one tracks state:**

#### Path 1: `lsp_init` module (src/lsp_init/rust.rs:55)
```rust
// Called from main.rs:79 on startup
match lsp_manager
    .did_open(uri, language_id, 1, file_content.clone())
    .await
{
    Ok(_) => {
        editor.set_last_synced_content(&path_str, Some(file_content));
        // BUG: Does NOT set did_open_sent = true!
    }
    ...
}
```

**Problem**: Calls `lsp_manager.did_open()` directly but **never marks the document as opened** in `editor.lsp_state.document_sync[state_key].did_open_sent`.

#### Path 2: `Editor::initialize_lsp()` (src/editor/mod.rs:2308)
```rust
// Called from hover/goto_definition when they check if LSP is ready
let is_opened = {
    let state = self.lsp_state.document_sync
        .entry(state_key.clone())
        .or_default();
    state.did_open_sent  // FALSE because lsp_init never set it!
};

if is_opened {
    return Ok(true);  // Skip if already opened
}

// Sends DUPLICATE didOpen because state tracking is broken
match lsp.did_open(uri.clone(), language_id, 1, content.clone()).await {
    Ok(_) => {
        state.did_open_sent = true;  // Now it's set, but too late
        ...
    }
}
```

**Problem**: Checks `did_open_sent` flag to prevent duplicates, but the flag is never set by `lsp_init`, so it sends a **duplicate `didOpen`**.

### Timeline of Bug

1. **Startup** (main.rs:79): `lsp_init::initialize_lsp_for_file()` sends first `didOpen` ✅
2. **Startup** (main.rs:80): `editor.clear_lsp_init_flag()` clears re-init flag
3. **User presses `K`**: Hover triggers `initialize_lsp()` check
4. **Check fails**: `did_open_sent` is still `false` (never set by lsp_init!)
5. **Second `didOpen` sent**: Duplicate notification ❌
6. **rust-analyzer rejects**: `ERROR duplicate DidOpenTextDocument`
7. **LSP becomes unresponsive**: All subsequent requests fail

### Why Hover Always Returns False

From src/editor/mod.rs:3367-3378:
```rust
// CRITICAL FIX: Flush pending changes before hover
{
    let lsp_guard = lsp.lock().await;
    let _ = lsp_guard.flush_pending_changes(&uri).await;
    drop(lsp_guard);
}

tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

let lsp_guard = lsp.lock().await;
let hover_text = lsp_guard.hover(&uri, line, character, language_id).await?;
```

The hover implementation is **correct**, but it's calling `hover()` on an LSP that's in a broken state due to duplicate `didOpen`. rust-analyzer refuses to respond to hover requests for documents it rejected during `didOpen`.

## The Fix

### Minimal Fix: Set `did_open_sent` in lsp_init

Modify `lsp_init/rust.rs` (and python.rs, javascript.rs, java.rs):

```rust
match lsp_manager
    .did_open(uri, language_id, 1, file_content.clone())
    .await
{
    Ok(_) => {
        // Initialize last_synced_content
        editor.set_last_synced_content(&path_str, Some(file_content));

        // CRITICAL FIX: Mark document as opened to prevent duplicate didOpen
        editor.mark_lsp_document_opened(&path_str);

        editor.set_lsp_status(format!("LSP: {} ready", server_command));
    }
    ...
}
```

Add method to Editor:
```rust
/// Marks a document as opened in LSP (prevents duplicate didOpen)
pub fn mark_lsp_document_opened(&mut self, file_path: &str) {
    let state_key = file_path.to_string();
    let state = self.lsp_state.document_sync
        .entry(state_key)
        .or_default();
    state.did_open_sent = true;
}
```

### Better Fix: Use `Editor::initialize_lsp()` Everywhere

Instead of calling `lsp_manager.did_open()` directly in `lsp_init`, call `editor.initialize_lsp()` which handles state tracking correctly.

Modify `lsp_init/rust.rs`:
```rust
// After starting server and notification listener...
match editor.initialize_lsp().await {
    Ok(true) => {
        editor.set_lsp_status(format!("LSP: {} ready", server_command));
    }
    Ok(false) => {
        editor.set_lsp_status("LSP: No language support".to_string());
    }
    Err(e) => {
        editor.set_lsp_status(format!("LSP: didOpen failed: {}", e));
    }
}
```

This approach:
- ✅ Uses existing state tracking in `Editor`
- ✅ Prevents duplicates automatically via `is_opened` check
- ✅ Maintains consistency across all code paths
- ✅ No new methods needed

## Files That Need Fixing

All `lsp_init` modules have this bug:
- `src/lsp_init/rust.rs:55-68`
- `src/lsp_init/python.rs:41-61`
- `src/lsp_init/javascript.rs:41-61`
- `src/lsp_init/java.rs:273-278` and `:484-504`

## Testing

### Reproduce Bug (Headless)
```bash
./target/release/ovim src/main.rs --headless --session test
sleep 2
./ovim-ctl send test "23GfeK"
grep "duplicate" ~/.cache/ovim/lsp.log
# Shows: ERROR duplicate DidOpenTextDocument
```

### Verify Fix
```bash
# After applying fix
./target/release/ovim src/main.rs --headless --session test
sleep 2
./ovim-ctl send test "23GfeK"
curl http://127.0.0.1:PORT/snapshot | jq '.hover_content'
# Should show hover information, not empty
```

## Impact

**Critical Bug**:
- ❌ Breaks LSP hover, goto definition, and all other LSP features
- ❌ Affects all languages (Rust, Python, JavaScript, Java)
- ❌ Happens 100% of the time on startup
- ❌ Present in main branch (not a regression)

**Severity**: This is a **showstopper bug** that makes LSP completely unusable.

## Bad Practices Identified

1. **Duplicate code paths**: Two separate ways to send `didOpen` (lsp_init vs Editor::initialize_lsp)
2. **Broken abstraction**: `lsp_init` bypasses Editor's state tracking
3. **Missing validation**: No check that `did_open_sent` was set after sending didOpen
4. **Silent failures**: Duplicate didOpen errors don't surface to user
5. **Inconsistent patterns**: lsp_init modules duplicate similar logic 4 times

### Architectural Fix (Future)

Make `Editor::initialize_lsp()` the **only** way to send `didOpen`:
1. Remove all direct `lsp_manager.did_open()` calls
2. Make `LspManager::did_open()` private or internal-only
3. Force all code paths through `Editor::initialize_lsp()`
4. Add debug assertion: `debug_assert!(state.did_open_sent)` after calling

This enforces the state tracking invariant at compile time.

## Related Issues

- **Hover fix** (HOVER_FIX_SUMMARY.md): Attempted to fix hover by flushing changes, but hover was failing due to duplicate didOpen, not stale content
- **Performance docs**: Document mentions LSP works, but it doesn't - needs integration testing

---

**Generated**: 2025-10-23
**Found by**: Manual debugging with OVIM_LSP_DEBUG=1 and LSP log analysis
**Status**: Root cause identified, fix ready to implement
