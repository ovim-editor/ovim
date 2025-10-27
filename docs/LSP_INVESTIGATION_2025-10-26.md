# LSP Investigation - TypeScript & Rust Not Working
**Date**: 2025-10-26
**Status**: Root cause still under investigation

## Problem Statement

Both TypeScript and Rust LSP servers start successfully and report full capabilities, but hover (K) and goto definition (gd) return `null` for all positions.

## Test Results

### TypeScript LSP
- **Language Server**: typescript-language-server ✅ Installed via npm
- **Server Status**: Ready with full capabilities
- **Capabilities**: Hover, Definition, Completion, etc. all advertised
- **Test File**: `fixtures/typescript_test.ts`
- **Hover Result**: `null` ❌
- **Session**: Started in headless mode successfully

### Rust LSP
- **Language Server**: rust-analyzer ✅ Installed via cargo
- **Server Status**: Ready with full capabilities
- **Capabilities**: Hover, Definition, Completion, etc. all advertised
- **Test File**: `src/main.rs`
- **Hover Result**: `null` ❌
- **Session**: Started in headless mode successfully

## Previous Fixes Applied

### 1. Duplicate `didOpen` Fix ✅
**Status**: Already implemented

The duplicate `didOpen` issue documented in `notes/LSP_UNRESPONSIVENESS_ANALYSIS.md` has been fixed:
- `lsp_init` modules (rust.rs, javascript.rs, python.rs) no longer call `lsp_manager.did_open()` directly
- Comments explicitly state: "Don't send didOpen here - it will be handled by `ensure_document_opened`"
- State tracking via `did_open_sent` flag is working correctly in `Editor::ensure_document_opened()`

**Files Updated**:
- `src/lsp_init/rust.rs` (lines 42-44)
- `src/lsp_init/javascript.rs` (lines 28-30)
- `src/editor/mod.rs` (lines 2287-2335)

## Current Investigation

### Hypothesis
The `didOpen` notification may not be sent until the first hover/gd attempt, but even then the LSP returns `null`. This suggests:

1. **Timing Issue**: LSP server may need time to index/process the file after `didOpen`
2. **Workspace Configuration**: LSP may need specific initialization options (as noted in `LSP_HOVER_INVESTIGATION.md`)
3. **URI/Path Issues**: File paths or URIs may not match what LSP expects
4. **Missing didOpen**: Despite the fix, `didOpen` may not actually be sent

### Code Flow

```
User presses 'K' for hover
  ↓
Editor::hover_impl() [src/editor/mod.rs:3559]
  ↓
Editor::ensure_document_opened() [src/editor/mod.rs:2257]
  ↓
Checks did_open_sent flag [src/editor/mod.rs:2294]
  ↓
If false: LspManager::did_open() [src/lsp/mod.rs:342]
  ↓
Marks did_open_sent = true [src/editor/mod.rs:2325]
  ↓
LspManager::hover() [src/lsp/mod.rs:1129]
  ↓
Returns null
```

### Next Steps for Debugging

1. **Enable LSP Protocol Logging**
   - Check if `didOpen` is actually being sent to the language server
   - Verify the request/response format for hover requests
   - Check for any error responses from the LSP server

2. **Test Initialization Parameters**
   - Compare ovim's `initialize` request with what Neovim sends
   - Check if `initializationOptions` are needed
   - Verify workspace folders configuration

3. **Test with Simple File**
   - Create a minimal TypeScript file outside any workspace
   - Test if hover works there (to rule out workspace issues)

4. **Check Timing**
   - Add delay between `didOpen` and first hover request
   - See if LSP needs time to index the file

## Comparison with Neovim

According to previous notes (`LSP_HOVER_INVESTIGATION.md`), hover works in Neovim on the same files. This suggests:
- The LSP servers themselves are functioning correctly
- ovim's LSP protocol implementation is mostly correct (no duplicate didOpen errors)
- The issue is likely in initialization, configuration, or timing

## Files Involved

### LSP Initialization
- `src/lsp_init/mod.rs` - Routes to language-specific init
- `src/lsp_init/rust.rs` - Rust LSP setup
- `src/lsp_init/javascript.rs` - TypeScript/JavaScript LSP setup

### LSP Management
- `src/lsp/mod.rs` - LspManager (coordinator)
- `src/lsp/server.rs` - LanguageServer (individual server)
- `src/lsp/protocol.rs` - JSON-RPC protocol handling

### Editor Integration
- `src/editor/mod.rs` - Editor with LSP methods
  - `ensure_document_opened()` (line 2257)
  - `hover_impl()` (line ~3559)
  - `goto_definition_impl()` (line ~2552)

### State Tracking
- `src/editor/lsp_state.rs` - LSP state management
  - `did_open_sent` flag
  - `last_synced_content`
  - `document_sync` HashMap

## Testing Commands

```bash
# Start TypeScript session with debug logging
OVIM_LSP_DEBUG=1 ./target/release/ovim fixtures/typescript_test.ts --headless --session ts_test

# Test hover
./ovim-ctl send ts_test "6lK"  # Move to someNumber and hover
./ovim-ctl snapshot ts_test | jq '.hover_content'

# Check LSP status
./ovim-ctl lsp ts_test

# Check logs (if logging is working)
grep -i "hover\|didopen" ~/.cache/ovim/lsp.log
```

## Known Issues

1. **OVIM_LSP_DEBUG not producing logs** - LSP debug logging doesn't appear to be working in headless mode
2. **No error messages** - LSP returns `null` without any error indication
3. **Missing protocol logs** - Can't see actual JSON-RPC messages being sent/received

## Recommendations

1. **Add comprehensive LSP protocol logging** to see actual messages
2. **Test with wireshark/tcpdump** to capture LSP communication
3. **Compare initialize parameters** with working Neovim setup
4. **Add timing delay** after didOpen before first LSP request
5. **Test simple standalone file** without workspace to isolate the issue

---

**Investigated by**: Claude Code
**Status**: In progress - LSP servers start but don't respond to hover/gd
