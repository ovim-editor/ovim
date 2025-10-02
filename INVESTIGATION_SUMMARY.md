# ovim Investigation Summary

## Syntax Highlighting Issue - RESOLVED ✅

### Problem
Syntax highlighting appeared to be implemented but wasn't working when opening files.

### Root Cause
The tree-sitter query files contained **invalid node types** that don't exist in the respective grammars:

1. **`src/syntax/queries/rust.scm`** - Line 21: `"mut"` is not a valid keyword node
2. **`src/syntax/queries/javascript.scm`** - Lines 44, 51: Invalid node types `(function ...)` and `(type_identifier)`

### Why It Was Hidden
The `Buffer::enable_syntax_highlighting()` method silently swallows errors from `SyntaxHighlighter::new()`:

```rust
if let Ok(mut highlighter) = SyntaxHighlighter::new(lang) {
    // Only sets self.syntax if successful
}
```

This caused query parsing errors to be completely invisible - syntax highlighting just didn't activate.

### Solution
Fixed the query files:
- **Rust**: Removed `"mut"` from keyword list, added `(mutable_specifier) @keyword`
- **JavaScript**: Removed `(function ...)` and TypeScript-specific `(type_identifier)`
- **Python**: Already correct ✅

### Verification
All languages now work correctly:
```bash
$ cargo test --test all_languages_test
test test_javascript_highlighter ... ok
test test_python_highlighter ... ok
test test_rust_highlighter ... ok
```

Example output:
- Rust: `[(0..2, Keyword), (3..7, Function), ...]` for `fn main()`
- JavaScript: `[(0..8, Keyword), (9..13, Function)]` for `function test()`
- Python: `[(0..3, Keyword), (4..8, Function)]` for `def test()`

---

## LSP Integration Plan

### Architecture Overview

LSP support will be added through a modular design:

```
Editor → LspManager → LanguageServer → rust-analyzer/pyright/tsserver
                     ↓
               Diagnostics Store
               Request Tracking
               Configuration
```

### Core Components

1. **LspManager** - Coordinates all LSP interactions
   - Manages multiple language servers
   - Tracks diagnostics
   - Handles request/response matching
   - Provides high-level API to Editor

2. **LanguageServer** - Manages individual server process
   - Spawns and monitors server process
   - Handles stdio communication
   - Implements JSON-RPC protocol
   - Tracks document versions

3. **Protocol Layer** - JSON-RPC message handling
   - Message serialization/deserialization
   - Content-Length header framing
   - Request ID generation
   - Error handling

4. **Integration Points**
   - Buffer: Track changes, send didChange notifications
   - Editor: Coordinate LSP operations, store state
   - UI: Display diagnostics, completion menu, hover popups
   - Input: Add keybindings (gd, K, etc.)

### Implementation Phases

**Phase 1: Foundation** (Week 1)
- Add `lsp-types` dependency
- Create module structure
- Implement JSON-RPC protocol
- Process management

**Phase 2: Document Synchronization** (Week 2)
- Initialize handshake
- didOpen/didChange/didSave/didClose
- Document version tracking
- Integration with Buffer

**Phase 3: Diagnostics** (Week 3)
- Receive publishDiagnostics
- Store and display diagnostics
- UI rendering (underlines, status bar)

**Phase 4: Navigation** (Week 4)
- Go to definition (gd)
- Hover information (K)
- Jump to location

**Phase 5: Completion** (Week 5)
- Completion requests
- Completion menu UI
- Trigger characters
- Item selection

**Phase 6: Advanced Features** (Week 6+)
- Code actions
- Formatting
- Rename
- Find references

### Key Design Decisions

1. **Async I/O with Tokio** - Already using tokio for REST API, reuse for LSP
2. **One Server Per Language** - Start with simple 1:1 mapping
3. **Incremental Sync** - Send only changed text, not full document
4. **Graceful Degradation** - Editor works even if LSP fails
5. **Configuration File** - `LSP.toml` for server settings

### Testing Strategy

- **Unit tests**: Protocol parsing, change tracking
- **Integration tests**: Mock language servers
- **E2E tests**: Real servers (rust-analyzer, pyright)
- **REST API tests**: Verify in headless mode

### Example Usage

```rust
// Editor with LSP enabled
let config = LspConfig::from_file("LSP.toml")?;
editor.enable_lsp(config)?;

// Opening a file automatically starts LSP
editor.open_file("src/main.rs")?;
// → starts rust-analyzer
// → sends didOpen notification
// → receives diagnostics

// User presses 'gd' on a symbol
editor.goto_definition()?;
// → sends textDocument/definition request
// → jumps to location

// User presses 'K' on a symbol
editor.show_hover()?;
// → sends textDocument/hover request
// → displays documentation in popup
```

### Configuration Example

```toml
# LSP.toml
[servers.rust]
command = "rust-analyzer"
args = []

[servers.python]
command = "pyright-langserver"
args = ["--stdio"]

[servers.javascript]
command = "typescript-language-server"
args = ["--stdio"]

[servers.typescript]
command = "typescript-language-server"
args = ["--stdio"]
```

### Performance Optimizations

- **Debouncing**: Don't send didChange on every keystroke
- **Caching**: Cache hover/definition results
- **Background tasks**: Process LSP messages asynchronously
- **Incremental changes**: Use LSP's incremental sync

### Error Handling

- Server startup failure → Warning, continue without LSP
- Server crash → Auto-restart with backoff
- Request timeout → Cancel, show error
- Invalid response → Log, gracefully degrade
- Missing capability → Disable feature

---

## Next Steps

### Immediate Actions

1. **Syntax Highlighting**
   - ✅ Fixed all query files
   - ✅ Verified all languages work
   - Consider adding better error messages if queries fail in future

2. **LSP Integration**
   - Start with Phase 1: Add dependencies and create module structure
   - Implement basic JSON-RPC protocol handling
   - Create LanguageServer process management
   - Test with rust-analyzer

### Long-term Improvements

1. **Syntax Highlighting**
   - Add more languages (C/C++, Go, Java, etc.)
   - Improve query files for better highlighting
   - Add semantic highlighting from LSP

2. **LSP Features**
   - Complete all phases (diagnostics → navigation → completion → advanced)
   - Add configuration UI
   - Support multiple servers per language
   - Implement all LSP features (rename, references, etc.)

3. **Editor Enhancements**
   - Plugin system
   - Customizable themes
   - Better error reporting
   - Performance profiling

---

## Files Modified

### Syntax Highlighting Fix
- `src/syntax/queries/rust.scm` - Fixed `"mut"` keyword issue
- `src/syntax/queries/javascript.scm` - Removed invalid nodes
- `tests/syntax_test.rs` - Added tests
- `tests/highlighter_test.rs` - Added tests
- `tests/all_languages_test.rs` - Added tests
- `tests/language_detection_test.rs` - Added tests

### Documentation
- `LSP_DESIGN.md` - Complete LSP architecture design
- `INVESTIGATION_SUMMARY.md` - This file

### No Changes Needed
- `src/syntax/queries/python.scm` - Already correct ✅
