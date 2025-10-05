# LSP Implementation Review

## Summary

The LSP (Language Server Protocol) implementation in ovim is **architecturally complete** but **not functional** due to critical missing components and implementation gaps.

## Architecture Overview

### Components ✅

The LSP implementation consists of well-designed modules:

1. **`src/lsp/mod.rs`** - `LspManager` - Central coordinator
   - Manages multiple language servers
   - Handles diagnostics storage and retrieval
   - Document version tracking
   - Methods for didOpen, didChange, didSave, didClose
   - Request methods for goto-definition and hover

2. **`src/lsp/server.rs`** - `LanguageServer` - Process management
   - Spawns language server processes
   - Handles stdio communication
   - Request/response matching
   - Initialization handshake

3. **`src/lsp/protocol.rs`** - JSON-RPC protocol
   - Message framing with Content-Length headers
   - Request/response/notification types
   - Serialization/deserialization

4. **`src/lsp/types.rs`** - Type conversions
   - Position and Range helpers

### Integration ✅

- Editor has `lsp_manager: Option<Arc<TokioMutex<LspManager>>>`
- `enable_lsp()` method initializes LSP manager
- `initialize_lsp_for_file()` in main.rs starts language servers based on file extension
- Keybindings: `gd` (goto definition), `K` (hover)
- Async processing: `process_pending_lsp_actions()` in event loop

## Critical Issues ❌

### 1. **Language Servers Not Installed**

The main reason LSP doesn't work is that **language servers are not installed**:

```bash
# Rust
$ rust-analyzer --version
# ERROR: Unknown binary 'rust-analyzer' in official toolchain

# JavaScript/TypeScript
$ which typescript-language-server
# Not found

# Python
$ which pylsp
# Not found
```

**Fix:** Install language servers:
```bash
# Rust
rustup component add rust-analyzer

# JavaScript/TypeScript (requires npm)
npm install -g typescript-language-server typescript

# Python (requires pip)
pip install python-lsp-server
```

### 2. **No Notification Listener Started** ❌

The LSP manager has a `start_notification_listener()` method, but **it's never called**:

```rust
// src/lsp/mod.rs:309
pub async fn start_notification_listener(&self, language_id: String) {
    // ... spawns task to listen for notifications
}
```

**Impact:** Diagnostics from the language server are never received because no background task is listening for `textDocument/publishDiagnostics` notifications.

**Fix:** After starting a language server, call:
```rust
lsp_manager.start_notification_listener(language_id.to_string()).await;
```

### 3. **Notification Handling Not Connected** ❌

Even if the listener were started, it doesn't call back to the manager:

```rust
// src/lsp/mod.rs:316-327
tokio::spawn(async move {
    loop {
        if let Some(msg) = server.receive().await {
            if msg.is_notification() {
                // Silently handle notifications
                // In a real implementation, we'd send this to the manager
                // ⚠️ DOES NOTHING!
            }
        } else {
            break; // Server closed
        }
    }
});
```

**Fix:** Send notifications to the manager:
```rust
// Need a channel from listener back to manager
if msg.is_notification() {
    self.handle_notification(&language_id, msg).await;
}
```

But this requires architectural changes since `self` is moved into the spawn.

### 4. **No didChange Notifications** ❌

When the buffer changes, the LSP server is never notified:

- No calls to `lsp_manager.did_change()` in the codebase
- The language server never receives updates about file edits
- Diagnostics become stale after the first edit

**Fix:** After every buffer modification, send didChange:
```rust
// In editor after text changes
if let Some(ref lsp) = self.lsp_manager {
    if let Some(file_path) = self.buffer.file_path() {
        // Build change event and send
        let _ = lsp.lock().await.did_change(...).await;
    }
}
```

### 5. **Synchronization Architecture Problem** ❌

The current design has a fundamental issue:

1. `LspManager::start_notification_listener()` moves the `LanguageServer` into a spawned task
2. But the server is also stored in `LspManager.servers`
3. This causes ownership conflicts

The notification listener needs to be redesigned to work with the shared server or use a different communication pattern (e.g., channels).

### 6. **No Error Handling for LSP Startup** ⚠️

```rust
// src/main.rs:481
if let Err(_e) = lsp.start_server(language_id, server_command, server_args, root_path).await {
    return;  // Silently fails!
}
```

If the language server fails to start (e.g., not installed), it fails silently with no user feedback.

### 7. **Hardcoded Language Server Commands** ⚠️

```rust
// src/main.rs:453-458
let (language_id, server_command, server_args) = match extension {
    "rs" => ("rust", "rust-analyzer", vec![]),
    "js" | "ts" | "jsx" | "tsx" => ("javascript", "typescript-language-server", vec!["--stdio".to_string()]),
    "py" => ("python", "pylsp", vec![]),
    _ => return,
};
```

These paths assume language servers are in PATH. Should be configurable.

## What Works ✅

1. **Protocol Implementation** - JSON-RPC message handling is correct
2. **Process Spawning** - Can spawn language server processes
3. **Initialization** - LSP initialize/initialized handshake works
4. **didOpen** - Files are opened correctly with the language server
5. **Request/Response** - goto-definition and hover requests work (if server responds)
6. **Keybindings** - `gd` and `K` are properly bound

## What Doesn't Work ❌

1. **Diagnostics** - Never received because no notification listener
2. **Live Updates** - No didChange notifications sent on edits
3. **Error Feedback** - Silent failures when language servers missing
4. **Multiple Files** - Only works for the initially opened file
5. **didSave** - Never called when saving files
6. **didClose** - Never called when closing/switching files

## Testing Status

**Unit Tests:** ✅ Basic LSP module tests pass (manager creation, versioning)

**Integration Tests:** ❌ LSP operations test only verifies keybindings exist, not functionality

**Manual Testing:** ❌ Cannot test without language servers installed

## Recommended Fixes (Priority Order)

### High Priority

1. **Install Language Servers** (5 min)
   ```bash
   rustup component add rust-analyzer
   npm install -g typescript-language-server typescript
   pip install python-lsp-server
   ```

2. **Fix Notification Architecture** (2-3 hours)
   - Redesign to use channels for server→manager communication
   - Implement proper notification handling in background task
   - Connect notifications to `handle_notification()` method

3. **Add didChange Notifications** (1 hour)
   - Hook into buffer modification events
   - Send incremental or full document changes
   - Track document versions properly

### Medium Priority

4. **Add didSave Notifications** (30 min)
   - Call `lsp_manager.did_save()` after successful file save
   - In `src/buffer/mod.rs` save methods

5. **Error Handling & User Feedback** (1 hour)
   - Show errors when language server fails to start
   - Display LSP status in status bar
   - Log LSP errors to a debug file

6. **Configuration System** (2 hours)
   - Config file for language server paths and settings
   - Per-language configuration
   - LSP enable/disable per file type

### Low Priority

7. **Multiple File Support** (2-3 hours)
   - Handle didClose when switching files
   - Proper cleanup on file close
   - Track open documents

8. **Additional LSP Features** (ongoing)
   - Code completion
   - Code actions
   - Formatting
   - Rename refactoring

## Quick Test Script

After installing language servers, test with:

```bash
# Create test file with error
cat > /tmp/test.rs << 'EOF'
fn main() {
    let x = 5
}
EOF

# Start ovim in headless mode
cargo run -- /tmp/test.rs --headless &

# Wait for server to start
sleep 2

# Get the port
PORT=$(lsof -i -P -n | grep ovim | grep LISTEN | awk '{print $9}' | cut -d: -f2)

# Check snapshot (should show diagnostics if working)
curl "http://127.0.0.1:$PORT/snapshot" | jq '.buffer'
```

## Conclusion

The LSP implementation is **well-architected** with clean separation of concerns and proper async handling. However, it's **non-functional** due to:

1. **Missing runtime dependencies** (language servers not installed)
2. **Incomplete integration** (notification listener not started)
3. **Missing change notifications** (didChange never sent)
4. **Architectural issue** (notification handling not connected)

**Estimated effort to make functional:** 4-6 hours of focused development

**Recommended approach:**
1. Install language servers (5 min) ✓
2. Fix notification architecture (3 hours)
3. Add didChange integration (1 hour)
4. Test and iterate (1-2 hours)

The foundation is solid - it just needs the plumbing completed.
