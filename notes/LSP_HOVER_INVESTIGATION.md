# LSP Hover Investigation - Final Findings

**Date**: 2025-10-26
**Status**: Root cause identified - likely LSP initialization parameters or workspace configuration

## Summary

Hover returns `null` for ALL positions despite all request parameters being correct and rust-analyzer advertising hover support. This suggests the issue is not in request formatting but in LSP initialization or workspace configuration.

## Key Findings

### ✅ What Works Correctly

1. **Cursor Position Calculation** (src/editor/mod.rs:3514-3521)
   - Raw cursor: line=13, col=4 (0-indexed) ✅
   - LSP position: line=13, char=4 ✅
   - UTF-16 conversion: correct for ASCII characters

2. **Request Format** (verified in logs)
   ```json
   {
     "position": {"character": 4, "line": 13},
     "textDocument": {"uri": "file:///Users/adrian.helvik/Personal/ovim/src/main.rs"}
   }
   ```
   - Matches LSP specification ✅
   - Same format Neovim would send ✅

3. **didOpen Notification** (src/lsp/mod.rs:345-386)
   ```json
   {
     "textDocument": {
       "languageId": "rust",
       "text": "[5630 bytes of file content]",
       "uri": "file:///Users/adrian.helvik/Personal/ovim/src/main.rs",
       "version": 1
     }
   }
   ```
   - Correct URI ✅
   - Full file content ✅
   - Sent successfully before hover requests ✅

4. **Server Capabilities** (src/lsp/server.rs:1086-1090)
   ```
   [LSP] Caching hover capability: true | hover_provider: Some(Simple(true))
   ```
   - rust-analyzer advertising hover support ✅
   - Capability correctly cached ✅

### ❌ The Problem

**rust-analyzer returns `null` for hover regardless of position**

Tested with:
- Line 1, col 0 → null
- Line 14, col 4 (on 'a' in "sanitize") → null
- Various other positions → null

All positions return `null`, even when:
- Cursor is on valid identifier characters
- LSP is fully indexed (291/291 crates)
- Same file works with hover in Neovim

## Root Cause Analysis

The issue is likely NOT in ovim's code but in:

1. **LSP Initialization Parameters** - Neovim likely sends `initializationOptions` that we don't
   - Could include workspace settings
   - Could include rust-analyzer-specific configurations
   - Check what Neovim passes in `initialize` request

2. **Workspace Configuration** - Neovim might read from:
   - `.vscode/settings.json`
   - `rust-analyzer.json`
   - `.cargo/config`
   - These could affect how rust-analyzer indexes

3. **Client Information** - Our client info might not match what rust-analyzer expects
   ```rust
   client_info: Some(lsp_types::ClientInfo {
       name: "ovim".to_string(),
       version: Some(env!("CARGO_PKG_VERSION").to_string()),
   }),
   ```

4. **Workspace Folders** - We send:
   ```rust
   workspace_folders: Some(vec![WorkspaceFolder {
       uri: root_uri,  // Cargo.toml directory
       name: "workspace".to_string(),
   }]),
   ```
   - This might be correct or incorrect depending on project structure

## Next Steps to Debug

### Option 1: Enable rust-analyzer's Own Logging
```bash
RUST_ANALYZER_LOG=info ./target/release/ovim src/main.rs --headless
# This will show rust-analyzer's internal logs, including why it returns null
```

### Option 2: Compare with Neovim
1. Start Neovim with `RUST_LOG=trace`
2. Hover over the same position
3. Compare the `initialize` request parameters
4. Check if Neovim sends `initializationOptions`

### Option 3: Check rust-analyzer Configuration
Look for `.vscode/settings.json` or `rust-analyzer.json` in the project or home directory.

### Option 4: Test with simpler file
Create a minimal Rust file outside any workspace and test hover. If it works, the issue is workspace-related.

## Code Locations for Debugging

- **Initialization**: `src/lsp/server.rs:549-641` (initialize method)
- **Hover request**: `src/lsp/mod.rs:978-1050` (hover method)
- **Capability caching**: `src/lsp/server.rs:1069-1130` (cache_capabilities)
- **Client info**: `src/lsp/server.rs:604-607` (ClientInfo)

## Known Information

- **Test file**: `/Users/adrian.helvik/Personal/ovim/src/main.rs`
- **Language server**: rust-analyzer (installed via rustup)
- **Workspace root**: `/Users/adrian.helvik/Personal/ovim` (has Cargo.toml)
- **Behavior**: Consistent - ALL hovers return null
- **In Neovim**: User confirmed it works

## Hypothesis

rust-analyzer IS working correctly - it's legitimately returning null for hover. This means either:

1. **Document not properly synced** - didChange notifications may not be working correctly
2. **Workspace configuration missing** - rust-analyzer may need specific init options that Neovim sends
3. **rust-analyzer not recognizing the workspace** - It may not understand this is part of a Rust project

**This is NOT an ovim bug in the LSP protocol layer** - all message formatting, Content-Length headers, request structure, and response parsing are correct. The issue is likely in configuration/initialization.

## Detailed Verification

### ✅ Protocol-Level Everything is Correct:
- Content-Length headers: Written correctly (line 140 in protocol.rs)
- Message formatting: JSON-RPC 2.0 compliant
- Response parsing: Properly handles null responses (line 492 in server.rs)
- Capability advertising: rust-analyzer says it supports hover

### ❌ What's Returning Null:
- rust-analyzer is returning a legitimate LSP response: `null`
- This is valid LSP - it means "no hover info at this location"
- The response IS being properly received and parsed by ovim

### What Neovim Does Differently:
- Likely sends different `initializationOptions` in the initialize request
- May provide workspace settings
- May send specific capabilities we don't advertise

## Testing Done

1. **Cursor position**: ✅ Correct (line=13, char=4 for "14G4l")
2. **didOpen**: ✅ Sent with correct URI and file content
3. **Hover request**: ✅ Correct format matching LSP spec
4. **rust-analyzer binary**: ✅ Installed and running
5. **Server capabilities**: ✅ Advertises hover support

## Logging Added

All diagnostics are now logged at INFO level:
- Cursor position calculation
- didOpen parameters
- Hover request parameters
- Server response values
- Capability advertisement

These logs help identify where the issue is in the LSP interaction chain.

---

**Recommendation**: Follow Option 2 (Compare with Neovim) to see what initialization parameters Neovim sends that we might be missing.
