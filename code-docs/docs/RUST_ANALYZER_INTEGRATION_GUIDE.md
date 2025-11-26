# Rust-Analyzer Integration Guide for ovim

## Executive Summary

After manual testing of rust-analyzer's LSP protocol, here are the critical insights for ovim's implementation:

## 🔴 Critical Issues Found

### 1. Workspace Indexing Delay (MOST IMPORTANT)

**Problem**: Hover requests return `null` if sent before workspace indexing completes.

**Timeline**:
- `didOpen` notification sent → 0s
- Diagnostics arrive → ~500ms
- **Workspace indexing completes → 10-15 seconds** ⚠️
- Hover requests work → after indexing

**Impact on ovim**:
- Users pressing `K` immediately after opening a file will get "No hover information"
- This appears as a bug, but it's actually rust-analyzer still indexing

**Solution Options**:

1. **Status indicator** (Recommended):
```rust
enum LspStatus {
    NotStarted,
    Initializing,
    Indexing { started_at: Instant },
    Ready,
    Error(String),
}

// Show in status bar:
// "LSP: Indexing... (5s)"
// "LSP: Ready ✓"
```

2. **Delayed availability** (Alternative):
```rust
// Track didOpen timestamp per file
if let Some(opened_at) = file.lsp_opened_at {
    if opened_at.elapsed() < Duration::from_secs(15) {
        show_message("LSP still indexing workspace...");
        return;
    }
}
```

3. **Progressive enhancement**:
```rust
// Try hover anyway, show helpful message on null:
match lsp.hover(position).await {
    Some(hover) => show_hover(hover),
    None => show_message("No hover info (LSP may still be indexing)"),
}
```

### 2. UTF-16 Position Encoding

**Problem**: rust-analyzer uses UTF-16 code units, not UTF-8 bytes or characters.

**Impact**:
- For ASCII: No problem (1 char = 1 UTF-16 unit)
- For emoji/multibyte: Positions will be wrong ⚠️

**Example**:
```rust
// Line: "let 😀 = 5;"
//        0123456789  (byte offsets)
//        0  1 2 3 4  (char offsets)
//        0  12 3 4 5 (UTF-16 offsets) ← LSP uses this

// To get position of '=', ovim must send character: 6 (not 5)
```

**Required Implementation**:
```rust
/// Convert ovim's char offset to LSP UTF-16 offset
fn char_to_lsp_position(line: &str, char_offset: usize) -> usize {
    line.chars()
        .take(char_offset)
        .map(|c| c.len_utf16())
        .sum()
}

/// Convert LSP UTF-16 offset to ovim's char offset
fn lsp_position_to_char(line: &str, utf16_offset: usize) -> usize {
    let mut utf16_count = 0;
    for (char_idx, ch) in line.chars().enumerate() {
        if utf16_count >= utf16_offset {
            return char_idx;
        }
        utf16_count += ch.len_utf16();
    }
    line.chars().count()
}
```

**Where to use**:
- ✅ Before sending hover/definition requests
- ✅ After receiving hover ranges
- ✅ After receiving definition positions
- ✅ For all LSP position conversions

### 3. Protocol Requirements

**Critical details**:
- ✅ Use `\r\n` (CRLF), not `\n` (LF) for headers
- ✅ Format: `Content-Length: {bytes}\r\n\r\n{json}`
- ✅ Shutdown params must be `null`, not `{}`

**Current ovim implementation check**:
```rust
// In lsp/server.rs, verify:
write!(writer, "Content-Length: {}\r\n\r\n", content.len())?; // ✓ Good
// NOT:
write!(writer, "Content-Length: {}\n\n", content.len())?;     // ✗ Bad
```

## ✅ What Works Well

### 1. Initialization Sequence

The current ovim initialization appears correct:
```rust
1. initialize request with rootUri
2. Wait for response
3. Send initialized notification
4. Send didOpen for file
```

### 2. Hover Response Format

Hover responses are well-structured:
```json
{
  "contents": {
    "kind": "markdown",
    "value": "```rust\npub struct Buffer { ... }\n```\n\nDoc comments here"
  },
  "range": {
    "start": {"line": 57, "character": 11},
    "end": {"line": 57, "character": 17}
  }
}
```

**Implementation tip**: Extract `contents.value` and render as markdown in hover window.

### 3. Workspace Detection

rust-analyzer automatically detects workspace from:
- `rootUri` in initialize request
- Presence of `Cargo.toml`
- Dependencies loaded from Cargo metadata

## 🛠️ Implementation Checklist

### Phase 1: Fix Critical Issues

- [ ] **Implement UTF-16 position conversion**
  - [ ] Add `char_to_lsp_position()` function
  - [ ] Add `lsp_position_to_char()` function
  - [ ] Use in all LSP position conversions
  - [ ] Add tests for ASCII and emoji text

- [ ] **Add workspace indexing awareness**
  - [ ] Track `didOpen` timestamp per file
  - [ ] Add `LspStatus` enum
  - [ ] Show "LSP: Indexing..." message
  - [ ] Wait 15s or check status before allowing hover

- [ ] **Verify protocol compliance**
  - [ ] Confirm CRLF line endings in messages
  - [ ] Confirm shutdown uses `params: null`

### Phase 2: User Experience

- [ ] **Status indicator**
  - [ ] Show LSP status in status bar
  - [ ] Show indexing progress if available
  - [ ] Show "Ready" when indexing complete

- [ ] **Error messages**
  - [ ] When hover returns `null`: "No hover info (LSP may still be indexing)"
  - [ ] When LSP not ready: "LSP still starting up..."
  - [ ] When LSP error: Show actual error from server

### Phase 3: Testing

- [ ] **Manual test with rust-analyzer**
  - [ ] Open large file (like buffer/mod.rs)
  - [ ] Press `K` immediately → should show "indexing"
  - [ ] Wait 15s, press `K` → should show hover
  - [ ] Test with emoji in code → positions correct

- [ ] **Automated tests**
  - [ ] UTF-16 conversion unit tests
  - [ ] Mock LSP server with timing tests
  - [ ] Integration test with actual rust-analyzer

## 📊 Timing Reference

| Event | Time | Action |
|-------|------|--------|
| Send `initialize` | 0s | Start LSP |
| Receive `initialize` response | +0.5s | Server capabilities received |
| Send `initialized` | +0.5s | Notification |
| Send `didOpen` | +1s | Open file |
| Receive `publishDiagnostics` | +1.5s | Diagnostics available |
| **Workspace indexing done** | **+15s** | **Hover now works** ⚠️ |
| User presses `K` | +15s | Show hover |
| Receive hover response | +15.2s | Display hover window |

## 🔍 Debugging Tips

### When hover returns `null`:

1. **Check timing**: Has 15s passed since `didOpen`?
2. **Check workspace**: Is file in `Cargo.toml` workspace?
3. **Check position**: Is UTF-16 conversion correct?
4. **Check range**: Is position within line bounds?
5. **Check logs**: Any errors from rust-analyzer?

### Test positions:

```rust
// File: src/buffer/mod.rs
// Line 57 (0-indexed): "pub struct Buffer {"

Position { line: 57, character: 15 }  // ✓ On "Buffer"
Position { line: 57, character: 11 }  // ✓ On "B" of Buffer
Position { line: 57, character: 100 } // ✗ Out of bounds (clamped)
```

## 📝 Sample Implementation

### Hover Request with Timing Check

```rust
impl Editor {
    pub async fn hover_at_cursor(&mut self) -> Result<Option<Hover>> {
        let (file_uri, line, col) = self.get_cursor_position();

        // Check if LSP is ready
        if let Some(lsp) = &self.lsp_manager {
            // Check workspace indexing status
            if let Some(opened_at) = self.file_opened_timestamps.get(&file_uri) {
                let elapsed = opened_at.elapsed();
                if elapsed < Duration::from_secs(15) {
                    self.show_message(format!(
                        "LSP still indexing workspace... ({:.0}s)",
                        15 - elapsed.as_secs()
                    ));
                    return Ok(None);
                }
            }

            // Convert position to UTF-16
            let line_text = self.buffer.line(line)?;
            let utf16_col = char_to_lsp_position(&line_text, col);

            let lsp_position = LspPosition {
                line: line as u32,
                character: utf16_col as u32
            };

            // Send hover request
            match lsp.hover(&file_uri, lsp_position).await {
                Ok(Some(hover)) => Ok(Some(hover)),
                Ok(None) => {
                    self.show_message("No hover information available");
                    Ok(None)
                }
                Err(e) => {
                    self.show_error(format!("LSP error: {}", e));
                    Err(e)
                }
            }
        } else {
            self.show_message("LSP not initialized");
            Ok(None)
        }
    }
}
```

## 📚 References

- Full test report: `/workspace/docs/rust_analyzer_manual_test_report.md`
- Working messages: `/workspace/docs/rust_analyzer_working_messages.json`
- Test scripts: `/workspace/test_ra_workspace.sh`
- Test output: `/tmp/ra_workspace_output.log`

## Summary

**The #1 issue to fix**: Workspace indexing delay. Users will report "hover doesn't work" when it's actually just not ready yet. Add status indication and timing checks.

**The #2 issue to fix**: UTF-16 position encoding. This will cause subtle bugs with non-ASCII text.

Both are straightforward to implement and will dramatically improve the LSP experience.
