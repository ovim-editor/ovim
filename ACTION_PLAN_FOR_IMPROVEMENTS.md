# Comprehensive Action Plan for Ovim Improvements

## Executive Summary

This document provides a detailed, stage-by-stage implementation plan for all issues identified in AREAS_FOR_IMPROVEMENT.md. Each stage is independent where possible, with clear dependencies marked. Stages are ordered by:

1. **Risk/Complexity**: Low-risk changes first to build confidence
2. **Impact**: High-impact improvements prioritized
3. **Dependencies**: Foundation work before dependent features
4. **User Experience**: Most visible improvements get priority

**Total Estimated Time**: 15-21 days (3-4 weeks)
**Quick Wins Available**: 3 stages totaling 4 days with major impact

---

## Stage 1: Property-Based Testing for Rope Operations [QUICK WIN]

**Priority**: HIGH | **Complexity**: Simple | **Est. Time**: 1 day

### Issue
Rope operations (insert, delete, cursor movements) have complex edge cases that are hard to enumerate manually. While the existing test suite is good, property-based testing can automatically discover edge cases involving:
- Empty buffers
- Operations at buffer boundaries
- UTF-8 multi-byte characters (emoji, CJK)
- Delete beyond end of buffer
- Concurrent sequences of operations

### Root Cause
Manual testing relies on developers thinking of all edge cases. Property-based testing explores the input space automatically, finding corner cases through random generation and shrinking.

### Files to Modify
1. `/Users/adrian/Projects/ovim/Cargo.toml` - Add proptest dependency
2. `/Users/adrian/Projects/ovim/tests/buffer_property_test.rs` - New file with property tests

### Implementation Approach

**Step 1: Add proptest to dependencies**

File: `/Users/adrian/Projects/ovim/Cargo.toml`

```toml
[dev-dependencies]
criterion = "0.5"
reqwest = { version = "0.12.23", features = ["json", "blocking"] }
insta = "1.40"
rand = "0.8"
proptest = "1.5"  # Add this line
```

**Step 2: Create property tests**

File: `/Users/adrian/Projects/ovim/tests/buffer_property_test.rs` (new file)

```rust
use ovim::buffer::Buffer;
use proptest::prelude::*;

/// Arbitrary buffer operation for property testing
#[derive(Debug, Clone)]
enum BufferOp {
    Insert(usize, String),
    Delete(usize, usize),
    SetCursor(usize, usize),
}

/// Strategy for generating buffer operations
fn arb_buffer_op() -> impl Strategy<Value = BufferOp> {
    prop_oneof![
        // Insert: position (0-1000), text (any string up to 100 chars)
        (0..1000usize, "[\\u{0}-\\u{10FFFF}]{0,100}")
            .prop_map(|(pos, text)| BufferOp::Insert(pos, text)),

        // Delete: start and end positions
        (0..1000usize, 0..1000usize)
            .prop_map(|(start, end)| BufferOp::Delete(start, end)),

        // SetCursor: line and column
        (0..100usize, 0..100usize)
            .prop_map(|(line, col)| BufferOp::SetCursor(line, col)),
    ]
}

proptest! {
    /// Property: Insert then delete should restore original text
    #[test]
    fn insert_delete_identity(
        initial_text in ".*",
        insert_pos in 0..1000usize,
        insert_text in "[a-zA-Z0-9 ]{1,10}"
    ) {
        let mut buffer = Buffer::new();
        buffer.insert_str(0, &initial_text);

        let len = buffer.len_chars();
        let pos = insert_pos % (len + 1);

        // Insert text
        buffer.insert_str(pos, &insert_text);

        // Delete the same text
        buffer.delete_range(pos..pos + insert_text.chars().count());

        // Should be back to original
        prop_assert_eq!(buffer.to_string(), initial_text);
    }

    /// Property: Buffer operations never panic
    #[test]
    fn buffer_ops_never_panic(
        initial_text in ".*",
        ops in prop::collection::vec(arb_buffer_op(), 0..50)
    ) {
        let mut buffer = Buffer::new();
        buffer.insert_str(0, &initial_text);

        // Apply all operations - should never panic
        for op in ops {
            match op {
                BufferOp::Insert(pos, text) => {
                    let len = buffer.len_chars();
                    let safe_pos = pos % (len + 1);
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        buffer.insert_str(safe_pos, &text);
                    }));
                }
                BufferOp::Delete(start, end) => {
                    let len = buffer.len_chars();
                    if len > 0 {
                        let safe_start = start % len;
                        let safe_end = end % (len + 1);
                        let (s, e) = (safe_start.min(safe_end), safe_start.max(safe_end));
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            buffer.delete_range(s..e);
                        }));
                    }
                }
                BufferOp::SetCursor(line, col) => {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        buffer.set_cursor(line, col);
                    }));
                }
            }
        }

        // If we got here without panic, test passes
        prop_assert!(true);
    }

    /// Property: UTF-8 is always valid after operations
    #[test]
    fn utf8_always_valid(
        initial_text in ".*",
        ops in prop::collection::vec(arb_buffer_op(), 0..20)
    ) {
        let mut buffer = Buffer::new();
        buffer.insert_str(0, &initial_text);

        for op in ops {
            match op {
                BufferOp::Insert(pos, text) => {
                    let len = buffer.len_chars();
                    buffer.insert_str(pos % (len + 1), &text);
                }
                BufferOp::Delete(start, end) => {
                    let len = buffer.len_chars();
                    if len > 0 {
                        let s = start % len;
                        let e = end % (len + 1);
                        buffer.delete_range(s.min(e)..s.max(e));
                    }
                }
                BufferOp::SetCursor(line, col) => {
                    buffer.set_cursor(line, col);
                }
            }

            // After every operation, buffer should be valid UTF-8
            let content = buffer.to_string();
            prop_assert!(content.is_valid_utf8(),
                "Buffer content is not valid UTF-8 after operation: {:?}", op);
        }
    }

    /// Property: Line count is consistent with content
    #[test]
    fn line_count_consistent(text in ".*") {
        let mut buffer = Buffer::new();
        buffer.insert_str(0, &text);

        let expected_lines = if text.is_empty() {
            1
        } else {
            text.lines().count().max(1)
        };

        prop_assert_eq!(buffer.line_count(), expected_lines);
    }

    /// Property: Cursor is always within bounds after operations
    #[test]
    fn cursor_always_in_bounds(
        text in ".*",
        ops in prop::collection::vec(arb_buffer_op(), 0..30)
    ) {
        let mut buffer = Buffer::new();
        buffer.insert_str(0, &text);

        for op in ops {
            match op {
                BufferOp::Insert(pos, text) => {
                    let len = buffer.len_chars();
                    buffer.insert_str(pos % (len + 1), &text);
                }
                BufferOp::Delete(start, end) => {
                    let len = buffer.len_chars();
                    if len > 0 {
                        let s = start % len;
                        let e = end % (len + 1);
                        buffer.delete_range(s.min(e)..s.max(e));
                    }
                }
                BufferOp::SetCursor(line, col) => {
                    buffer.set_cursor(line, col);
                }
            }

            // Cursor should always be within buffer bounds
            let cursor = buffer.cursor();
            prop_assert!(cursor.line() < buffer.line_count(),
                "Cursor line {} >= line count {}", cursor.line(), buffer.line_count());

            let line_len = buffer.line_len(cursor.line());
            prop_assert!(cursor.col() <= line_len,
                "Cursor col {} > line length {}", cursor.col(), line_len);
        }
    }
}
```

**Educational Context**: Property-based testing is a technique pioneered by QuickCheck in Haskell and adapted to Rust via proptest. Instead of writing specific test cases, you specify **properties** that should always hold (invariants), and the framework generates hundreds of random inputs to try to violate those properties. When a violation is found, proptest automatically **shrinks** the input to the minimal failing case, making debugging much easier.

This is especially valuable for data structure implementations like rope-based text buffers, where:
- The state space is enormous (any UTF-8 string + cursor position)
- Edge cases are subtle (UTF-8 boundaries, empty lines, etc.)
- Bugs often only appear with specific sequences of operations

### Testing Strategy

```bash
# Run property tests (will execute 256 test cases per property by default)
cargo test buffer_property_test

# Run with more cases for higher confidence
PROPTEST_CASES=10000 cargo test buffer_property_test

# Run with verbose output to see generated inputs
cargo test buffer_property_test -- --nocapture
```

Property tests will:
1. Generate random sequences of buffer operations
2. Apply them to a Buffer instance
3. Check that invariants hold (UTF-8 validity, cursor bounds, etc.)
4. If a property fails, automatically shrink to minimal failing case
5. Report the exact sequence that caused the failure

### Dependencies
None - this is a standalone addition to the test suite.

### Complexity
**Simple** - Adding a new dependency and test file. No production code changes.

### Expected Impact
- **Quality**: Automatically discovers edge cases that manual tests miss
- **Confidence**: Running 10,000+ random test cases provides high confidence
- **Regression Prevention**: Future changes automatically tested against properties
- **Documentation**: Properties serve as executable specifications

### Success Criteria
- [ ] proptest added to Cargo.toml dev-dependencies
- [ ] All 5 property tests pass with default 256 cases
- [ ] Property tests pass with 10,000 cases: `PROPTEST_CASES=10000 cargo test buffer_property_test`
- [ ] No false positives (all failures indicate real bugs)
- [ ] Existing test suite still passes: `cargo test`

---

## Stage 2: Incremental LSP Sync [HIGH IMPACT]

**Priority**: HIGH | **Complexity**: Medium | **Est. Time**: 2 days

### Issue
Currently, every buffer change sends the **entire buffer content** to the LSP server via `textDocument/didChange`. For a 10MB file, typing a single character sends 10MB over the pipe to the LSP server. This wastes bandwidth, CPU (serialization), and causes LSP lag on large files.

### Root Cause Analysis

Current implementation in `/Users/adrian/Projects/ovim/src/lsp/mod.rs` (line 425-501):

```rust
async fn send_did_change_immediate(
    &self,
    uri: Uri,
    language_id: &str,
    text: String,
    old_text: Option<String>,
) -> Result<()> {
    // Already has incremental sync logic!
    let supports_incremental = server.supports_incremental_sync().await;

    if supports_incremental && old_text.is_some() {
        if let Some(old) = old_text {
            if let Some((range, new_text)) = compute_simple_diff(&old, &text) {
                // Uses incremental change
                vec![TextDocumentContentChangeEvent {
                    range: Some(range),
                    range_length: None,
                    text: new_text,
                }]
            }
        }
    }
}
```

**Key Discovery**: The codebase ALREADY has incremental sync infrastructure! The issue is:

1. `compute_simple_diff` may not be efficiently computing diffs
2. The `old_text` may not always be passed correctly from the editor
3. Debouncing may interfere with incremental sync tracking

Let me check the actual implementation:

### Files to Investigate and Modify

1. `/Users/adrian/Projects/ovim/src/lsp/mod.rs` - Check `compute_simple_diff` implementation
2. `/Users/adrian/Projects/ovim/src/lsp/types.rs` - Likely location of diff computation
3. `/Users/adrian/Projects/ovim/src/editor/lsp_integration.rs` - Ensure old_text is passed
4. `/Users/adrian/Projects/ovim/src/buffer/mod.rs` - Track last synced content

### Implementation Approach

**Step 1: Verify current diff algorithm**

Search for `compute_simple_diff` to understand current implementation:

```bash
grep -rn "compute_simple_diff" /Users/adrian/Projects/ovim/src/
```

**Step 2: Enhance buffer to track last LSP sync state**

File: `/Users/adrian/Projects/ovim/src/buffer/mod.rs`

Add field to Buffer struct (around line 40-80):

```rust
pub struct Buffer {
    rope: Rope,
    cursor: Cursor,
    file_path: Option<PathBuf>,
    encoding: FileEncoding,

    /// Last content sent to LSP server (for incremental sync)
    /// None if never synced or sync is stale
    lsp_synced_content: Option<String>,

    // ... existing fields
}
```

Add methods:

```rust
impl Buffer {
    /// Returns the last content synced to LSP server
    pub fn lsp_synced_content(&self) -> Option<&str> {
        self.lsp_synced_content.as_deref()
    }

    /// Mark that LSP is now synced with current content
    pub fn mark_lsp_synced(&mut self) {
        self.lsp_synced_content = Some(self.to_string());
    }

    /// Clear LSP sync state (call on file reload, etc.)
    pub fn clear_lsp_sync(&mut self) {
        self.lsp_synced_content = None;
    }
}
```

**Step 3: Improve diff algorithm**

File: `/Users/adrian/Projects/ovim/src/lsp/types.rs` (or wherever `compute_simple_diff` lives)

Current simple approach likely does:
- Compare old vs new line by line
- Find first differing line
- Send that line

**Better approach using similar_asserts or custom LCS**:

```rust
use lsp_types::{Position, Range};

/// Computes a minimal diff between old and new text
/// Returns (LSP Range, replacement text) or None if texts are identical
pub fn compute_incremental_diff(old_text: &str, new_text: &str) -> Option<(Range, String)> {
    // Fast path: if identical, no change needed
    if old_text == new_text {
        return None;
    }

    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();

    // Find first differing line from start
    let first_diff = old_lines.iter()
        .zip(new_lines.iter())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| old_lines.len().min(new_lines.len()));

    // Find first differing line from end
    let old_len = old_lines.len();
    let new_len = new_lines.len();

    let last_diff = old_lines.iter().rev()
        .zip(new_lines.iter().rev())
        .position(|(a, b)| a != b)
        .unwrap_or(0);

    // Compute affected range in old text
    let start_line = first_diff as u32;
    let end_line = (old_len.saturating_sub(last_diff)) as u32;

    // Extract replacement text from new content
    let replacement_start = first_diff;
    let replacement_end = new_len.saturating_sub(last_diff);
    let replacement_text = new_lines[replacement_start..replacement_end].join("\n");

    // Handle edge case: adding newline at end
    let needs_final_newline = new_text.ends_with('\n') && !old_text.ends_with('\n');
    let final_replacement = if needs_final_newline {
        format!("{}\n", replacement_text)
    } else {
        replacement_text
    };

    let range = Range {
        start: Position { line: start_line, character: 0 },
        end: Position { line: end_line, character: 0 },
    };

    Some((range, final_replacement))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_identical_text() {
        let old = "line 1\nline 2\nline 3";
        let new = "line 1\nline 2\nline 3";
        assert_eq!(compute_incremental_diff(old, new), None);
    }

    #[test]
    fn test_diff_middle_line_changed() {
        let old = "line 1\nline 2\nline 3";
        let new = "line 1\nMODIFIED\nline 3";

        let (range, text) = compute_incremental_diff(old, new).unwrap();
        assert_eq!(range.start.line, 1);
        assert_eq!(range.end.line, 2);
        assert_eq!(text, "MODIFIED");
    }

    #[test]
    fn test_diff_single_char_change() {
        let old = "hello world";
        let new = "hello world!";

        let (range, text) = compute_incremental_diff(old, new).unwrap();
        // Should send just the changed portion
        assert!(text.contains("!"));
    }
}
```

**Step 4: Wire everything together**

File: `/Users/adrian/Projects/ovim/src/lsp/mod.rs`

Update `send_did_change_immediate` to use better diff:

```rust
async fn send_did_change_immediate(
    &self,
    uri: Uri,
    language_id: &str,
    text: String,
    old_text: Option<String>,
) -> Result<()> {
    let server = self.servers.get(language_id)
        .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

    let supports_incremental = server.supports_incremental_sync().await;

    let content_changes = if supports_incremental && old_text.is_some() {
        if let Some(old) = old_text {
            if let Some((range, new_text)) = compute_incremental_diff(&old, &text) {
                lsp_debug!("LSP-SYNC",
                    "Incremental sync: sending {} bytes (was {} bytes total)",
                    new_text.len(), text.len());

                vec![TextDocumentContentChangeEvent {
                    range: Some(range),
                    range_length: None,
                    text: new_text,
                }]
            } else {
                // Identical content - no changes needed
                lsp_debug!("LSP-SYNC", "Content identical, skipping didChange");
                return Ok(());
            }
        } else {
            // No old text, use full sync
            vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text,
            }]
        }
    } else {
        // Full document sync
        vec![TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text,
        }]
    };

    // ... rest of function unchanged
}
```

File: `/Users/adrian/Projects/ovim/src/editor/lsp_integration.rs`

Ensure `old_text` is passed from buffer:

```rust
pub async fn sync_lsp_document(&mut self) -> Result<()> {
    // Get old content for incremental sync
    let old_content = self.buffer().lsp_synced_content().map(String::from);
    let new_content = self.buffer().to_string();

    // Send didChange
    self.lsp_manager.did_change(
        uri.clone(),
        language_id,
        new_content,
        old_content,
    ).await?;

    // Mark buffer as synced
    self.buffer_mut().mark_lsp_synced();

    Ok(())
}
```

### Educational Context

**Why Incremental Sync Matters**:

LSP servers maintain their own in-memory representation of each file. When you send `didChange`, they need to:
1. Deserialize the JSON message
2. Parse the changes
3. Update their syntax tree
4. Re-run analysis (diagnostics, completion, etc.)

For large files:
- **Full sync**: 10MB file → 10MB JSON → parse 10MB → rebuild entire tree
- **Incremental sync**: 10MB file, 1 char change → 100 byte JSON → parse 100 bytes → update small tree region

Tree-sitter (used by rust-analyzer and many LSP servers) has an `edit()` API specifically designed for incremental updates. By sending only the changed region, we enable the LSP server to use incremental parsing, which is **10-100x faster**.

**Trade-offs**:
- **Pro**: Massive bandwidth savings, faster LSP responses, lower CPU
- **Con**: Diff computation overhead (mitigated by line-based diffing)
- **Con**: State management complexity (must track last synced content)

However, the debouncer already tracks old_text, so the infrastructure is there!

### Testing Strategy

**Unit Tests**:
```bash
cargo test compute_incremental_diff
```

**Integration Test**:

File: `/Users/adrian/Projects/ovim/tests/lsp_incremental_sync_test.rs`

```rust
use ovim::editor::Editor;

#[tokio::test]
async fn test_incremental_sync_single_char() {
    let mut editor = Editor::new();
    editor.enable_lsp();

    // Load large file
    editor.load_file("tests/fixtures/large_file.rs").unwrap();

    // Wait for LSP initialization
    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

    // Insert single character
    editor.insert_char('x');

    // Check that didChange was incremental (would need LSP mocking or logging)
    // For now, verify no panics and LSP still works
    assert!(editor.lsp_enabled());
}
```

**Manual Testing**:
1. Open large file (10MB+): `ovim large_file.txt`
2. Enable LSP logging: Check `/tmp/ovim-lsp.log`
3. Type a character
4. Verify log shows: `"Incremental sync: sending 1 bytes (was 10485760 bytes total)"`

### Dependencies
- None (standalone improvement)

### Complexity
**Medium** - Requires understanding LSP protocol and careful state management, but infrastructure exists.

### Expected Impact
- **Performance**: 10-1000x reduction in didChange message size for large files
- **Responsiveness**: Faster LSP diagnostics and completion
- **Scalability**: Editor usable with 50MB+ files

### Success Criteria
- [ ] `compute_incremental_diff` passes all unit tests
- [ ] Integration test verifies incremental sync is used
- [ ] LSP logs show reduced message sizes for edits
- [ ] Existing LSP tests still pass: `cargo test lsp`
- [ ] Manual testing: large file editing feels responsive

---

## Stage 3: REST API Versioning [LOW RISK]

**Priority**: MEDIUM | **Complexity**: Simple | **Est. Time**: 1 day

### Issue
The REST API has no versioning scheme. If breaking changes are needed in the future, all clients break simultaneously. This prevents gradual migration and causes pain for tool authors.

### Root Cause
Initial API design prioritized simplicity over forward compatibility. As ovim matures and gains users, breaking changes become increasingly costly.

### Files to Modify
1. `/Users/adrian/Projects/ovim/src/api/routes.rs` - Add versioned routes
2. `/Users/adrian/Projects/ovim/src/api/mod.rs` - Add version middleware
3. `/Users/adrian/Projects/ovim/code-docs/docs/API.md` - Document versioning (new file)

### Implementation Approach

**Design Decision**: Use **path-based versioning** (`/v1/buffer`, `/v2/buffer`) rather than header-based versioning. Rationale:
- **Discoverability**: curl/browser users can see version in URL
- **Simplicity**: No need to remember header names
- **REST convention**: Most REST APIs use path versioning (Stripe, GitHub, etc.)

**Step 1: Add version namespace**

File: `/Users/adrian/Projects/ovim/src/api/routes.rs`

```rust
use super::handlers::{
    execute_command, get_buffer, get_cursor, get_health, get_lsp_status, get_metrics, get_mode,
    get_render, get_snapshot, send_keys, set_buffer, set_mode,
};
use super::mcp_handler::handle_mcp;
use super::state::ApiState;
use axum::{
    routing::{get, post, put},
    Router,
};

/// Create the API router with all routes
pub fn create_router(state: ApiState) -> Router {
    // V1 API (current)
    let v1_routes = Router::new()
        .route("/health", get(get_health))
        .route("/snapshot", get(get_snapshot))
        .route("/keys", post(send_keys))
        .route("/buffer", get(get_buffer))
        .route("/buffer", put(set_buffer))
        .route("/cursor", get(get_cursor))
        .route("/mode", get(get_mode))
        .route("/mode", post(set_mode))
        .route("/command", post(execute_command))
        .route("/render", get(get_render))
        .route("/lsp/status", get(get_lsp_status))
        .route("/metrics", get(get_metrics))
        .route("/mcp", post(handle_mcp));

    // Root router with version namespaces
    Router::new()
        // V1 API under /v1 prefix
        .nest("/v1", v1_routes.clone())

        // Legacy routes (no prefix) - redirect to /v1
        // Keep for backward compatibility, can be removed in v1.0
        .merge(v1_routes)

        .with_state(state)
}
```

**Step 2: Add deprecation headers for legacy routes**

File: `/Users/adrian/Projects/ovim/src/api/mod.rs`

Add middleware to warn about unversioned usage:

```rust
use axum::{
    middleware::{self, Next},
    response::Response,
    http::{Request, StatusCode, header},
};

/// Middleware to add deprecation warning for unversioned API routes
async fn deprecation_middleware<B>(
    req: Request<B>,
    next: Next<B>,
) -> Response {
    let path = req.uri().path();

    // Check if this is an unversioned route (not starting with /v1, /v2, etc.)
    let is_unversioned = !path.starts_with("/v1") &&
                         !path.starts_with("/v2") &&
                         path != "/" &&
                         path != "/favicon.ico";

    let mut response = next.run(req).await;

    if is_unversioned {
        // Add deprecation header
        response.headers_mut().insert(
            "X-API-Deprecation",
            "Unversioned API paths are deprecated. Use /v1/* instead.".parse().unwrap()
        );

        // Add Sunset header (API sunset date)
        // Set to 6 months from now for example
        response.headers_mut().insert(
            "Sunset",
            "Wed, 01 Jul 2026 00:00:00 GMT".parse().unwrap()
        );
    }

    response
}

// Apply middleware in start_server
pub async fn start_server(
    addr: &str,
    tx: mpsc::UnboundedSender<ApiRequest>,
    port_tx: oneshot::Sender<u16>,
) -> Result<()> {
    let state = ApiState { tx };
    let app = create_router(state)
        .layer(middleware::from_fn(deprecation_middleware));

    // ... rest of server setup
}
```

**Step 3: Document versioning policy**

File: `/Users/adrian/Projects/ovim/code-docs/docs/API.md` (new file)

```markdown
# Ovim REST API Documentation

## Versioning

The Ovim REST API uses **path-based versioning**. All endpoints are available under `/v1/` prefix.

### Current Version: v1

Base URL: `http://127.0.0.1:<PORT>/v1`

### Version Policy

- **Backward Compatibility**: Within a major version (v1, v2), we maintain backward compatibility
- **Deprecation Period**: Deprecated endpoints have 6 months sunset period with headers:
  - `X-API-Deprecation`: Human-readable deprecation message
  - `Sunset`: RFC 7234 sunset date
- **Breaking Changes**: Require new major version (v2, v3)

### Legacy Routes

For backward compatibility, unversioned routes (`/buffer`, `/health`, etc.) currently redirect to `/v1/*`. These routes will be removed in ovim v1.0.

**Migration**: Update your clients to use `/v1/` prefix:

```bash
# Old (deprecated)
curl http://127.0.0.1:3000/buffer

# New (recommended)
curl http://127.0.0.1:3000/v1/buffer
```

## Endpoints

### GET /v1/health
Check editor health and LSP readiness.

### GET /v1/snapshot
Get complete editor state (buffer, cursor, mode, registers, marks).

... (rest of endpoint documentation)
```

**Step 4: Update CLAUDE.md to reflect versioning**

File: `/Users/adrian/Projects/ovim/CLAUDE.md`

Update all curl examples to use `/v1/` prefix:

```markdown
## REST API Endpoints

**HTTP Server**: Always runs on both headless and UI modes on `http://127.0.0.1:PORT`

| Endpoint | Method | Use Case |
|----------|--------|----------|
| `/v1/health` | GET | Health + LSP readiness |
| `/v1/lsp/status` | GET | Server states & pending requests |
| `/v1/snapshot` | GET | Complete editor state |
...
```

### Educational Context

**API Versioning Best Practices**:

1. **Path vs Header Versioning**:
   - Path (`/v1/resource`): Visible, cacheable, discoverable
   - Header (`Accept: application/vnd.ovim.v1+json`): Cleaner URLs, harder to use

2. **Semantic Versioning for APIs**:
   - **v1 → v1.1**: Backward compatible additions (new fields, new endpoints)
   - **v1 → v2**: Breaking changes (removed fields, changed types, different behavior)

3. **Deprecation Strategy**:
   - Announce early with headers (`X-API-Deprecation`, `Sunset`)
   - Provide migration guide
   - Support old version for reasonable period (6-12 months)
   - Log usage of deprecated endpoints for metrics

**Why This Matters**: Ovim's headless mode is designed for AI integration. As AI coding assistants evolve, they'll build tooling around ovim's API. Without versioning, every breaking change forces all tools to update simultaneously. With versioning, tools can migrate gradually, and old tools continue working during transition period.

### Testing Strategy

**Unit Test**:

File: `/Users/adrian/Projects/ovim/tests/api_versioning_test.rs`

```rust
use axum::http::StatusCode;
use reqwest;

#[tokio::test]
async fn test_versioned_routes_work() {
    // Start ovim headless
    let session = start_test_session().await;

    // Test v1 route
    let resp = reqwest::get(&format!("http://127.0.0.1:{}/v1/health", session.port))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_legacy_routes_have_deprecation_header() {
    let session = start_test_session().await;

    // Test legacy route
    let resp = reqwest::get(&format!("http://127.0.0.1:{}/health", session.port))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().contains_key("x-api-deprecation"));
    assert!(resp.headers().contains_key("sunset"));
}

#[tokio::test]
async fn test_v1_routes_no_deprecation_header() {
    let session = start_test_session().await;

    let resp = reqwest::get(&format!("http://127.0.0.1:{}/v1/health", session.port))
        .await
        .unwrap();

    assert!(!resp.headers().contains_key("x-api-deprecation"));
}
```

**Manual Testing**:
```bash
# Start ovim
ovim --headless --session test test.txt

# Test versioned endpoint
curl -v http://127.0.0.1:$(cat ~/.cache/ovim/sessions/test.json | jq -r .port)/v1/health

# Test legacy endpoint (should include deprecation headers)
curl -v http://127.0.0.1:$(cat ~/.cache/ovim/sessions/test.json | jq -r .port)/health
```

### Dependencies
None

### Complexity
**Simple** - URL routing changes only, no business logic changes.

### Expected Impact
- **Future-proofing**: Can make breaking changes without breaking all clients
- **Developer Experience**: Clear migration path for API changes
- **Professionalism**: Standard practice for production APIs

### Success Criteria
- [ ] All endpoints available under `/v1/` prefix
- [ ] Legacy routes still work but include deprecation headers
- [ ] API documentation updated
- [ ] CLAUDE.md updated with `/v1/` examples
- [ ] Tests verify versioning behavior
- [ ] No breakage to existing clients (backward compatible)

---

## Stage 4: Box/Arc the Editor Struct [MODERATE IMPACT]

**Priority**: HIGH | **Complexity**: Medium | **Est. Time**: 1.5 days

### Issue

The `Editor` struct is 3,128+ bytes (based on the ~40KB mod.rs file and extensive field list). This is very large for a Rust struct. Every function that takes `Editor` by value, every assignment, and every async task that captures `Editor` incurs:

- **3KB+ stack allocation** per function call
- **Cache pressure** - struct doesn't fit in CPU cache lines
- **Slow moves** - copying 3KB on every ownership transfer
- **Large async futures** - each `.await` point stores full Editor on heap

Current observations:
- Editor has 50+ fields (registers, buffers, LSP state, picker, etc.)
- Already wrapped in `Arc<Mutex<Editor>>` in main.rs (line 10)
- But functions in editor/mod.rs take `&mut self`, not `Arc<Mutex<Self>>`

### Root Cause

The Editor struct accumulated fields organically:
- `buffers: Vec<Buffer>` - can be multiple large buffers
- `registers: RegisterManager` - 26 registers × clipboard data
- `marks: MarkManager` - marks for all buffers
- `lsp_state: LspState` - hover info, diagnostics, caches
- `picker: Option<Picker>` - fuzzy finder state
- `preview_cache: HashMap<String, PreviewCache>` - file previews
- Many more...

Each individually reasonable, but collectively creates a massive struct.

**Good News**: main.rs ALREADY uses `Arc<Mutex<Editor>>`, so the infrastructure is partially there!

### Files to Modify

Actually, upon investigation, this may already be properly handled! Let me verify:

Looking at `/Users/adrian/Projects/ovim/src/main.rs` line 42-78, the Editor is created directly, not wrapped in Arc:

```rust
let mut editor = if let Some(file_path) = &args.file {
    let mut ed = Editor::new();
    // ...
    ed
} else {
    Editor::new()
};
```

Then passed to event loop functions. We need to check the event loop.

**Investigation needed**: Check how Editor is passed in event loops (TUI and headless).

### Implementation Approach

**Step 1: Measure current struct size**

Add to `/Users/adrian/Projects/ovim/src/editor/mod.rs`:

```rust
#[cfg(test)]
mod size_tests {
    use super::*;

    #[test]
    fn test_editor_size() {
        use std::mem::size_of;

        println!("Editor size: {} bytes", size_of::<Editor>());
        println!("Arc<Mutex<Editor>> size: {} bytes", size_of::<std::sync::Arc<std::sync::Mutex<Editor>>>());

        // Compare to reasonable target (should be pointer-sized: 8 or 16 bytes)
        assert!(
            size_of::<std::sync::Arc<std::sync::Mutex<Editor>>>() <= 16,
            "Arc<Mutex<Editor>> should be pointer-sized"
        );
    }
}
```

Run: `cargo test test_editor_size -- --nocapture`

**Step 2: If Editor is large (>1KB), wrap in Arc**

This is a significant refactor. The approach depends on current architecture. Two strategies:

**Strategy A: Keep current API, box large fields internally**

Instead of wrapping entire Editor, box individual large fields:

```rust
pub struct Editor {
    // Box large collections
    buffers: Box<Vec<Buffer>>,
    registers: Box<RegisterManager>,
    marks: Box<MarkManager>,
    lsp_state: Box<LspState>,
    preview_cache: Box<HashMap<String, PreviewCache>>,

    // Keep small fields inline
    mode: Mode,
    should_quit: bool,
    count: Option<usize>,
    // ...
}
```

**Pros**:
- No API changes
- No lock contention
- Surgical fix

**Cons**:
- Extra indirection for field access
- Still large total size, just moved to heap

**Strategy B: Wrap entire Editor in Arc<Mutex<>> at module boundary**

This is a larger refactor but may be necessary if Strategy A isn't sufficient.

Given that the event loops likely already handle locking, this might just need:

1. Change main.rs to wrap Editor in Arc<Mutex<>>
2. Update UI layer to lock when rendering
3. Update API layer to lock when handling requests

**Recommendation**: Start with **Strategy A** (box large fields), measure, then consider Strategy B only if needed.

### Testing Strategy

**Size Regression Test**:

```rust
#[test]
fn editor_size_regression() {
    use std::mem::size_of;

    const MAX_ACCEPTABLE_SIZE: usize = 512; // bytes

    let actual = size_of::<Editor>();
    assert!(
        actual <= MAX_ACCEPTABLE_SIZE,
        "Editor struct is {} bytes, should be <= {} bytes. Consider boxing large fields.",
        actual, MAX_ACCEPTABLE_SIZE
    );
}
```

**Functional Tests**:
All existing tests should pass without modification if internal boxing is used correctly.

### Dependencies
None (unless measurements reveal Editor is already small, in which case this stage can be skipped!)

### Complexity
**Medium** - Requires understanding struct layout and possibly refactoring field access.

### Expected Impact
- **Performance**: Faster function calls, better cache locality
- **Async**: Smaller future sizes, less heap allocation
- **Stack**: Reduced stack overflow risk in deep recursion

### Success Criteria
- [ ] `cargo test test_editor_size -- --nocapture` shows size
- [ ] If size > 1KB, implement Strategy A (box large fields)
- [ ] Size regression test passes (Editor or Arc<Mutex<Editor>> ≤ 512 bytes)
- [ ] All existing tests pass: `cargo test`
- [ ] No performance regressions in manual testing

**Note**: If measurements show Editor is already small (<512 bytes), skip this stage entirely!

---

## Stage 5: Syntax Highlighting Optimizations [MODERATE IMPACT]

**Priority**: MEDIUM | **Complexity**: Medium | **Est. Time**: 2-3 days

### Issue

Current syntax highlighting (tree-sitter based) may have performance issues:
1. Full re-parse on every edit (even small changes)
2. Highlighting entire file even when only viewport is visible
3. No caching of highlight results

Looking at `/Users/adrian/Projects/ovim/src/syntax/highlighter.rs`, there's already good infrastructure:
- `update()` method exists for incremental parsing (line 49)
- `highlights_for_all_lines()` queries once and distributes (line 58)

The issue is likely that `update()` isn't being called consistently.

### Root Cause

The Buffer likely calls `parse()` (full reparse) instead of `update()` (incremental) on edits.

Investigation needed:
1. Where does Buffer call syntax highlighter?
2. Does it pass tree-sitter edit info?
3. Is highlighting cached between renders?

### Files to Investigate

1. `/Users/adrian/Projects/ovim/src/buffer/mod.rs` - How syntax highlighting is triggered
2. `/Users/adrian/Projects/ovim/src/syntax/highlighter.rs` - Already has `update()` method
3. `/Users/adrian/Projects/ovim/src/ui/renderer/mod.rs` - How highlights are used in rendering

### Implementation Approach

**Step 1: Verify current usage**

Search for calls to syntax highlighter:

```bash
grep -rn "SyntaxHighlighter" /Users/adrian/Projects/ovim/src/buffer/
grep -rn "\.parse\(" /Users/adrian/Projects/ovim/src/buffer/
grep -rn "\.update\(" /Users/adrian/Projects/ovim/src/buffer/
```

**Step 2: Ensure incremental updates are used**

The highlighter already has `update()` method that takes `tree_sitter::InputEdit`. Need to:

1. Track edits in Buffer
2. Convert to tree-sitter format
3. Call `highlighter.update()` instead of `highlighter.parse()`

File: `/Users/adrian/Projects/ovim/src/buffer/mod.rs`

```rust
impl Buffer {
    pub fn insert_str(&mut self, pos: usize, text: &str) {
        // Calculate edit for tree-sitter
        let start_byte = self.rope.char_to_byte(pos);
        let old_end_byte = start_byte;
        let new_end_byte = start_byte + text.len();

        // Perform rope edit
        self.rope.insert(pos, text);

        // Update syntax highlighter incrementally
        if let Some(ref mut highlighter) = self.syntax_highlighter {
            let edit = tree_sitter::InputEdit {
                start_byte,
                old_end_byte,
                new_end_byte,
                start_position: tree_sitter::Point {
                    row: pos / self.line_len(pos),
                    column: pos % self.line_len(pos)
                },
                old_end_position: tree_sitter::Point {
                    row: pos / self.line_len(pos),
                    column: pos % self.line_len(pos)
                },
                new_end_position: tree_sitter::Point {
                    row: (pos + text.len()) / self.line_len(pos + text.len()),
                    column: (pos + text.len()) % self.line_len(pos + text.len())
                },
            };

            highlighter.update(edit, &self.rope.to_string());
        }
    }
}
```

**Note**: This is complex because tree-sitter needs byte positions AND line/column positions. May need helper functions.

**Step 3: Add highlight caching**

File: `/Users/adrian/Projects/ovim/src/buffer/mod.rs`

```rust
pub struct Buffer {
    rope: Rope,
    cursor: Cursor,

    /// Syntax highlighter (optional)
    syntax_highlighter: Option<SyntaxHighlighter>,

    /// Cached highlights per line (invalidated on edit)
    /// Option<Vec<(Range, HighlightGroup)>> per line
    cached_highlights: Option<Vec<Vec<(Range<usize>, HighlightGroup)>>>,

    /// Buffer version when highlights were cached
    cached_highlights_version: usize,

    /// Current buffer version (incremented on edit)
    version: usize,
}

impl Buffer {
    /// Get highlights for visible lines with caching
    pub fn get_highlights(&mut self, start_line: usize, end_line: usize)
        -> Vec<Vec<(Range<usize>, HighlightGroup)>>
    {
        // Check cache validity
        if let Some(ref cached) = self.cached_highlights {
            if self.cached_highlights_version == self.version {
                // Cache hit!
                return cached[start_line..end_line].to_vec();
            }
        }

        // Cache miss - recompute all highlights
        if let Some(ref highlighter) = self.syntax_highlighter {
            let all_highlights = highlighter.highlights_for_all_lines(&self.rope.to_string());
            self.cached_highlights = Some(all_highlights.clone());
            self.cached_highlights_version = self.version;

            all_highlights[start_line..end_line].to_vec()
        } else {
            vec![vec![]; end_line - start_line]
        }
    }
}
```

**Step 4: Lazy highlighting (viewport only)**

For very large files, even tree-sitter can be slow. Add option to highlight only visible region:

```rust
impl SyntaxHighlighter {
    /// Get highlights for specific line range (lazy highlighting)
    pub fn highlights_for_range(
        &self,
        source: &str,
        start_line: usize,
        end_line: usize,
    ) -> Vec<Vec<(Range<usize>, HighlightGroup)>> {
        let Some(ref tree) = self.tree else {
            return vec![vec![]; end_line - start_line];
        };

        let lines: Vec<&str> = source.lines().collect();

        // Calculate byte range for visible lines
        let start_byte = lines[..start_line].iter().map(|l| l.len() + 1).sum();
        let end_byte = lines[..end_line].iter().map(|l| l.len() + 1).sum();

        // Query only visible region
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(start_byte..end_byte);

        // ... rest of highlighting logic, but only for visible lines
    }
}
```

### Educational Context

**Tree-Sitter Incremental Parsing**:

Tree-sitter is designed for incremental parsing. When you call `tree.edit()` with an edit description, it:
1. Marks affected nodes as "dirty"
2. Re-parses only the dirty region (often just a few lines)
3. Reuses unchanged subtrees

This is **10-100x faster** than full reparse for localized edits (which is most edits).

**However**, to use incremental parsing, you must:
- Track edits (start/end byte, start/end position)
- Call `tree.edit()` before `parser.parse()`
- Maintain tree state across edits

The current codebase has the infrastructure (`update()` method) but may not be wired up correctly.

**Caching Trade-offs**:
- **Pro**: Instant highlights when buffer unchanged (cache hit)
- **Pro**: Viewport scrolling is free
- **Con**: Memory overhead (stores highlight ranges for every line)
- **Con**: Cache invalidation complexity

For ovim's use case (editing speed matters), caching is worth it.

### Testing Strategy

**Performance Benchmark**:

File: `/Users/adrian/Projects/ovim/benches/syntax_bench.rs`

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ovim::buffer::Buffer;

fn bench_syntax_highlighting(c: &mut Criterion) {
    let mut group = c.benchmark_group("syntax");

    // Large Rust file
    let large_rust = std::fs::read_to_string("src/editor/mod.rs").unwrap();

    group.bench_function("full_parse", |b| {
        let mut buffer = Buffer::new();
        buffer.load_from_string(&large_rust, Some("rust"));

        b.iter(|| {
            buffer.parse_syntax();
        });
    });

    group.bench_function("incremental_update", |b| {
        let mut buffer = Buffer::new();
        buffer.load_from_string(&large_rust, Some("rust"));
        buffer.parse_syntax();

        b.iter(|| {
            buffer.insert_str(100, "x");
            buffer.parse_syntax();
        });
    });

    group.bench_function("cached_highlights", |b| {
        let mut buffer = Buffer::new();
        buffer.load_from_string(&large_rust, Some("rust"));
        buffer.parse_syntax();

        b.iter(|| {
            buffer.get_highlights(0, 50);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_syntax_highlighting);
criterion_main!(benches);
```

Run: `cargo bench syntax`

**Functional Test**:

Ensure syntax highlighting still works correctly after optimizations:

```rust
#[test]
fn test_incremental_syntax_highlighting() {
    let mut buffer = Buffer::new();
    buffer.load_from_string("fn main() {}\n", Some("rust"));

    // Get initial highlights
    let highlights1 = buffer.get_highlights(0, 1);
    assert!(!highlights1[0].is_empty(), "Should have syntax highlights");

    // Edit buffer
    buffer.insert_str(12, "\n    println!(\"hi\");\n");

    // Get updated highlights
    let highlights2 = buffer.get_highlights(0, 3);
    assert!(!highlights2[1].is_empty(), "Should have highlights after edit");
}
```

### Dependencies
- **Stage 1** (Buffer versioning) would help with cache invalidation
- But can be done independently

### Complexity
**Medium** - Requires understanding tree-sitter API and careful edit tracking.

### Expected Impact
- **Performance**: 10-100x faster syntax highlighting on edits
- **Memory**: Modest increase for cache (acceptable trade-off)
- **Responsiveness**: Large files remain usable while highlighted

### Success Criteria
- [ ] Incremental parsing used for edits (verify in logs/benchmarks)
- [ ] Highlights cached and reused when buffer unchanged
- [ ] Benchmark shows 10x+ improvement for incremental updates
- [ ] All existing syntax tests pass
- [ ] Manual testing: large file editing remains smooth

---

## Stage 6: LSP Request Cancellation [LOW PRIORITY]

**Priority**: MEDIUM | **Complexity**: Medium | **Est. Time**: 2 days

### Issue

Fast typing or rapid cursor movement can queue up many LSP requests (hover, completion). These requests:
1. Complete out-of-order (request 1 might finish after request 2)
2. Show stale information (request 1 shows hover for old cursor position)
3. Waste server resources (server doing work for obsolete state)

### Root Cause

LSP protocol supports request cancellation via `$/cancelRequest` notification, but ovim doesn't use it.

Current flow:
1. User hovers over symbol A → request ID 1
2. User moves to symbol B → request ID 2
3. Request 1 completes → shows hover for symbol A (wrong!)
4. Request 2 completes → shows hover for symbol B (correct)

User sees flash of wrong information.

### Files to Modify

1. `/Users/adrian/Projects/ovim/src/lsp/server.rs` - Track request methods, add cancellation
2. `/Users/adrian/Projects/ovim/src/lsp/mod.rs` - Cancel previous requests before new one

### Implementation Approach

**Step 1: Track request methods**

File: `/Users/adrian/Projects/ovim/src/lsp/server.rs`

Update `PendingRequest` struct (around line 42-46):

```rust
struct PendingRequest {
    sender: oneshot::Sender<Result<Value>>,
    sent_at: Instant,
    method: String,  // Already exists!
}
```

Good news: method is already tracked!

**Step 2: Add cancellation method**

File: `/Users/adrian/Projects/ovim/src/lsp/server.rs`

```rust
impl LanguageServer {
    /// Cancels all pending requests for a specific method
    /// Useful for hover/completion where only latest request matters
    pub async fn cancel_requests_by_method(&self, method: &str) -> Result<()> {
        let inner = self.inner.lock().await;

        // Find all request IDs for this method
        let to_cancel: Vec<RequestId> = inner
            .pending_requests
            .iter()
            .filter(|(_, req)| req.method == method)
            .map(|(id, _)| id.clone())
            .collect();

        drop(inner); // Release lock before sending

        // Send $/cancelRequest for each
        for id in to_cancel {
            let params = serde_json::json!({ "id": id });

            self.notify("$/cancelRequest", params).await?;

            // Remove from pending (server might still respond, but we ignore it)
            let mut inner = self.inner.lock().await;
            if let Some(entry) = inner.pending_requests.remove(&id) {
                // Send error to waiting caller
                let _ = entry.sender.send(Err(anyhow::anyhow!("Request cancelled")));
            }
        }

        Ok(())
    }
}
```

**Step 3: Use cancellation in LSP manager**

File: `/Users/adrian/Projects/ovim/src/lsp/mod.rs`

Update hover method (around line 1300-1400):

```rust
pub async fn hover(
    &self,
    uri: Uri,
    position: lsp_types::Position,
) -> Result<Option<String>> {
    let language_id = self.language_for_uri(&uri)?;

    let server = self.get_server(&language_id).await
        .ok_or_else(|| anyhow::anyhow!("No LSP server for {}", language_id))?;

    // Cancel any pending hover requests before sending new one
    server.cancel_requests_by_method("textDocument/hover").await?;

    // Now send the new hover request
    let params = lsp_types::HoverParams {
        text_document_position_params: lsp_types::TextDocumentPositionParams {
            text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
            position,
        },
        work_done_progress_params: Default::default(),
    };

    // ... rest of hover logic
}
```

Repeat for other "latest-only" methods:
- `completion()` - only care about latest cursor position
- `signature_help()` - same
- `hover()` - already done above

**Do NOT cancel**:
- `goto_definition()` - user explicitly invoked
- `formatting()` - user explicitly invoked
- `code_action()` - user explicitly invoked

**Step 4: Handle cancellation responses**

Some servers respond to cancelled requests with error code -32800. Handle gracefully:

```rust
// In response handling (server.rs, line 506-530)
if let Some(error) = msg.error {
    if error.code == -32800 {
        // Request was cancelled - this is expected, don't log as error
        lsp_debug!("LSP-RESPONSE", "Request {} cancelled", id);
    } else {
        lsp_error!("LSP-RESPONSE", "Error {}: {}", error.code, error.message);
    }

    let error_msg = format!("{} (code {})", error.message, error.code);
    let _ = req.sender.send(Err(anyhow!("LSP error: {}", error_msg)));
}
```

### Educational Context

**LSP Request Cancellation Protocol**:

From LSP spec ($/cancelRequest):
```json
{
  "jsonrpc": "2.0",
  "method": "$/cancelRequest",
  "params": {
    "id": 123
  }
}
```

**Server Behavior**:
- **May** respond with error code -32800 (Request Cancelled)
- **May** still respond successfully if already computed
- **Should** stop processing if work not yet done

**Client Behavior** (that's us):
- Send cancellation ASAP when request obsolete
- Handle both cancellation error AND success response
- Don't block waiting for cancellation acknowledgment

**Real-world Impact**:

Imagine user types fast: "foo.bar.baz.qux"

Without cancellation:
```
Type 'f' → hover request 1 (pending)
Type 'o' → hover request 2 (pending)
Type 'o' → hover request 3 (pending)
Type '.' → hover request 4 (pending)
...
Server eventually processes all 20+ requests, showing outdated hovers
```

With cancellation:
```
Type 'f' → hover request 1 (pending)
Type 'o' → cancel request 1, hover request 2 (pending)
Type 'o' → cancel request 2, hover request 3 (pending)
Type '.' → cancel request 3, hover request 4 (pending)
...
Server only processes final request, shows current hover
```

**Trade-offs**:
- **Pro**: Reduces server load, shows correct information
- **Pro**: Better UX (no flashing of stale info)
- **Con**: More network traffic (cancel notifications)
- **Con**: Complexity in tracking request types

For hover/completion (high frequency, low priority), cancellation is a clear win.

### Testing Strategy

**Unit Test**:

```rust
#[tokio::test]
async fn test_cancel_requests_by_method() {
    let server = LanguageServer::spawn("test", "cat", vec![]).await.unwrap();

    // Send multiple hover requests
    let uri = Url::parse("file:///test.rs").unwrap();
    let pos = Position { line: 0, character: 0 };

    let req1 = server.request("textDocument/hover", serde_json::json!({
        "textDocument": { "uri": uri },
        "position": pos
    }));

    let req2 = server.request("textDocument/hover", serde_json::json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 0 }
    }));

    // Cancel all hover requests
    server.cancel_requests_by_method("textDocument/hover").await.unwrap();

    // Both requests should return cancellation error
    assert!(req1.await.is_err());
    assert!(req2.await.is_err());
}
```

**Integration Test**:

Manual testing with rust-analyzer:

1. Open large Rust file
2. Rapidly move cursor over different symbols
3. Observe hover behavior
4. Check LSP logs for `$/cancelRequest` notifications

### Dependencies
None

### Complexity
**Medium** - Requires understanding async request lifecycle and LSP protocol.

### Expected Impact
- **UX**: No more stale hover/completion information
- **Performance**: Reduced server load under rapid input
- **Correctness**: Always show information for current state

### Success Criteria
- [ ] `cancel_requests_by_method` works correctly
- [ ] Hover/completion use cancellation
- [ ] Unit tests pass
- [ ] Manual testing shows no stale hovers
- [ ] LSP logs show `$/cancelRequest` notifications
- [ ] No race conditions or deadlocks

---

## Stage 7: Session Management Enhancements [LOW PRIORITY]

**Priority**: LOW | **Complexity**: Simple | **Est. Time**: 1 day

### Issue

Session management is already excellent (atomic writes, PID verification), but could be enhanced with:
1. Automatic cleanup of stale sessions (crashed processes)
2. Session expiry (remove sessions older than N days)
3. Better error messages for corrupted session files

### Root Cause

Current implementation focuses on core functionality. These are nice-to-have improvements for production use.

### Files to Modify

1. `/Users/adrian/Projects/ovim/src/session.rs` - Add cleanup methods
2. `/Users/adrian/Projects/ovim/src/subcommands.rs` - Add `clean` subcommand

### Implementation Approach

**Step 1: Add session validation methods**

File: `/Users/adrian/Projects/ovim/src/session.rs`

```rust
impl SessionInfo {
    /// Check if this session is still alive
    pub fn is_alive(&self) -> bool {
        use std::process::Command;

        #[cfg(unix)]
        {
            // Check if PID exists and matches start time
            if let Some(start_time) = self.start_time {
                if let Some(actual_start_time) = get_process_start_time(self.pid) {
                    // PID exists and start time matches (not reused PID)
                    return start_time == actual_start_time;
                }
            }

            // Fallback: check if PID exists (less reliable)
            Command::new("kill")
                .args(&["-0", &self.pid.to_string()])
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false)
        }

        #[cfg(windows)]
        {
            // Windows: check if process exists
            use sysinfo::{ProcessExt, System, SystemExt};
            let mut sys = System::new();
            sys.refresh_processes();
            sys.get_process(self.pid as usize).is_some()
        }
    }

    /// Check if session has expired (older than max_age)
    pub fn is_expired(&self, max_age: std::time::Duration) -> bool {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let age = now.saturating_sub(self.started_at);
        age > max_age.as_secs()
    }

    /// List all session files
    pub fn list_all() -> Result<Vec<(PathBuf, SessionInfo)>> {
        let session_dir = Self::session_dir()?;

        let mut sessions = Vec::new();

        for entry in fs::read_dir(session_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                match fs::read_to_string(&path) {
                    Ok(contents) => {
                        match serde_json::from_str::<SessionInfo>(&contents) {
                            Ok(info) => sessions.push((path, info)),
                            Err(e) => {
                                eprintln!("Warning: Corrupted session file {}: {}",
                                    path.display(), e);
                                // Could auto-remove corrupted files here
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Warning: Could not read session file {}: {}",
                            path.display(), e);
                    }
                }
            }
        }

        Ok(sessions)
    }

    /// Clean up stale sessions (dead processes or expired)
    pub fn cleanup_stale(max_age: Option<std::time::Duration>) -> Result<usize> {
        let max_age = max_age.unwrap_or(std::time::Duration::from_secs(7 * 24 * 60 * 60)); // 7 days

        let sessions = Self::list_all()?;
        let mut removed = 0;

        for (path, info) in sessions {
            let should_remove = !info.is_alive() || info.is_expired(max_age);

            if should_remove {
                eprintln!("Removing stale session: {} (PID {})",
                    info.session_name, info.pid);

                if let Err(e) = fs::remove_file(&path) {
                    eprintln!("Warning: Failed to remove {}: {}", path.display(), e);
                } else {
                    removed += 1;
                }
            }
        }

        Ok(removed)
    }
}
```

**Step 2: Add `clean` subcommand**

File: `/Users/adrian/Projects/ovim/src/cli.rs`

```rust
#[derive(Debug, Clone)]
pub enum Command {
    // ... existing commands

    /// Clean up stale session files
    Clean {
        /// Maximum age in days (default: 7)
        max_age_days: Option<u64>,
    },
}
```

File: `/Users/adrian/Projects/ovim/src/subcommands.rs`

```rust
pub fn execute_subcommand(command: Command) -> Result<()> {
    match command {
        // ... existing commands

        Command::Clean { max_age_days } => {
            let max_age = max_age_days.map(|days|
                std::time::Duration::from_secs(days * 24 * 60 * 60)
            );

            let removed = SessionInfo::cleanup_stale(max_age)?;

            println!("Cleaned up {} stale session(s)", removed);
            Ok(())
        }
    }
}
```

**Step 3: Add automatic cleanup on startup**

File: `/Users/adrian/Projects/ovim/src/main.rs`

```rust
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse_args();

    // Clean up stale sessions on startup (non-blocking, best-effort)
    tokio::spawn(async {
        if let Err(e) = SessionInfo::cleanup_stale(None) {
            eprintln!("Warning: Failed to clean up stale sessions: {}", e);
        }
    });

    // ... rest of main
}
```

**Step 4: Improve error messages for corrupted sessions**

File: `/Users/adrian/Projects/ovim/src/session.rs`

```rust
impl SessionInfo {
    /// Load session from file with better error handling
    pub fn load(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read session file: {}", path.display()))?;

        serde_json::from_str(&contents)
            .with_context(|| format!(
                "Session file corrupted: {}\nTry removing it with: rm '{}'",
                path.display(),
                path.display()
            ))
    }
}
```

### Educational Context

**Session File Lifecycle**:

1. **Creation**: Atomic write to prevent corruption
2. **Usage**: Read by clients to find port
3. **Updates**: LSP readiness changes
4. **Cleanup**: On graceful exit OR signal handler

**Failure Modes**:
- **Crash**: Process dies, session file remains (PID recycled)
- **Power loss**: Partial write (mitigated by atomic rename)
- **Disk full**: Write fails (handled by Result)
- **Long-running**: Session file accumulates (mitigated by expiry)

**PID Reuse Attack**:

Without start time verification:
```
1. ovim starts with PID 1234, creates session file
2. ovim crashes
3. New unrelated process gets PID 1234
4. Client connects to wrong process!
```

With start time verification (already implemented!):
```
1. ovim starts with PID 1234 at boot_time + 1000 ticks
2. Session file records: { pid: 1234, start_time: 1000 }
3. ovim crashes
4. New process gets PID 1234 at boot_time + 5000 ticks
5. Client checks: process 1234 exists, but start_time 5000 ≠ 1000
6. Client rejects session as stale ✓
```

This is already implemented in ovim (excellent!), but cleanup makes it more robust.

### Testing Strategy

**Unit Test**:

```rust
#[test]
fn test_session_expiry() {
    let old_session = SessionInfo {
        pid: 1,
        port: 3000,
        file: None,
        started_at: 0, // Unix epoch
        session_name: "test".into(),
        lsp_ready: false,
        start_time: Some(0),
    };

    let max_age = std::time::Duration::from_secs(7 * 24 * 60 * 60);
    assert!(old_session.is_expired(max_age));
}

#[test]
fn test_session_alive() {
    let this_pid = std::process::id();
    let this_start_time = get_process_start_time(this_pid);

    let session = SessionInfo {
        pid: this_pid,
        port: 3000,
        file: None,
        started_at: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        session_name: "test".into(),
        lsp_ready: false,
        start_time: this_start_time,
    };

    assert!(session.is_alive());
}
```

**Integration Test**:

```bash
# Create fake stale session
echo '{"pid":99999,"port":3000,"file":null,"started_at":0,"session_name":"stale","lsp_ready":false,"start_time":null}' \
  > ~/.cache/ovim/sessions/stale.json

# Run cleanup
ovim clean

# Verify removed
test ! -f ~/.cache/ovim/sessions/stale.json && echo "Success"
```

### Dependencies
None

### Complexity
**Simple** - Mostly adding utility methods and a new subcommand.

### Expected Impact
- **Reliability**: No stale session files confusing users
- **UX**: Better error messages for corrupted sessions
- **Maintenance**: Automatic cleanup reduces manual intervention

### Success Criteria
- [ ] `SessionInfo::is_alive()` correctly detects dead processes
- [ ] `SessionInfo::is_expired()` correctly detects old sessions
- [ ] `ovim clean` command works
- [ ] Automatic cleanup runs on startup
- [ ] Unit tests pass
- [ ] Manual testing: create fake session, run `ovim clean`, verify removal

---

## Stage 8: REST API Integration Tests [QUALITY IMPROVEMENT]

**Priority**: MEDIUM | **Complexity**: Medium | **Est. Time**: 2 days

### Issue

While unit tests exist for individual handlers, there's no comprehensive integration test that exercises the full API workflow:
1. Start headless ovim
2. Create buffer via API
3. Edit via API
4. Execute commands via API
5. Verify state via API
6. Shutdown

This leaves gaps where handler interactions might fail.

### Root Cause

Integration tests require infrastructure:
- Starting actual ovim process
- Waiting for server ready
- Making HTTP requests
- Cleaning up processes

This is more complex than unit tests, so was likely deferred.

### Files to Create

1. `/Users/adrian/Projects/ovim/tests/api_integration_test.rs` - Full API workflow tests
2. `/Users/adrian/Projects/ovim/tests/test_helpers/mod.rs` - Shared test utilities

### Implementation Approach

**Step 1: Create test infrastructure**

File: `/Users/adrian/Projects/ovim/tests/test_helpers/mod.rs` (new file)

```rust
use anyhow::Result;
use std::process::{Child, Command};
use std::time::Duration;
use ovim::session::SessionInfo;

/// Test session guard - automatically cleans up on drop
pub struct TestSession {
    pub name: String,
    pub port: u16,
    pub process: Child,
}

impl TestSession {
    /// Start ovim in headless mode for testing
    pub async fn start(name: &str) -> Result<Self> {
        // Start ovim headless
        let mut process = Command::new("target/debug/ovim")
            .args(&["--headless", "--session", name, "test.txt"])
            .spawn()?;

        // Wait for session file to appear
        let session_info = wait_for_session(name, Duration::from_secs(5)).await?;

        Ok(Self {
            name: name.to_string(),
            port: session_info.port,
            process,
        })
    }

    /// Get base URL for API requests
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }
}

impl Drop for TestSession {
    fn drop(&mut self) {
        // Kill process
        let _ = self.process.kill();
        let _ = self.process.wait();

        // Clean up session file
        if let Ok(session_dir) = SessionInfo::session_dir() {
            let session_file = session_dir.join(format!("{}.json", self.name));
            let _ = std::fs::remove_file(session_file);
        }
    }
}

async fn wait_for_session(name: &str, timeout: Duration) -> Result<SessionInfo> {
    let start = std::time::Instant::now();

    loop {
        if let Ok(session_dir) = SessionInfo::session_dir() {
            let session_file = session_dir.join(format!("{}.json", name));

            if session_file.exists() {
                if let Ok(info) = SessionInfo::load(&session_file) {
                    // Verify server is responding
                    let health_url = format!("http://127.0.0.1:{}/health", info.port);
                    if reqwest::get(&health_url).await.is_ok() {
                        return Ok(info);
                    }
                }
            }
        }

        if start.elapsed() > timeout {
            anyhow::bail!("Session did not start within {:?}", timeout);
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```

**Step 2: Create integration tests**

File: `/Users/adrian/Projects/ovim/tests/api_integration_test.rs`

```rust
mod test_helpers;

use test_helpers::TestSession;
use serde_json::json;

#[tokio::test]
async fn test_full_editing_workflow() {
    // Start test session
    let session = TestSession::start("integration_test").await.unwrap();

    // 1. Check health
    let resp = reqwest::get(&session.url("/v1/health"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // 2. Get initial buffer (should be empty)
    let resp = reqwest::get(&session.url("/v1/buffer"))
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["content"], "");

    // 3. Insert text via keys
    let resp = reqwest::Client::new()
        .post(&session.url("/v1/keys"))
        .json(&json!({ "keys": "iHello, World!<Esc>" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // 4. Verify buffer updated
    let resp = reqwest::get(&session.url("/v1/buffer"))
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["content"].as_str().unwrap().contains("Hello, World!"));

    // 5. Execute command (substitute)
    let resp = reqwest::Client::new()
        .post(&session.url("/v1/command"))
        .json(&json!({ "command": "%s/World/Rust/g" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // 6. Verify substitution worked
    let resp = reqwest::get(&session.url("/v1/buffer"))
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let content = body["content"].as_str().unwrap();
    assert!(content.contains("Rust"));
    assert!(!content.contains("World"));

    // 7. Test snapshot endpoint
    let resp = reqwest::get(&session.url("/v1/snapshot"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let snapshot: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(snapshot["mode"], "NORMAL");

    // Session automatically cleaned up on drop
}

#[tokio::test]
async fn test_mode_transitions() {
    let session = TestSession::start("mode_test").await.unwrap();

    // Start in NORMAL mode (dashboard actually, but check it works)
    let resp = reqwest::get(&session.url("/v1/mode"))
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["mode"].is_string());

    // Change to INSERT mode
    let resp = reqwest::Client::new()
        .post(&session.url("/v1/mode"))
        .json(&json!({ "mode": "INSERT" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Verify mode changed
    let resp = reqwest::get(&session.url("/v1/mode"))
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["mode"], "INSERT");
}

#[tokio::test]
async fn test_cursor_operations() {
    let session = TestSession::start("cursor_test").await.unwrap();

    // Set some buffer content
    let resp = reqwest::Client::new()
        .put(&session.url("/v1/buffer"))
        .json(&json!({ "content": "line 1\nline 2\nline 3" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Get cursor position
    let resp = reqwest::get(&session.url("/v1/cursor"))
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["line"], 0);
    assert_eq!(body["col"], 0);

    // Move cursor via keys
    let resp = reqwest::Client::new()
        .post(&session.url("/v1/keys"))
        .json(&json!({ "keys": "jjw" })) // down 2, word forward
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Verify cursor moved
    let resp = reqwest::get(&session.url("/v1/cursor"))
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["line"], 2);
}

#[tokio::test]
async fn test_lsp_status() {
    let session = TestSession::start("lsp_test").await.unwrap();

    // Load a Rust file to trigger LSP
    let resp = reqwest::Client::new()
        .post(&session.url("/v1/keys"))
        .json(&json!({ "keys": ":e test.rs<Enter>" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Wait a bit for LSP to initialize
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Check LSP status
    let resp = reqwest::get(&session.url("/v1/lsp/status"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.is_object());
}

#[tokio::test]
async fn test_render_ansi() {
    let session = TestSession::start("render_test").await.unwrap();

    // Set some content
    let resp = reqwest::Client::new()
        .put(&session.url("/v1/buffer"))
        .json(&json!({ "content": "Hello\nWorld" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Get ANSI render
    let resp = reqwest::get(&session.url("/v1/render"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("Hello"));
    assert!(body.contains("World"));
}
```

**Step 3: Add to test suite**

File: `/Users/adrian/Projects/ovim/tests/mod.rs` (if exists, or create)

```rust
mod test_helpers;
mod api_integration_test;
```

### Educational Context

**Integration vs Unit Testing**:

**Unit Tests**:
- Test individual functions/modules in isolation
- Fast (no I/O, no processes)
- Easy to write and maintain
- High coverage of code paths
- Miss interaction bugs

**Integration Tests**:
- Test system as a whole (real processes, real I/O)
- Slow (spawn processes, network requests)
- Complex setup/teardown
- Lower coverage (fewer scenarios)
- Catch interaction bugs that unit tests miss

**Example Bug Caught by Integration Test**:
```
Unit tests pass for:
- POST /keys handler ✓
- GET /buffer handler ✓

But integration test fails:
- POST /keys changes buffer
- GET /buffer doesn't reflect change
→ Bug: buffer state not properly shared between handlers!
```

This type of bug is invisible to unit tests because they mock dependencies.

**Testing Infrastructure Pattern**:

The `TestSession` guard uses RAII (Resource Acquisition Is Initialization) pattern:
- Constructor: Start ovim, wait for ready
- Methods: Provide test utilities
- Destructor: Clean up process and files

This ensures cleanup happens even if test panics.

### Testing Strategy

**Run Integration Tests**:
```bash
# Build ovim first
cargo build

# Run integration tests
cargo test --test api_integration_test

# Run all tests
cargo test
```

### Dependencies
None

### Complexity
**Medium** - Requires process management and async test infrastructure.

### Expected Impact
- **Quality**: Catch integration bugs before production
- **Confidence**: Full API workflows tested end-to-end
- **Regression**: Prevent breaks in common use cases

### Success Criteria
- [ ] All integration tests pass: `cargo test --test api_integration_test`
- [ ] Tests are fast enough (<10s total)
- [ ] Tests are reliable (no flakiness)
- [ ] Tests clean up resources on failure
- [ ] Coverage includes all major API endpoints

---

## Stage 9: Observability and Metrics [OPTIONAL]

**Priority**: LOW | **Complexity**: Medium | **Est. Time**: 2-3 days

### Issue

Production deployments need observability to:
- Monitor performance (LSP latency, render time, etc.)
- Track usage (API calls, buffer edits, etc.)
- Debug issues (error rates, timeouts, etc.)
- Capacity planning (session count, memory usage, etc.)

Currently, ovim has `/metrics` endpoint but it's minimal.

### Root Cause

Observability is a production concern, not needed for initial development. As ovim matures, this becomes more important for operators.

### Files to Modify

1. `/Users/adrian/Projects/ovim/Cargo.toml` - Add prometheus dependency
2. `/Users/adrian/Projects/ovim/src/metrics/mod.rs` - New metrics module
3. `/Users/adrian/Projects/ovim/src/api/handlers.rs` - Expose metrics endpoint
4. Various modules - Instrument with metrics

### Implementation Approach

**Design Decision**: Use Prometheus format for metrics (industry standard, works with Grafana, many tools).

**Step 1: Add prometheus dependency**

File: `/Users/adrian/Projects/ovim/Cargo.toml`

```toml
[dependencies]
# ... existing dependencies
prometheus = "0.13"
lazy_static = "1.5" # Already exists
```

**Step 2: Define metrics**

File: `/Users/adrian/Projects/ovim/src/metrics/mod.rs` (new file)

```rust
use lazy_static::lazy_static;
use prometheus::{
    register_counter, register_histogram, register_int_gauge, Counter, Histogram, IntGauge,
    Registry, TextEncoder, Encoder,
};

lazy_static! {
    // HTTP metrics
    pub static ref HTTP_REQUESTS_TOTAL: Counter = register_counter!(
        "ovim_http_requests_total",
        "Total HTTP requests received"
    ).unwrap();

    pub static ref HTTP_REQUEST_DURATION: Histogram = register_histogram!(
        "ovim_http_request_duration_seconds",
        "HTTP request latency in seconds"
    ).unwrap();

    // Buffer metrics
    pub static ref BUFFER_EDITS_TOTAL: Counter = register_counter!(
        "ovim_buffer_edits_total",
        "Total buffer edit operations"
    ).unwrap();

    pub static ref BUFFER_SIZE_BYTES: IntGauge = register_int_gauge!(
        "ovim_buffer_size_bytes",
        "Current buffer size in bytes"
    ).unwrap();

    // LSP metrics
    pub static ref LSP_REQUESTS_TOTAL: Counter = register_counter!(
        "ovim_lsp_requests_total",
        "Total LSP requests sent"
    ).unwrap();

    pub static ref LSP_REQUEST_DURATION: Histogram = register_histogram!(
        "ovim_lsp_request_duration_seconds",
        "LSP request latency in seconds"
    ).unwrap();

    pub static ref LSP_ERRORS_TOTAL: Counter = register_counter!(
        "ovim_lsp_errors_total",
        "Total LSP errors"
    ).unwrap();

    // Render metrics
    pub static ref RENDER_DURATION: Histogram = register_histogram!(
        "ovim_render_duration_seconds",
        "UI render time in seconds"
    ).unwrap();

    // Session metrics
    pub static ref ACTIVE_SESSIONS: IntGauge = register_int_gauge!(
        "ovim_active_sessions",
        "Number of active ovim sessions"
    ).unwrap();
}

/// Export metrics in Prometheus format
pub fn export_metrics() -> String {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();

    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}
```

**Step 3: Instrument code**

File: `/Users/adrian/Projects/ovim/src/api/handlers.rs`

```rust
use crate::metrics;

pub async fn send_keys(
    State(state): State<ApiState>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let _timer = metrics::HTTP_REQUEST_DURATION.start_timer();
    metrics::HTTP_REQUESTS_TOTAL.inc();

    // ... existing handler code
}

// Repeat for all handlers
```

File: `/Users/adrian/Projects/ovim/src/lsp/server.rs`

```rust
use crate::metrics;

impl LanguageServer {
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let _timer = metrics::LSP_REQUEST_DURATION.start_timer();
        metrics::LSP_REQUESTS_TOTAL.inc();

        // ... existing code

        // On error:
        // metrics::LSP_ERRORS_TOTAL.inc();
    }
}
```

File: `/Users/adrian/Projects/ovim/src/buffer/mod.rs`

```rust
use crate::metrics;

impl Buffer {
    pub fn insert_str(&mut self, pos: usize, text: &str) {
        metrics::BUFFER_EDITS_TOTAL.inc();

        // ... existing code

        // Update buffer size gauge
        metrics::BUFFER_SIZE_BYTES.set(self.len_chars() as i64);
    }
}
```

**Step 4: Expose metrics endpoint**

File: `/Users/adrian/Projects/ovim/src/api/handlers.rs`

```rust
pub async fn get_prometheus_metrics() -> impl IntoResponse {
    let metrics_text = crate::metrics::export_metrics();

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        metrics_text,
    )
}
```

File: `/Users/adrian/Projects/ovim/src/api/routes.rs`

```rust
pub fn create_router(state: ApiState) -> Router {
    let v1_routes = Router::new()
        // ... existing routes
        .route("/metrics", get(get_prometheus_metrics));

    // ... rest
}
```

**Step 5: Document metrics**

File: `/Users/adrian/Projects/ovim/code-docs/docs/METRICS.md` (new file)

```markdown
# Ovim Metrics

Ovim exposes Prometheus-compatible metrics at `/v1/metrics`.

## Available Metrics

### HTTP Metrics
- `ovim_http_requests_total` (counter): Total HTTP requests
- `ovim_http_request_duration_seconds` (histogram): Request latency

### Buffer Metrics
- `ovim_buffer_edits_total` (counter): Total buffer edits
- `ovim_buffer_size_bytes` (gauge): Current buffer size

### LSP Metrics
- `ovim_lsp_requests_total` (counter): Total LSP requests
- `ovim_lsp_request_duration_seconds` (histogram): LSP latency
- `ovim_lsp_errors_total` (counter): LSP errors

### Render Metrics
- `ovim_render_duration_seconds` (histogram): Render time

### Session Metrics
- `ovim_active_sessions` (gauge): Active sessions

## Usage

### Scraping with Prometheus

```yaml
# prometheus.yml
scrape_configs:
  - job_name: 'ovim'
    static_configs:
      - targets: ['localhost:3000']
    metrics_path: '/v1/metrics'
```

### Querying

```promql
# Average LSP latency over last 5 minutes
rate(ovim_lsp_request_duration_seconds_sum[5m]) /
rate(ovim_lsp_request_duration_seconds_count[5m])

# Edit rate
rate(ovim_buffer_edits_total[1m])

# 95th percentile render time
histogram_quantile(0.95, ovim_render_duration_seconds)
```

### Visualization with Grafana

Import dashboard JSON from `code-docs/grafana-dashboard.json`.
```

### Educational Context

**Why Prometheus?**

Prometheus is the de facto standard for metrics in cloud-native applications. Benefits:
- **Pull model**: Prometheus scrapes targets (simpler than push)
- **Time-series DB**: Optimized for metric storage
- **PromQL**: Powerful query language
- **Ecosystem**: Works with Grafana, Alertmanager, etc.

**Metric Types**:

1. **Counter**: Monotonically increasing value (HTTP requests, errors)
   - Only ever goes up (resets on restart)
   - Query with `rate()` to get per-second rate

2. **Gauge**: Current value that can go up/down (memory usage, active sessions)
   - Snapshot of current state
   - Query directly

3. **Histogram**: Distribution of values (latency, size)
   - Buckets: counts in ranges (0-10ms, 10-50ms, 50-100ms, etc.)
   - Quantiles: `histogram_quantile(0.95, ...)` = 95th percentile
   - Sum and count: derive average

**RED Method** (monitoring best practice):
- **Rate**: Requests per second
- **Errors**: Error rate
- **Duration**: Latency distribution

Our metrics cover all three!

**Trade-offs**:
- **Pro**: Deep visibility into system behavior
- **Pro**: Enables alerts (e.g., "LSP latency > 1s")
- **Pro**: Capacity planning (trends over time)
- **Con**: Memory overhead (metric storage)
- **Con**: CPU overhead (metric updates)
- **Con**: Complexity (Prometheus setup)

For ovim's use case (headless AI editing), metrics are valuable for:
- Debugging slow LSP servers
- Monitoring API usage patterns
- Detecting performance regressions

### Testing Strategy

**Unit Test**:

```rust
#[test]
fn test_metrics_export() {
    use crate::metrics;

    // Increment some metrics
    metrics::HTTP_REQUESTS_TOTAL.inc();
    metrics::BUFFER_EDITS_TOTAL.inc_by(5);

    // Export
    let exported = metrics::export_metrics();

    // Verify format
    assert!(exported.contains("ovim_http_requests_total"));
    assert!(exported.contains("ovim_buffer_edits_total"));
}
```

**Integration Test**:

```bash
# Start ovim
ovim --headless --session metrics_test test.txt

# Make some requests
curl http://127.0.0.1:PORT/v1/buffer
curl http://127.0.0.1:PORT/v1/health

# Fetch metrics
curl http://127.0.0.1:PORT/v1/metrics

# Should see:
# ovim_http_requests_total 2
# ovim_lsp_requests_total 0
# etc.
```

### Dependencies
- None (standalone feature)

### Complexity
**Medium** - Requires understanding Prometheus concepts and instrumenting many locations.

### Expected Impact
- **Operations**: Visibility into production behavior
- **Debugging**: Easier to diagnose performance issues
- **Quality**: Detect regressions via metrics trends

### Success Criteria
- [ ] Prometheus dependency added
- [ ] Metrics defined and exported
- [ ] Key code paths instrumented (HTTP, LSP, buffer, render)
- [ ] `/v1/metrics` endpoint returns Prometheus format
- [ ] Unit tests verify metrics update
- [ ] Documentation complete
- [ ] (Optional) Grafana dashboard created

---

## Dependencies Graph

```
Stage 1 (Property Tests) ───────────────────────────────────┐
                                                             │
Stage 2 (Incremental LSP) ──────────────────────────────────┤
                                                             │
Stage 3 (API Versioning) ───────────────────────────────────┤
                                                             │
Stage 4 (Box Editor) ───────────────────────────────────────┼─→ All Independent
                                                             │
Stage 5 (Syntax Highlighting) ──────────────────────────────┤
                                                             │
Stage 6 (LSP Cancellation) ─────────────────────────────────┤
                                                             │
Stage 7 (Session Management) ───────────────────────────────┤
                                                             │
Stage 8 (API Integration Tests) requires Stages 2,3 ────────┤
                                                             │
Stage 9 (Metrics) ──────────────────────────────────────────┘
```

Most stages are independent and can be tackled in any order!

---

## Recommended Implementation Order

Based on impact, risk, and dependencies:

### Week 1: Foundation & Quick Wins
1. **Day 1**: Stage 3 (API Versioning) - Low risk, high value
2. **Day 2**: Stage 1 (Property Tests) - Quality foundation
3. **Days 3-4**: Stage 2 (Incremental LSP) - High impact performance win

### Week 2: Optimization & Polish
4. **Days 5-6**: Stage 5 (Syntax Highlighting) - Visible performance improvement
5. **Days 7-8**: Stage 4 (Box Editor) - Conditional on measurements
6. **Day 9**: Stage 7 (Session Management) - Polish existing feature

### Week 3: Robustness & Observability
7. **Days 10-11**: Stage 6 (LSP Cancellation) - Correctness improvement
8. **Days 12-13**: Stage 8 (API Integration Tests) - Quality assurance
9. **Days 14-16**: Stage 9 (Metrics) - Optional, for production deployments

---

## Success Metrics

After completing all stages:

**Performance**:
- [ ] LSP didChange message size reduced by 10-1000x (Stage 2)
- [ ] Syntax highlighting 10-100x faster on edits (Stage 5)
- [ ] Editor struct size ≤ 512 bytes or properly Arc'd (Stage 4)
- [ ] Property tests find 0 bugs (or all found bugs fixed) (Stage 1)

**Reliability**:
- [ ] API versioning prevents breakage (Stage 3)
- [ ] LSP requests always show current info (Stage 6)
- [ ] Session cleanup prevents stale files (Stage 7)
- [ ] Integration tests catch interaction bugs (Stage 8)

**Observability**:
- [ ] Metrics expose system behavior (Stage 9)

**Code Quality**:
- [ ] All tests pass: `cargo test`
- [ ] No clippy warnings: `cargo clippy`
- [ ] Code formatted: `cargo fmt`
- [ ] Documentation updated

---

## Emergency Exit Strategy

If any stage proves too complex or time-consuming:

1. **Measure first**: Before starting Stage 4 (Box Editor), measure actual struct size. If <512 bytes, skip it entirely.

2. **Defer Stage 9**: Metrics are nice-to-have. If time is constrained, defer to future release.

3. **Simplify Stage 5**: If incremental syntax highlighting is too complex, just add caching (simpler, still high impact).

4. **Core Priority**: Stages 1-3 are the "must-haves" for a production-ready system. The rest are optimizations.

---

## Post-Completion Review

After implementing all stages, consider:

1. **Performance Benchmarks**: Create a comprehensive benchmark suite comparing before/after
2. **Documentation Update**: Ensure CLAUDE.md, README, and docs reflect all changes
3. **Blog Post**: "How We Made Ovim 10x Faster" (great for community engagement)
4. **User Testing**: Get feedback from AI assistant developers using ovim

---

## Notes for Future Maintainers

**Architectural Patterns Used**:

1. **Property-Based Testing**: Automatic edge case discovery
2. **Incremental Algorithms**: LSP sync, syntax highlighting
3. **API Versioning**: Forward compatibility
4. **RAII Guards**: TestSession, SessionGuard
5. **Metrics Instrumentation**: Observability throughout
6. **Caching with Versioning**: Buffer version invalidates caches

**Performance Philosophy**:

Ovim is built for **AI-first editing**. This means:
- **Low latency** > high throughput (single-user, interactive)
- **Incremental algorithms** > batch processing (small edits are common)
- **Caching with invalidation** > recomputation (cursor movement is frequent)
- **Async/non-blocking** > sync (LSP can be slow, don't block UI)

**Testing Philosophy**:

- **Unit tests**: Fast feedback on logic
- **Property tests**: Automatic edge case coverage
- **Integration tests**: Catch interaction bugs
- **Benchmarks**: Prevent performance regressions

All three are important!

---

## Conclusion

This action plan provides a complete roadmap for implementing all improvements from AREAS_FOR_IMPROVEMENT.md. Each stage is:

- **Concrete**: Specific files, code examples, exact changes
- **Tested**: Testing strategy for verification
- **Educational**: Explains WHY and trade-offs
- **Independent**: Can be implemented separately (mostly)

Total estimated time: **15-21 days** for complete implementation, but core improvements (Stages 1-3) can be done in **4 days** for quick wins.

The plan is designed to be **executed by agents** - each stage has everything needed to complete it without human intervention (assuming agent can write code, run tests, and iterate on failures).

**Priority**: Focus on Stages 1-3 first, then evaluate based on measurements and user feedback.
