# LSP Performance Optimization Implementation Plan

## Executive Summary

This plan implements 5 performance optimizations incrementally, each tested and verified before proceeding. Total estimated time: 2-3 days. Expected performance improvement: **10-100x faster hover operations** in common cases.

---

## Implementation Order & Rationale

```
Phase 1: Foundation (Low Risk, High Value)
├─ Step 1: Add buffer versioning system
├─ Step 2: Fix lock contention in response handling
└─ Step 3: Reduce allocations in hover parsing

Phase 2: Major Wins (Medium Risk, Very High Value)
├─ Step 4: Implement hover cache
└─ Step 5: Conditional sleep optimization

Phase 3: Polish (Low Risk, Medium Value)
└─ Step 6: Comprehensive benchmarking & validation
```

**Rationale**: Start with low-risk infrastructure (versioning), then fix clear bugs (lock contention), finally add caching (highest impact).

---

## Phase 1: Foundation

### Step 1: Add Buffer Versioning System

**Why First**: This is required for the hover cache (Step 4), has zero risk, and provides useful infrastructure for future optimizations.

#### Dependencies
- None (standalone change)

#### Files to Modify
1. `/Users/adrian/Projects/ovim/src/buffer/mod.rs`
2. `/Users/adrian/Projects/ovim/src/editor/lsp_integration.rs` (testing only)

#### Detailed Implementation

**File: `/Users/adrian/Projects/ovim/src/buffer/mod.rs`**

Location: Add to `Buffer` struct (around line 50-100 where struct is defined)

```rust
// AFTER (add version field)
pub struct Buffer {
    rope: Rope,
    cursor: Cursor,
    file_path: Option<String>,
    /// Monotonically increasing version number, incremented on every edit
    /// Used for cache invalidation in LSP hover, completion, etc.
    version: usize,
    // ... other fields ...
}
```

Location: Update `Buffer::new()` (around line 200-250)

```rust
// AFTER
pub fn new() -> Self {
    Self {
        rope: Rope::new(),
        cursor: Cursor::new(),
        file_path: None,
        version: 0,  // Start at version 0
        // ... other initializations ...
    }
}
```

Location: Add public getter (around line 400-500, near other getters)

```rust
/// Returns the current version of this buffer.
/// The version increments on every edit operation (insert, delete, etc.)
/// and is used for cache invalidation.
pub fn version(&self) -> usize {
    self.version
}
```

Location: Increment version in all mutation methods

Search for these methods and add `self.version += 1;` at the start:
- `insert_char()` - likely around line 600-700
- `insert_str()` - likely around line 700-800
- `delete_char()` - likely around line 800-900
- `delete_range()` - likely around line 900-1000
- Any other methods that modify `self.rope`

**Example for `insert_char()`:**

```rust
// AFTER
pub fn insert_char(&mut self, ch: char) {
    self.version += 1;  // Increment version on edit

    let line = self.cursor.line();
    let col = self.cursor.col();
    // ... existing code ...
}
```

**Find all mutation methods:**
```bash
cd /Users/adrian/Projects/ovim
grep -n "pub fn.*(&mut self" src/buffer/mod.rs | grep -v "cursor\|mark_dirty\|set_"
```

#### Testing Strategy

**Test File: `/Users/adrian/Projects/ovim/tests/buffer_version_test.rs`** (new file)

```rust
use ovim::buffer::Buffer;

#[test]
fn test_buffer_version_increments_on_insert() {
    let mut buffer = Buffer::new();
    let initial_version = buffer.version();
    assert_eq!(initial_version, 0, "New buffer should start at version 0");

    buffer.insert_char('a');
    assert_eq!(buffer.version(), 1, "Version should increment after insert");

    buffer.insert_char('b');
    assert_eq!(buffer.version(), 2, "Version should increment again");
}

#[test]
fn test_buffer_version_increments_on_delete() {
    let mut buffer = Buffer::new();
    buffer.insert_char('a');
    let version_after_insert = buffer.version();

    buffer.delete_char();
    assert_eq!(buffer.version(), version_after_insert + 1, "Version should increment after delete");
}

#[test]
fn test_buffer_version_does_not_increment_on_read() {
    let mut buffer = Buffer::new();
    buffer.insert_char('a');
    let version = buffer.version();

    // Read-only operations should NOT increment version
    let _ = buffer.cursor();
    let _ = buffer.rope();
    let _ = buffer.version();

    assert_eq!(buffer.version(), version, "Version should not change on reads");
}
```

#### Success Criteria

- [ ] All tests pass: `cargo test buffer_version_test`
- [ ] No regressions: `cargo test` (full suite)
- [ ] Version increments on every edit
- [ ] Version does NOT increment on reads

---

### Step 2: Fix Lock Contention in Response Handling

**Why Second**: Clear bug, high impact, low risk. No dependencies on Step 1.

#### Dependencies
- None (standalone fix)

#### Files to Modify
1. `/Users/adrian/Projects/ovim/src/lsp/server.rs`

#### Detailed Implementation

**File: `/Users/adrian/Projects/ovim/src/lsp/server.rs`**

Location: Line 506-530 (inside the reader task's response handling)

```rust
// AFTER (extract from lock, then send)
if let Some(id) = msg.id {
    // Extract the PendingRequest from the map without holding lock during send
    let pending_req = {
        let mut pending = inner_clone.pending_requests.lock().await;
        pending.remove(&id)
    }; // Lock released immediately

    // Send response outside the lock scope
    if let Some(req) = pending_req {
        if let Some(error) = msg.error {
            let error_msg = format!("{} (code {})", error.message, error.code);
            let _ = req.sender.send(Err(anyhow!("LSP error: {}", error_msg)));
        } else if let Some(result) = msg.result {
            let _ = req.sender.send(Ok(result));
        } else {
            let _ = req.sender.send(Ok(Value::Null));
        }
    }
}
```

#### Success Criteria

- [ ] All existing tests pass: `cargo test`
- [ ] No deadlocks under concurrent load
- [ ] Lock hold time drops from ~50μs to ~5μs
- [ ] Hover still works correctly

---

### Step 3: Reduce Allocations in Hover Parsing

**Why Third**: Clear optimization, medium impact, low risk. No dependencies.

#### Dependencies
- None (standalone optimization)

#### Files to Modify
1. `/Users/adrian/Projects/ovim/src/lsp/mod.rs`

#### Detailed Implementation

**File: `/Users/adrian/Projects/ovim/src/lsp/mod.rs`**

Location: Line 1343-1375 (hover response parsing)

```rust
// AFTER (move result, optimize join)
let response: Option<lsp_types::Hover> = if result.is_null() {
    lsp_info!("LSP-HOVER", "Result is null");
    None
} else {
    // Move result instead of cloning (from_value consumes it)
    match serde_json::from_value(result) {
        Ok(hover) => {
            lsp_info!("LSP-HOVER", "Successfully parsed hover response");
            Some(hover)
        }
        Err(e) => {
            lsp_warn!("LSP-HOVER", "Failed to parse hover response: {}", e);
            None
        }
    }
};

// Extract text from hover response with optimized array handling
let hover_text = response.and_then(|hover| match hover.contents {
    lsp_types::HoverContents::Scalar(content) => Some(marked_string_to_text(content)),
    lsp_types::HoverContents::Array(mut contents) => {
        if contents.is_empty() {
            None
        } else if contents.len() == 1 {
            // Single item: no need to allocate a Vec and join
            Some(marked_string_to_text(contents.remove(0)))
        } else {
            // Multiple items: allocate and join
            let texts: Vec<String> = contents.into_iter().map(marked_string_to_text).collect();
            Some(texts.join("\n\n"))
        }
    }
    lsp_types::HoverContents::Markup(content) => Some(content.value),
});
```

#### Success Criteria

- [ ] All existing tests pass: `cargo test lsp_hover`
- [ ] Hover still displays correctly
- [ ] Allocation count drops (20-40% reduction)
- [ ] No panics or errors

---

## Phase 2: Major Wins

### Step 4: Implement Hover Cache

**Why Fourth**: Highest impact optimization (100-500x speedup), depends on Step 1 (versioning).

#### Dependencies
- **Step 1 complete**: Buffer versioning system must be in place

#### Files to Modify
1. `/Users/adrian/Projects/ovim/src/editor/lsp_state.rs` - Add cache struct
2. `/Users/adrian/Projects/ovim/src/editor/lsp_integration.rs` - Implement caching logic
3. `/Users/adrian/Projects/ovim/src/editor/mod.rs` - Invalidate cache on edits

#### Detailed Implementation

**File 1: `/Users/adrian/Projects/ovim/src/editor/lsp_state.rs`**

Add after `DocumentSyncState` struct:

```rust
/// Cache for LSP hover results to avoid redundant requests
#[derive(Debug, Clone)]
pub struct HoverCache {
    pub file_path: String,
    pub line: usize,
    pub col: usize,
    pub buffer_version: usize,
    pub hover_text: String,
    pub cached_at: std::time::Instant,
}

impl HoverCache {
    const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(60);

    pub fn is_valid(&self, file_path: &str, line: usize, col: usize, buffer_version: usize) -> bool {
        self.file_path == file_path
            && self.line == line
            && self.col == col
            && self.buffer_version == buffer_version
            && self.cached_at.elapsed() < Self::MAX_AGE
    }

    pub fn new(file_path: String, line: usize, col: usize, buffer_version: usize, hover_text: String) -> Self {
        Self {
            file_path,
            line,
            col,
            buffer_version,
            hover_text,
            cached_at: std::time::Instant::now(),
        }
    }
}
```

Update `LspState` struct to include:

```rust
/// Cached hover result to avoid redundant LSP requests
pub hover_cache: Option<HoverCache>,
```

**File 2: `/Users/adrian/Projects/ovim/src/editor/lsp_integration.rs`**

In `hover_impl()`, add cache check at the beginning:

```rust
// Check hover cache first
let cursor = self.buffer().cursor();
let buffer_version = self.buffer().version();

if let Some(ref cache) = self.lsp_state.hover_cache {
    if cache.is_valid(file_path, cursor.line(), cursor.col(), buffer_version) {
        crate::lsp_info!("LSP-HOVER", "Cache HIT");
        self.lsp_state.hover_info = Some(cache.hover_text.clone());
        self.lsp_state.hover_scroll = 0;
        self.lsp_state.hover_position = Some((cursor.line(), cursor.col()));
        self.mode = crate::mode::Mode::HoverPreview;
        self.mark_dirty();
        self.set_lsp_status(String::new());
        return Ok(true);
    }
}
```

After successful hover, cache the result:

```rust
// Cache the hover result
self.lsp_state.hover_cache = Some(HoverCache::new(
    file_path.to_string(),
    cursor.line(),
    cursor.col(),
    buffer_version,
    hover_text,
));
```

Add invalidation method:

```rust
pub fn invalidate_hover_cache(&mut self) {
    if self.lsp_state.hover_cache.is_some() {
        self.lsp_state.hover_cache = None;
    }
}
```

**File 3: `/Users/adrian/Projects/ovim/src/editor/mod.rs`**

Add `self.invalidate_hover_cache()` to all buffer mutation methods:
- `insert_char()`
- `delete_char()`
- `insert_newline()`
- `paste()`
- etc.

#### Success Criteria

- [ ] Cache hit test passes
- [ ] Cache invalidation test passes
- [ ] All existing hover tests pass
- [ ] Manual testing shows cache hits in logs
- [ ] Benchmark shows 100x+ speedup for cache hits

---

### Step 5: Conditional Sleep Optimization

**Why Fifth**: Medium-high impact, low risk, no dependencies.

#### Dependencies
- None (can be done independently)

#### Files to Modify
1. `/Users/adrian/Projects/ovim/src/editor/lsp_integration.rs`

#### Detailed Implementation

Modify `ensure_lsp_document_synced` to return whether it flushed:

```rust
async fn ensure_lsp_document_synced(&mut self) -> bool {
    // ... existing code ...

    if needs_did_open {
        // ... send didOpen ...
        return true;  // We sent didOpen
    }

    if needs_flush {
        // ... send didChange ...
        return true;  // We flushed changes
    }

    false  // No flush needed
}
```

Update all callers to conditionally sleep:

```rust
let did_flush = self.ensure_lsp_document_synced().await;
if did_flush {
    tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;
}
```

Update in:
- `hover_impl()`
- `goto_definition_impl()`
- `completion_impl()`
- All other LSP request methods

#### Success Criteria

- [ ] `ensure_lsp_document_synced()` returns bool
- [ ] All callers updated to conditional sleep
- [ ] All existing tests pass
- [ ] Manual testing shows fast hover on unmodified code
- [ ] No race conditions observed

---

## Phase 3: Validation

### Step 6: Comprehensive Benchmarking & Validation

Create benchmark suite and validation tests.

#### Files to Create
1. `/Users/adrian/Projects/ovim/benches/lsp_performance_bench.rs`
2. `/Users/adrian/Projects/ovim/tests/lsp_regression_test.rs`

Run comprehensive validation and measure improvements.

---

## Expected Performance Improvements

**Before Optimizations:**
- First hover: 20-50ms
- Repeated hover: 20-50ms (no cache)
- Hover after edit: 30-60ms (10ms sleep + didChange)

**After Optimizations:**
- First hover: 10-40ms (2ms sleep reduction)
- Repeated hover (cache hit): 0.1-1ms (**100-500x faster**)
- Hover after edit: 12-42ms (2ms sleep + cache miss)

---

## Commit Strategy

Each step should be a separate commit:

```bash
git commit -m "feat: add buffer versioning system for cache invalidation"
git commit -m "perf: reduce lock contention in LSP response handling"
git commit -m "perf: reduce allocations in hover response parsing"
git commit -m "feat: implement hover result caching with version-based invalidation"
git commit -m "perf: conditional sleep in ensure_lsp_document_synced"
git commit -m "test: add comprehensive LSP performance benchmarks"
```

---

## Timeline

- **Phase 1** (Steps 1-3): 4-6 hours
- **Phase 2** (Steps 4-5): 6-8 hours
- **Phase 3** (Step 6): 2-4 hours
- **Total**: 12-18 hours (1.5-2 days)
