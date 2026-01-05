# Ovim - Areas for Improvement

Based on architecture review (Rating: 8.5/10)

**Note**: The architecture review incorrectly stated that ovim lacks syntax highlighting. Ovim DOES support syntax highlighting. This has been corrected below.

## 🔴 High Priority (High Impact, Low-Medium Effort)

### 1. Box/Arc the Editor Struct
**Impact**: Medium | **Effort**: Low | **Est. Time**: 1 day

**Problem**: `Editor` struct is 3,128 bytes. Expensive to pass around, high stack cost.

**Current Issues**:
- Every function taking `Editor` by value = 3KB stack frame
- Recursive calls compound the issue
- Async tasks each allocate 3KB
- Thread spawning has 3KB overhead before work

**Solution**:
```rust
// src/main.rs
let editor = Arc::new(Mutex::new(Editor::new()));

// Now passing Arc<Mutex<Editor>> = 8 bytes (pointer size)
```

**Files to Change**:
- `src/main.rs` - Wrap Editor in Arc
- `src/server.rs` - Accept `Arc<Mutex<Editor>>`
- `src/ui/mod.rs` - Accept `Arc<Mutex<Editor>>`
- Update all function signatures taking Editor

**Expected Impact**: Reduced stack usage, faster function calls, better async performance

---

### 2. Incremental LSP Sync
**Impact**: High | **Effort**: Medium | **Est. Time**: 2 days

**Problem**: Sending full buffer on every change (could be 10MB for large files)

**Current Code** (`lsp/client.rs`):
```rust
pub async fn notify_did_change(&self, uri: Url, text: String) {
    self.notify("textDocument/didChange", json!({
        "textDocument": {"uri": uri},
        "contentChanges": [{
            "text": text  // FULL BUFFER (could be 10MB!)
        }]
    })).await;
}
```

**Better Approach**: Send only changed region
```rust
pub async fn notify_did_change(&self, uri: Url, range: Range, text: String) {
    self.notify("textDocument/didChange", json!({
        "textDocument": {"uri": uri, "version": self.version},
        "contentChanges": [{
            "range": range,  // Only the changed part
            "text": text
        }]
    })).await;
}
```

**Impact**:
- 10MB file + 1 char change: 10MB transfer → 1 byte transfer
- Faster LSP response times
- Less network/memory pressure

**Implementation**:
1. Track edits in `Buffer` (already have undo stack)
2. Convert edit to LSP range
3. Send incremental update

**Files**:
- `src/buffer/mod.rs` - Expose last edit
- `src/lsp/client.rs` - Use incremental sync
- `src/editor/mod.rs` - Pass edit info to LSP

---

### 3. Property-Based Tests for Rope Operations
**Impact**: Medium | **Effort**: Low | **Est. Time**: 1 day

**Problem**: Rope edge cases (empty buffer, delete past end, UTF-8 boundaries) are hard to enumerate

**Add to `tests/buffer_tests.rs`**:
```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn insert_then_delete_is_identity(text in ".*", pos in 0..1000usize) {
        let mut buffer = Buffer::new();
        buffer.insert(0, &text);

        let pos = pos % (buffer.len_chars() + 1);
        buffer.insert(pos, "X");
        buffer.delete(pos..pos+1);

        prop_assert_eq!(buffer.to_string(), text);
    }

    #[test]
    fn rope_never_panics(
        text in ".*",
        ops in prop::collection::vec(arb_buffer_op(), 0..100)
    ) {
        let mut buffer = Buffer::new();
        buffer.insert(0, &text);

        for op in ops {
            // Should never panic, even on invalid ops
            let _ = apply_op(&mut buffer, op);
        }
    }

    #[test]
    fn utf8_boundaries_respected(text in ".*") {
        let mut buffer = Buffer::new();
        buffer.insert(0, &text);

        // All operations should preserve valid UTF-8
        prop_assert!(buffer.to_string().is_valid_utf8());
    }
}

fn arb_buffer_op() -> impl Strategy<Value = BufferOp> {
    prop_oneof![
        (any::<usize>(), ".*").prop_map(|(pos, text)| BufferOp::Insert(pos, text)),
        any::<(usize, usize)>().prop_map(|(start, end)| BufferOp::Delete(start..end)),
    ]
}
```

**Why This Matters**: Property tests automatically find edge cases that manual tests miss

---

## 🟡 Medium Priority (Medium Impact, Medium Effort)

### 4. Request Cancellation for LSP
**Impact**: Low | **Effort**: Medium | **Est. Time**: 2 days

**Problem**: Fast typing causes hover/completion requests to pile up

**Current Behavior**:
1. User hovers over symbol A → request 1
2. User moves to symbol B → request 2
3. Request 1 completes (now stale) → shows wrong info

**Solution**: Cancel pending requests
```rust
pub async fn hover(&self, position: Position) -> Result<Option<Hover>> {
    // Cancel any pending hover requests
    self.cancel_requests_by_method("textDocument/hover");

    let id = self.next_request_id();
    // ... send new request
}

fn cancel_requests_by_method(&self, method: &str) {
    for entry in self.pending_requests.iter() {
        if entry.value().method == method {
            // Send $/cancelRequest notification
            self.notify("$/cancelRequest", json!({"id": entry.key()}));
            self.pending_requests.remove(entry.key());
        }
    }
}
```

**Files**: `src/lsp/client.rs`

---

### 5. Optimize Syntax Highlighting Performance
**Impact**: Medium | **Effort**: Medium | **Est. Time**: 2-3 days

**Current State**: Ovim has syntax highlighting (the review was incorrect)

**Potential Optimizations**:
1. **Incremental highlighting**: Use tree-sitter's edit API to re-highlight only changed regions
   ```rust
   // Instead of re-parsing entire file on each edit
   pub fn highlight(&mut self, buffer: &Buffer) -> Vec<HighlightedSpan> {
       let source = buffer.to_string();
       self.tree = Some(self.parser.parse(&source, self.tree.as_ref()));
       extract_highlights(self.tree.as_ref().unwrap())
   }

   // Use incremental edits
   pub fn edit(&mut self, edit: &Edit) {
       if let Some(tree) = &mut self.tree {
           tree.edit(&to_tree_sitter_edit(edit));
       }
   }
   ```

2. **Cache highlights**: Don't re-compute on every render if buffer unchanged

3. **Lazy highlighting**: Only highlight visible lines, not entire file

**Files to Check**:
- Current highlighting implementation
- Performance profiling to identify bottlenecks

---

### 6. Improve Session Management Robustness
**Impact**: Medium | **Effort**: Low | **Est. Time**: 1 day

**Current State**: Already excellent (atomic writes, PID verification)

**Additional Improvements**:

1. **Session cleanup on crash**: Add systemd service or cron job
   ```bash
   # ~/.local/bin/cleanup-ovim-sessions.sh
   #!/bin/bash
   ovim-ctl clean
   ```

2. **Session expiry**: Auto-remove sessions older than 7 days
   ```rust
   pub fn is_expired(&self) -> bool {
       let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
       now - self.started_at > 7 * 24 * 60 * 60  // 7 days
   }
   ```

3. **Better error messages**: When session file is corrupted
   ```rust
   match SessionInfo::load(path) {
       Err(e) if e.kind() == io::ErrorKind::InvalidData => {
           eprintln!("Session file corrupted. Removing: {}", path.display());
           fs::remove_file(path)?;
       }
       Err(e) => return Err(e),
       Ok(info) => info,
   }
   ```

**Files**: `src/session.rs`

---

### 7. REST API Versioning
**Impact**: Low | **Effort**: Low | **Est. Time**: 1 day

**Problem**: No API versioning (breaking changes affect all clients)

**Current**:
```rust
.route("/api/buffer", get(get_buffer))
.route("/api/insert", post(insert_text))
```

**Better**:
```rust
.route("/v1/buffer", get(get_buffer))
.route("/v1/insert", post(insert_text))

// When breaking changes needed
.route("/v2/buffer", get(get_buffer_v2))
```

**Or use header-based versioning**:
```rust
async fn version_middleware(req: Request, next: Next) -> Response {
    let version = req.headers()
        .get("X-Ovim-API-Version")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("v1");

    // Route to appropriate handler
    next.run(req).await
}
```

**Files**: `src/server.rs`

---

### 8. Add Integration Tests for REST API
**Impact**: Medium | **Effort**: Medium | **Est. Time**: 2 days

**Current State**: Unit tests exist, but full API flow not tested

**Add** (`tests/api_integration.rs`):
```rust
#[tokio::test]
async fn test_full_editing_flow() {
    // Start headless ovim
    let server = start_ovim_headless().await;

    // Create buffer
    let resp = client.post("/v1/buffer/new")
        .send()
        .await?;
    assert_eq!(resp.status(), 200);

    // Insert text
    let resp = client.post("/v1/insert")
        .json(&json!({"text": "Hello, world!"}))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);

    // Get buffer
    let resp = client.get("/v1/buffer").send().await?;
    let body: BufferResponse = resp.json().await?;
    assert_eq!(body.content, "Hello, world!");

    // Execute command
    let resp = client.post("/v1/command")
        .json(&json!({"command": ":%s/world/Rust/"}))
        .send()
        .await?;

    let resp = client.get("/v1/buffer").send().await?;
    let body: BufferResponse = resp.json().await?;
    assert_eq!(body.content, "Hello, Rust!");
}
```

---

## 🟢 Low Priority (Nice to Have)

### 9. Add Metrics/Observability
**Impact**: Low | **Effort**: Medium | **Est. Time**: 2-3 days

**Add Prometheus metrics**:
```rust
use prometheus::{Counter, Histogram, Registry};

lazy_static! {
    static ref HTTP_REQUESTS: Counter = Counter::new(
        "ovim_http_requests_total",
        "Total HTTP requests"
    ).unwrap();

    static ref LSP_LATENCY: Histogram = Histogram::new(
        "ovim_lsp_latency_seconds",
        "LSP request latency"
    ).unwrap();
}

// In handlers
HTTP_REQUESTS.inc();
let timer = LSP_LATENCY.start_timer();
// ... do work
timer.observe_duration();
```

**Expose metrics endpoint**:
```rust
.route("/metrics", get(prometheus_metrics))
```

**Use cases**:
- Monitor LSP performance
- Track API usage
- Alert on errors

---

### 10. Add Vim Script Support
**Impact**: Low | **Effort**: High | **Est. Time**: 2+ weeks

**Goal**: Execute Vim scripts for configuration

**Challenges**:
- Vim script parser (complex language)
- Runtime environment (variables, functions)
- Integration with Rust code

**Alternative**: Support Lua like Neovim
```rust
use mlua::Lua;

pub fn execute_lua(&mut self, script: &str) -> Result<()> {
    let lua = Lua::new();
    lua.globals().set("buffer", self.buffer)?;
    lua.load(script).exec()?;
    Ok(())
}
```

---

### 11. WebAssembly Build Target
**Impact**: Low | **Effort**: Medium | **Est. Time**: 3-5 days

**Goal**: Run ovim in browser (ovim.wasm)

**Use Cases**:
- Online playground
- Browser-based editing
- Embed in web apps

**Challenges**:
- LSP in WASM (needs async runtime)
- File system access (use virtual FS)
- Terminal emulation in browser

**Starting Point**:
```bash
cargo build --target wasm32-unknown-unknown
wasm-pack build --target web
```

---

## 📊 Summary

| Priority | Count | Total Est. Time |
|----------|-------|-----------------|
| 🔴 High  | 3     | 4 days          |
| 🟡 Medium| 5     | 10-13 days      |
| 🟢 Low   | 3     | 7-13 days       |

**Quick Wins** (High Impact, Low Effort):
1. Box Editor struct - **1 day for better performance**
2. Property-based tests - **1 day for better quality**
3. Incremental LSP sync - **2 days for massive bandwidth savings**

**Path to 9/10**: Complete High Priority items + improve syntax highlighting performance

---

## Notes

- **Session management is exceptional** - atomic writes, PID verification, signal handling
- **LSP client is production-grade** - proper debouncing, async throughout, graceful degradation
- **MUTEX_ANALYSIS.md is exemplary** - more projects should document concurrency this well
- **Headless REST API is the differentiator** - no other Vim clone has this
- **Syntax highlighting already works** - the architecture review was incorrect on this point

---

## Correction to Architecture Review

The jon-gjengset agent incorrectly stated that ovim lacks syntax highlighting and suggested adding tree-sitter. **This is not accurate** - ovim already has syntax highlighting support.

Future improvements to syntax highlighting should focus on:
- Performance optimization (incremental updates)
- Broader language support
- Customizable color schemes

Not on "adding" a feature that already exists.
