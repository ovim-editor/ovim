# Editor God Object Refactoring Plan

## Executive Summary

This document provides a comprehensive, phased plan to refactor the `Editor` struct and associated large files to resolve two critical issues:

1. **Borrow checker conflicts** in `search_manager.rs` requiring unnecessary `Search` clones
2. **God object antipattern** with 60+ fields in `Editor` struct
3. **Oversized files** exceeding project guidelines (3 files > 2K lines)

**Current State:**
- `Editor` struct: 60+ fields (lines 171-319 in `src/editor/mod.rs`)
- Large files: `lsp_integration.rs` (2989L), `lsp/mod.rs` (2834L), `buffer/mod.rs` (2026L)
- Borrow conflicts force cloning in search operations (lines 99-128 in `search_manager.rs`)

**Target State:**
- Domain contexts extract related fields (SearchContext, CommandContext, etc.)
- All files under 2K lines
- Zero-cost borrowing with no defensive clones
- Incremental, test-driven refactoring with each step compiling

---

## Phase 1: Extract Domain Contexts (Weeks 1-2)

**Goal:** Group related fields into logical contexts to reduce borrow conflicts and improve code organization.

### 1.1 Extract SearchContext (Priority: HIGH - resolves cloning issue)

**Problem:** Lines 99-128 in `search_manager.rs` clone `Search` to avoid borrow conflicts with `self.buffer()` calls.

**Solution:** Extract all search-related fields into `SearchContext`.

**Fields to extract from `Editor`:**
```rust
// Current fields (lines 205-214 in mod.rs):
search_buffer: String,
search_forward: bool,
current_search: Option<Search>,
search_start_pos: Option<(usize, usize)>,
visual_search_state: Option<VisualSearchState>,
```

**New struct:**
```rust
// src/editor/search_context.rs
pub struct SearchContext {
    buffer: String,
    forward: bool,
    current: Option<Search>,
    start_pos: Option<(usize, usize)>,
    visual_state: Option<VisualSearchState>,
}

impl SearchContext {
    pub fn new() -> Self { ... }

    // Migrate methods from search_manager.rs
    pub fn execute(&mut self, buffer: &Buffer, registers: &mut RegisterManager, options: &EditorOptions) { ... }
    pub fn next(&mut self, buffer: &mut Buffer) { ... }
    pub fn prev(&mut self, buffer: &mut Buffer) { ... }
    pub fn select_next(&mut self, buffer: &mut Buffer, mode: Mode) -> bool { ... }
    pub fn select_prev(&mut self, buffer: &mut Buffer, mode: Mode) -> bool { ... }
}
```

**Changes to `Editor`:**
```rust
// In src/editor/mod.rs:
pub struct Editor {
    // ... other fields ...
    search_context: SearchContext,
    // ... other fields ...
}
```

**Files to modify:**
1. Create `src/editor/search_context.rs` (new)
2. Update `src/editor/search_manager.rs` → delegate to `search_context`
3. Update `src/editor/mod.rs` → add `search_context` field
4. Update `src/editor/input.rs` (or wherever search input is handled)

**Testing strategy:**
- Run existing search tests: `cargo test search`
- Test `n`, `N`, `gn`, `gN`, `/`, `?` commands
- Verify no performance regression (should improve - no clones!)

**Expected outcome:**
- Eliminates cloning in `search_next()`/`search_prev()`
- Reduces `Editor` fields by 5
- Clean borrow: `self.search_context.next(&mut self.buffer_mut())`

---

### 1.2 Extract CommandContext

**Fields to extract:**
```rust
// Lines 199-204 in mod.rs:
command_line: String,
command_history: Vec<String>,
command_history_index: Option<usize>,
```

**New struct:**
```rust
// src/editor/command_context.rs
pub struct CommandContext {
    line: String,
    history: Vec<String>,
    history_index: Option<usize>,
}

impl CommandContext {
    pub fn new() -> Self { ... }

    // Migrate from command_history.rs
    pub fn append(&mut self, ch: char) { ... }
    pub fn backspace(&mut self) { ... }
    pub fn execute(&mut self) -> String { ... }
    pub fn history_prev(&mut self) { ... }
    pub fn history_next(&mut self) { ... }
}
```

**Files to modify:**
1. Create `src/editor/command_context.rs` (new)
2. Update `src/editor/command_history.rs` → delegate to context
3. Update `src/editor/mod.rs`

**Testing:** `cargo test command_history`

---

### 1.3 Extract InputContext

**Fields to extract:**
```rust
// Lines 182-189, 226-232 in mod.rs:
count: Option<usize>,
pending_operator: Option<Operator>,
pending_command: Option<char>,
pending_register: Option<char>,
leader_key: char,
pending_leader: bool,
input_state: InputState,
last_find: Option<(char, FindType, FindDirection)>,
```

**New struct:**
```rust
// src/editor/input_context.rs
pub struct InputContext {
    pub count: Option<usize>,
    pub pending_operator: Option<Operator>,
    pub pending_command: Option<char>,
    pub pending_register: Option<char>,
    pub leader_key: char,
    pub pending_leader: bool,
    pub state: InputState,
    pub last_find: Option<(char, FindType, FindDirection)>,
}

impl InputContext {
    pub fn reset(&mut self) { ... }
    pub fn effective_count(&self) -> usize { self.count.unwrap_or(1) }
    // ... other input management methods ...
}
```

**Files to modify:**
1. Create `src/editor/input_context.rs` (new)
2. Update all files accessing these fields (32 references found earlier)
3. Update `src/editor/mod.rs`

**Testing:** Run full operator/motion test suite

---

### 1.4 Extract VisualContext

**Fields to extract:**
```rust
// Lines 193-198 in mod.rs:
visual_start: Option<(usize, usize)>,
visual_block_insert_state: Option<(usize, usize, usize, bool, bool)>,
last_visual_selection: Option<VisualSelection>,
```

**New struct:**
```rust
// src/editor/visual_context.rs
pub struct VisualContext {
    pub start: Option<(usize, usize)>,
    pub block_insert_state: Option<(usize, usize, usize, bool, bool)>,
    pub last_selection: Option<VisualSelection>,
}

impl VisualContext {
    pub fn start_visual(&mut self, line: usize, col: usize) { ... }
    pub fn save_selection(&mut self, start: (usize, usize), end: (usize, usize), mode: Mode) { ... }
    // Migrate from visual_mode.rs
}
```

**Files to modify:**
1. Create `src/editor/visual_context.rs` (new)
2. Update `src/editor/visual_mode.rs`
3. Update `src/editor/mod.rs`

**Testing:** `cargo test visual`

---

### 1.5 Extract PerformanceMetrics

**Fields to extract:**
```rust
// Lines 289-312 in mod.rs (performance tracking):
render_count: u64,
last_render_duration_micros: Option<u64>,
last_syntax_duration_micros: Option<u64>,
render_dirty: bool,
skip_scroll_update: bool,
viewport_command_active: bool,
input_latency_samples: Vec<u64>,
last_lsp_serialize_micros: Option<u64>,
last_git_status_micros: Option<u64>,
last_fold_calc_micros: Option<u64>,
last_diagnostic_query_micros: Option<u64>,
```

**New struct:**
```rust
// src/editor/performance_context.rs
pub struct PerformanceMetrics {
    pub render_count: u64,
    pub last_render_duration_micros: Option<u64>,
    pub last_syntax_duration_micros: Option<u64>,
    pub render_dirty: bool,
    pub skip_scroll_update: bool,
    pub viewport_command_active: bool,
    pub input_latency_samples: Vec<u64>,
    pub last_lsp_serialize_micros: Option<u64>,
    pub last_git_status_micros: Option<u64>,
    pub last_fold_calc_micros: Option<u64>,
    pub last_diagnostic_query_micros: Option<u64>,
}
```

**Files to modify:**
1. Create `src/editor/performance_context.rs` (new)
2. Update `src/editor/performance.rs` (merge or delegate)
3. Update `src/editor/mod.rs`

**Testing:** Check `/metrics` endpoint still works

---

### Phase 1 Summary

**Fields reduced:** 60 → ~35 (25 fields extracted)

**New structure:**
```rust
pub struct Editor {
    // Buffer management (keep as-is)
    pub buffers: Vec<Buffer>,
    current_buffer_index: usize,

    // Window/viewport (keep as-is)
    window_manager: Option<WindowManager>,
    tab_page_manager: TabPageManager,

    // Mode and quit state (keep as-is)
    mode: Mode,
    should_quit: bool,

    // Extracted contexts
    input_context: InputContext,
    search_context: SearchContext,
    command_context: CommandContext,
    visual_context: VisualContext,
    performance_metrics: PerformanceMetrics,

    // Subsystems (keep as-is)
    registers: RegisterManager,
    marks: MarkManager,
    keymaps: KeyMapManager,
    jump_list: JumpList,
    tag_stack: TagStack,
    macro_manager: MacroManager,
    lsp_state: LspState,

    // UI features (keep as-is)
    picker: Option<Picker>,
    completion_menu: CompletionMenu,
    file_tree: FileTree,
    quickfix_list: QuickfixList,
    location_list: LocationList,

    // Configuration (keep as-is)
    pub options: EditorOptions,
    color_scheme_registry: ColorSchemeRegistry,
    current_color_scheme: String,

    // Other (keep as-is for now)
    preview_cache: HashMap<String, PreviewCache>,
    lua_context: Option<LuaContext>,
    editor_bridge: Option<crate::lua::EditorBridge>,
    lsp_command_tx: Option<mpsc::UnboundedSender<LspCommand>>,
    lsp_command_rx: Option<mpsc::UnboundedReceiver<LspCommand>>,

    // ... remaining fields ...
}
```

---

## Phase 2: Split Large Files (Weeks 3-4)

**Goal:** Break files exceeding 2K lines into logical submodules.

### 2.1 Split `lsp_integration.rs` (2989 lines → 4 files ~750 lines each)

**Current structure:** Single `impl Editor` block with all LSP methods.

**New structure:**
```
src/editor/lsp/
├── mod.rs              (200 lines) - re-exports and shared types
├── initialization.rs   (400 lines) - LSP init, enable_lsp, document sync
├── actions.rs          (800 lines) - hover, goto_definition, completion, etc.
├── workspace.rs        (600 lines) - workspace edits, formatting, organize imports
└── diagnostics.rs      (500 lines) - diagnostic queries, navigation
```

**Split by functionality:**

**`lsp/initialization.rs`:**
- `enable_lsp()`
- `lsp_manager()`
- `lsp_command_sender()`
- `close_current_file_lsp()`
- `needs_lsp_init()`
- `request_lsp_init()`
- Document sync methods

**`lsp/actions.rs`:**
- `lsp_hover()`
- `lsp_goto_definition()`
- `lsp_goto_implementation()`
- `lsp_goto_type()`
- `lsp_completion()`
- `lsp_code_actions()`
- `lsp_find_references()`

**`lsp/workspace.rs`:**
- `lsp_format_document()`
- `lsp_organize_imports()`
- `apply_workspace_edit()`
- Rename methods

**`lsp/diagnostics.rs`:**
- `get_current_file_diagnostics()`
- `update_diagnostics()`
- `goto_next_diagnostic()`
- `goto_prev_diagnostic()`

**Migration steps:**
1. Create `src/editor/lsp/` directory
2. Move methods to appropriate files with `impl Editor` blocks
3. Update `src/editor/mod.rs` to include new modules
4. Run `cargo test --lib` after each file split

**Testing:** Full LSP integration tests

---

### 2.2 Split `lsp/mod.rs` (2834 lines → 3 files ~950 lines each)

**Current structure:** Monolithic `LspManager` with all functionality.

**New structure:**
```
src/lsp/
├── mod.rs              (300 lines) - LspManager struct, shared types
├── manager.rs          (1000 lines) - server lifecycle, requests
├── notifications.rs    (800 lines) - notification handling, debouncing
└── introspection.rs    (400 lines) - health checks, server info
```

**Split by concern:**

**`manager.rs`:**
- `LspManager` core struct
- `start_server()`
- `get_server()`
- `send_request()`
- Request methods (hover, definition, etc.)

**`notifications.rs`:**
- `did_open()`
- `did_change()`
- `did_save()`
- `did_close()`
- `ChangeDebouncer` struct
- Notification listener

**`introspection.rs`:**
- `get_server_info()`
- `get_health()`
- `get_diagnostics()`

**Migration steps:**
1. Keep `LspManager` struct in `mod.rs`
2. Split `impl LspManager` blocks across files
3. Move `ChangeDebouncer` to `notifications.rs`
4. Test with `cargo test lsp`

---

### 2.3 Split `buffer/mod.rs` (2026 lines → 3 files ~675 lines each)

**Current structure:** Monolithic `Buffer` struct with all text operations.

**New structure:**
```
src/buffer/
├── mod.rs              (300 lines) - Buffer struct, basic ops
├── text_ops.rs         (800 lines) - insert, delete, replace
├── syntax.rs           (600 lines) - highlighting, tree-sitter
└── git_integration.rs  (300 lines) - git status, blame
```

**Split by concern:**

**`mod.rs`:**
- `Buffer` struct definition
- Constructor methods
- File I/O (load, save)
- Getters (line, cursor, etc.)

**`text_ops.rs`:**
- `insert_char()`
- `delete_range()`
- `replace_range()`
- `indent_line()`
- Text manipulation methods

**`syntax.rs`:**
- `set_language()`
- `rebuild_highlight_cache()`
- `get_highlights_for_line()`
- Tree-sitter integration

**`git_integration.rs`:**
- `refresh_git_status()`
- Git-related queries

**Migration steps:**
1. Keep `Buffer` struct in `mod.rs`
2. Move impl blocks to specialized files
3. Re-export from `mod.rs` for public API compatibility
4. Test with `cargo test buffer`

---

### Phase 2 Summary

**Files affected:**
- `src/editor/lsp_integration.rs` (2989L) → 4 files (~750L each)
- `src/lsp/mod.rs` (2834L) → 3 files (~950L each)
- `src/buffer/mod.rs` (2026L) → 3 files (~675L each)

**Total:** 3 large files → 10 manageable files

All files now under 1K lines, well below 2K threshold.

---

## Phase 3: Further Editor Refinement (Week 5)

**Goal:** Extract remaining subsystems if `Editor` still feels bloated.

### 3.1 Extract ViewportContext (Optional)

**Fields:**
```rust
viewport_height: usize,
scroll_offset: usize,
last_picker_query_change: Option<std::time::Instant>,
last_picker_selection_change: Option<std::time::Instant>,
loading_preview: Option<String>,
pub last_shown_preview: Option<String>,
```

**Decision criteria:** Only extract if viewport logic causes borrow conflicts.

---

### 3.2 Extract SubstituteContext (Optional)

**Fields:**
```rust
substitute_matches: Vec<(usize, usize, usize, String)>,
substitute_match_index: usize,
substitute_pattern: Option<regex::Regex>,
```

**Rationale:** Substitute confirmation is a distinct feature with isolated state.

---

### 3.3 Box Large Subsystems (If Needed)

If `Editor` struct size still exceeds 10KB after Phase 1-2:

```rust
pub struct Editor {
    // Box large fields to reduce stack footprint
    lsp_state: Box<LspState>,
    tab_page_manager: Box<TabPageManager>,
    preview_cache: Box<HashMap<String, PreviewCache>>,
    // ... etc ...
}
```

**Note:** From size tests, `Editor` is already passed by reference everywhere, so this is low priority.

---

## Testing Strategy

### Per-Phase Testing

**Phase 1 (Contexts):**
```bash
# After each context extraction:
cargo test --lib                    # Unit tests
cargo test search                   # Search-specific
cargo test command_history          # Command-specific
cargo test visual                   # Visual mode
cargo clippy                        # Lints
cargo build --release              # Full build
```

**Phase 2 (File splits):**
```bash
# After each file split:
cargo test lsp                      # LSP tests
cargo test buffer                   # Buffer tests
cargo test --lib                    # All unit tests
./target/release/ovim test.rs --headless  # Integration test
```

**Phase 3 (Refinement):**
```bash
# Measure struct size:
cargo test measure_editor_size -- --nocapture
cargo test editor_size_regression

# Performance check:
./generate-benchmarks.sh
```

### Integration Testing

After each phase completes:
```bash
# Full test suite:
cargo test

# Headless mode test:
./target/release/ovim test.txt --headless --session test &
./target/release/ovim send test "iHello World\e"
./target/release/ovim buffer test
./target/release/ovim kill test

# LSP integration:
./target/release/ovim src/main.rs --headless --session dev &
./target/release/ovim send dev "K"  # Hover
./target/release/ovim lsp-status dev
./target/release/ovim kill dev
```

---

## Risk Assessment

### High Risk Items

1. **Borrow checker cascades** when extracting contexts
   - **Mitigation:** Extract one context at a time, run tests after each
   - **Fallback:** Use `Arc<RefCell<SearchContext>>` if borrow checker won't cooperate

2. **Breaking public API** when splitting files
   - **Mitigation:** Use `pub use` re-exports to maintain API surface
   - **Verification:** Check no external crates import internal modules

3. **Performance regression** from extra indirection
   - **Mitigation:** Benchmark before/after with `generate-benchmarks.sh`
   - **Acceptance criteria:** <5% performance change

### Medium Risk Items

1. **LSP state machine complexity** when splitting `lsp_integration.rs`
   - **Mitigation:** Keep LSP state machine logic intact, only move methods

2. **Merge conflicts** if active development continues during refactoring
   - **Mitigation:** Work in a dedicated `refactor/editor-god-object` branch
   - **Communication:** Announce refactoring window to team

### Low Risk Items

1. **Test failures** from import path changes
   - **Mitigation:** Fix imports immediately, don't accumulate

2. **Documentation drift**
   - **Mitigation:** Update `ARCHITECTURE.md` and inline docs as you go

---

## Success Criteria

### Phase 1 Complete
- [ ] `Editor` struct has ≤40 fields (down from 60+)
- [ ] Search operations use zero-cost borrows (no clones)
- [ ] All existing tests pass
- [ ] No performance regression (benchmark suite)

### Phase 2 Complete
- [ ] All files ≤1500 lines (well below 2K guideline)
- [ ] `lsp_integration.rs` → 4 files
- [ ] `lsp/mod.rs` → 3 files
- [ ] `buffer/mod.rs` → 3 files
- [ ] All tests pass
- [ ] Documentation updated

### Phase 3 Complete (If Needed)
- [ ] `Editor` struct size ≤8KB (current: likely >10KB)
- [ ] All borrow conflicts resolved
- [ ] Code review approval

---

## Rollout Plan

### Week 1: Phase 1.1-1.2
- Extract `SearchContext` (resolves cloning issue)
- Extract `CommandContext`
- Run full test suite

### Week 2: Phase 1.3-1.5
- Extract `InputContext`
- Extract `VisualContext`
- Extract `PerformanceMetrics`
- Benchmark and validate

### Week 3: Phase 2.1-2.2
- Split `lsp_integration.rs` → 4 files
- Split `lsp/mod.rs` → 3 files
- Integration testing

### Week 4: Phase 2.3
- Split `buffer/mod.rs` → 3 files
- Full regression testing
- Update documentation

### Week 5: Phase 3 (Buffer)
- Evaluate if further extraction needed
- Address any remaining issues
- Final code review and merge

---

## Notes and Caveats

### Why Not Arc<Mutex<Editor>>?

From size tests (lines 1139-1274 in `mod.rs`), `Editor` is **always passed by reference**, never by value. The god object antipattern is about code organization, not runtime overhead.

**Stack usage is one-time cost:**
- `Editor` allocated once in `main()`
- All functions take `&mut Editor`
- No moves, no copies

**Therefore:** Focus refactoring on **code clarity and borrow checker ergonomics**, not performance.

### Borrow Checker Philosophy

The borrow checker is telling us something: when we can't borrow disjoint fields simultaneously, they shouldn't be in the same struct. This refactoring is about **listening to the compiler**.

### Incremental Approach

Each step must:
1. Compile successfully
2. Pass all existing tests
3. Be committable to version control
4. Be reversible if issues arise

**No big bang rewrites.** This is methodical, test-driven refactoring.

---

## Appendix: Quick Reference

### Commands for Testing Each Phase

```bash
# Phase 1 (Contexts)
cargo test search_context
cargo test command_context
cargo test input_context
cargo test visual_context

# Phase 2 (File splits)
cargo test lsp::
cargo test buffer::
cargo test editor::lsp::

# Performance
cargo test measure_editor_size -- --nocapture
./generate-benchmarks.sh

# Integration
./target/release/ovim --version
./target/release/ovim test.rs --headless --session test &
./target/release/ovim kill test
```

### Files Changed Per Phase

**Phase 1:**
- `src/editor/mod.rs` (struct definition)
- `src/editor/{search,command,input,visual,performance}_context.rs` (new)
- `src/editor/{search,command}_manager.rs` (delegate to contexts)
- All editor impl files (update field accesses)

**Phase 2:**
- `src/editor/lsp_integration.rs` → `src/editor/lsp/{mod,initialization,actions,workspace,diagnostics}.rs`
- `src/lsp/mod.rs` → `src/lsp/{mod,manager,notifications,introspection}.rs`
- `src/buffer/mod.rs` → `src/buffer/{mod,text_ops,syntax,git_integration}.rs`

**Phase 3:** TBD based on results of Phase 1-2

---

## Conclusion

This plan transforms a 60-field god object into a well-organized, domain-driven structure while keeping files manageable (<2K lines). The borrow checker will thank us, and future contributors will understand the codebase faster.

**Estimated effort:** 5 weeks with careful, test-driven approach.

**Expected benefits:**
- Zero-cost abstractions (no cloning)
- Clearer separation of concerns
- Easier to navigate codebase
- Follows project guidelines (files <2K lines)
- Better compiler error messages

Let's do this methodically. Jon Gjengset would approve.
