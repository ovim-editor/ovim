# LSP Fix Investigation Summary
**Date**: 2025-10-26
**Status**: Partial fixes implemented, testing reveals additional issues

## Problems Identified

### 1. TypeScript Language Server Not Installed ✅ FIXED
**Issue**: `typescript-language-server` was not installed on the system
**Fix**: Installed via `npm install -g --prefix ~/.local typescript-language-server typescript`
**Location**: `~/.local/bin/typescript-language-server`

### 2. Wrong InitializationOptions for All Languages ✅ FIXED
**Issue**: All language servers received rust-analyzer-specific configuration
**Location**: `src/lsp/server.rs:618-714`

**Before**:
```rust
// Hardcoded rust-analyzer options sent to ALL servers
let initialization_options = Some(json!({
    "checkOnSave": { "command": "clippy" },  // TypeScript doesn't understand!
    "inlayHints": { ... }  // Wrong format for TypeScript
}));
```

**After**:
```rust
// Language-specific initialization
let initialization_options = match self.inner.language.as_str() {
    "rust" => Some(json!({ /* rust-analyzer config */ })),
    "javascript" | "typescript" => Some(json!({
        "preferences": {
            "includeInlayParameterNameHints": "all",
            "includeInlayFunctionParameterTypeHints": true,
            "includeInlayVariableTypeHints": true,
            ...
        }
    })),
    "python" => Some(json!({ /* pyright config */ })),
    _ => None
};
```

###  3. Rust LSP Timing Issue - DOCUMENTED
**Issue**: rust-analyzer needs 10-15 seconds to index workspace after `didOpen`
**Evidence**: Manual testing shows hover returns `null` for first 15 seconds, then works perfectly
**Impact**: Users pressing `K` immediately after opening get "No information" which appears as a bug
**Solution Needed**: Add indexing status tracking and show "LSP: Indexing..." message

## Manual Testing Results

### TypeScript Language Server
**Test**: Manual JSON-RPC message exchange with `typescript-language-server --stdio`
**Findings**:
- ✅ Hover works immediately after didOpen (no delay needed)
- ✅ didChange processed synchronously
- ✅ Returns rich markdown hover content: `const greeting: string`
- ✅ Returns `null` for whitespace/keywords (legitimate behavior)

**Example Working Messages**:
```json
// Initialize
{
  "id": 1,
  "method": "initialize",
  "params": {
    "capabilities": {...},
    "initializationOptions": {
      "preferences": {
        "includeInlayParameterNameHints": "all"
      }
    }
  }
}

// Response: {"result": {"capabilities": {"hoverProvider": true, ...}}}

// didOpen
{
  "method": "textDocument/didOpen",
  "params": {
    "textDocument": {
      "uri": "file:///tmp/test.ts",
      "languageId": "typescript",
      "version": 1,
      "text": "const greeting: string = \"Hello\";"
    }
  }
}

// Hover (immediately after didOpen)
{
  "id": 2,
  "method": "textDocument/hover",
  "params": {
    "textDocument": {"uri": "file:///tmp/test.ts"},
    "position": {"line": 0, "character": 6}
  }
}

// Response: {"result": {"contents": "const greeting: string"}}
```

### Rust Analyzer (rust-analyzer)
**Test**: Manual JSON-RPC with real ovim source file
**Findings**:
- ⏱️ **Critical**: Hover returns `null` for 10-15 seconds after didOpen
- ✅ After indexing completes, hover works perfectly
- ✅ Returns comprehensive hover data with full signatures

**Timeline**:
```
T+0.0s:  didOpen sent
T+0.5s:  Diagnostics received (server is responsive)
T+0-15s: Hover → null (workspace indexing in progress)
T+15s+:  Hover → full data including docs and type info
```

## Current Status After Fixes

### What Was Fixed
1. ✅ TypeScript language server installed
2. ✅ Language-specific initializationOptions implemented
3. ✅ Added comprehensive initialization logging
4. ✅ Rebuild completed successfully

### What Still Doesn't Work
**TypeScript hover returns `null`** even after fixes

Possible remaining issues:
1. **didOpen may not be sent** - Need to verify with logging
2. **Language detection** - May be sending "javascript" instead of "typescript"
3. **Document URI** - May have path/URI conversion issues
4. **Missing capabilities** - TypeScript LSP may need additional client capabilities

## Fixes Applied

### File: `src/lsp/server.rs`
**Lines 618-714**: Made `initializationOptions` language-specific

**Added**:
- Rust: Enhanced rust-analyzer config with `procMacro` and `cargo.buildScripts`
- TypeScript/JavaScript: Proper TypeScript LSP preferences
- Python: Basic pyright configuration
- Other languages: `None` (no custom options)

**Lines 735-741**: Added initialization logging
```rust
crate::lsp_info!(
    &self.log_prefix(),
    "LSP Initialize | Language: {} | Root: {} | InitOptions: {}",
    self.inner.language,
    root_uri,
    initialization_options...
);
```

### Logging Already Present
- `src/lsp/mod.rs:349-356`: didOpen logging
- `src/lsp/mod.rs:801-823`: Hover logging with language and position
- `src/lsp/server.rs:333-339`: Request/response logging

## Documentation Created

### From Subagent Testing
1. **TypeScript LSP Manual Testing**:
   - `/workspace/docs/typescript_lsp_manual_test_report.md`
   - `/workspace/docs/typescript_hover_example.json`
   - `/workspace/docs/simple_typescript_lsp_test.sh`

2. **Rust Analyzer Manual Testing**:
   - `/workspace/docs/rust_analyzer_manual_test_report.md`
   - `/workspace/docs/rust_analyzer_working_messages.json`
   - `/workspace/docs/RUST_ANALYZER_INTEGRATION_GUIDE.md`

3. **LSP Protocol Comparison**:
   - Comprehensive analysis comparing LSP spec vs our implementation
   - Identified the critical initializationOptions bug

## Next Steps for Debugging

### Immediate (TypeScript)
1. **Enable full protocol logging** to see actual JSON-RPC messages
2. **Verify didOpen is sent** - Check logs for didOpen notification
3. **Check language ID** - Verify we're sending "typescript" not "javascript" for `.ts` files
4. **Test with minimal file** - Create `/tmp/test.ts` and test manually
5. **Compare with working manual test** - Use exact same messages that worked in manual testing

### Recommended Fixes
1. **Add protocol-level logging** - Log all JSON-RPC messages to/from LSP
2. **Verify language detection** - Check `LanguageRegistry::get_lsp_language_id()` for `.ts` files
3. **Add indexing status** for Rust:
   ```rust
   pub enum LspStatus {
       Indexing { started_at: Instant },
       Ready,
       ...
   }
   ```
4. **Test with working script** - Use `/workspace/docs/simple_typescript_lsp_test.sh` as reference

## Test Commands

```bash
# Test TypeScript (after fixes)
export PATH="$HOME/.local/bin:$PATH"
./target/release/ovim fixtures/typescript_test.ts --headless --session ts_test

# Trigger hover
./ovim-ctl send ts_test "6lK"

# Check result
./ovim-ctl snapshot ts_test | jq '.hover_content'

# Check LSP status
./ovim-ctl lsp ts_test

# Kill session
./ovim-ctl kill ts_test
```

## Summary

### Progress Made
- ✅ Identified root cause: wrong initializationOptions for all languages
- ✅ Fixed TypeScript LSP not starting (installed language server)
- ✅ Implemented language-specific initialization
- ✅ Added comprehensive logging
- ✅ Documented Rust analyzer timing behavior (10-15s indexing)
- ✅ Manual testing proves LSP servers work correctly when configured properly

### Still Broken
- ❌ TypeScript hover still returns `null` despite fixes
- ❌ Need to verify didOpen is actually being sent
- ❌ Need protocol-level logging to debug further

### Confidence Level
- **initializationOptions fix**: 100% correct - This was definitely broken
- **TypeScript LSP working**: 50% - Fix is correct but something else may be wrong
- **Rust timing issue**: 100% confirmed - Need UI status updates

---

**Investigation Complete**: Core issues identified and partially fixed. Hover still not working suggests additional debugging needed at protocol level.
