# Knowledge Graph — Implementation Roadmap

Phased implementation plan. Each phase ends with something testable.

## Phase 1: Storage + Extraction

**Goal:** SQLite DB exists, tree-sitter export extraction works for Rust
and TypeScript, data round-trips through the DB. No LLM calls yet —
summaries are empty strings. This validates the data model and the
tree-sitter query approach.

### 1.1 Create the knowledge module

**New files:**
- `ovim-core/src/knowledge/mod.rs`
- `ovim-core/src/knowledge/db.rs`

Add `rusqlite` to `ovim-core/Cargo.toml` (with `bundled` feature for
zero-config SQLite).

Implement `KnowledgeDb`:
- `open(project_root: &Path) -> Result<Self>`
- Project ID: `<dirname>-<sha256_prefix_8>` from canonicalized root path
- DB location: `cache_dir/knowledge/<project_id>/knowledge.db`
- Connection setup: WAL mode, busy_timeout=5000, synchronous=NORMAL
- Schema creation with `IF NOT EXISTS`
- `upsert_file(path, hash, summary, byte_size, index_state)`
- `upsert_symbol(file_path, name, kind, signature, summary, body_byte_size)`
- `upsert_callers(symbol_file, symbol_name, callers: &[CallerRecord])`
- `get_file(path) -> Option<FileRecord>`
- `get_symbols(file_path) -> Vec<SymbolRecord>`
- `get_callers(symbol_file, symbol_name) -> Vec<CallerRecord>`
- `needs_update(path, current_hash) -> bool`
- `search_summaries(query) -> Vec<SearchResult>` (LIKE-based for V1)
- `stats() -> DbStats`
- `clear()`

### 1.2 Tree-sitter export extraction

**New files:**
- `ovim-core/src/knowledge/extractor.rs`
- `ovim-core/src/knowledge/queries/rust.scm`
- `ovim-core/src/knowledge/queries/typescript.scm`

Implement `ExportExtractor`:
- Loads export queries via `include_str!()`
- `extract(language, tree: &Tree, source: &str) -> Vec<ExportedSymbol>`
- Extracts: name, kind, signature, doc_comment, byte_range, body_byte_size
- Returns empty vec for languages without an export query (graceful degradation)

Start with Rust and TypeScript. These cover the immediate development
use case and exercise both "visibility modifier" (Rust pub) and "export
statement" (TS export) patterns.

### 1.3 Rate limiter and safety primitives

**New file:** `ovim-core/src/knowledge/safety.rs`

Implement before any LLM calls exist — these are the foundation:

- `RateLimiter`: circuit breaker with configurable window (default 200
  calls per 10 minutes). `try_acquire() -> bool`. Trips permanently
  until reset. Logged when tripped.
- `should_attempt(path, db) -> bool`: checks `last_attempt_at` against
  cooldown (default 5 minutes). Returns false if too recent.
- `record_attempt(path, db)`: writes `last_attempt_at = now` to DB.
  Called BEFORE the LLM call, not after — so even crashes leave a
  cooldown marker.
- `LspAttemptTracker`: in-memory per-file counter, caps at 3 attempts
  per session for pass 2 LSP readiness checks.

Test the rate limiter exhaustively: window rollover, trip behavior,
reset, concurrent access patterns.

### 1.4 Size budgeting

**In `extractor.rs`:**

Implement `budget_exports(exports: Vec<ExportedSymbol>, config: &BudgetConfig) -> BudgetedExports`:
- Apply max_symbols cap (default 20)
- Apply max_input_bytes budget (default 8192)
- Truncate large symbol bodies (max_symbol_bytes, default 2048)
- Return structured result with included symbols + overflow count

### Verification

```bash
cargo test -p ovim-core knowledge  # Unit tests
```

- DB round-trip: write file + symbols, read back, verify
- Rust export extraction: parse a Rust source string, verify pub items found
- TypeScript export extraction: parse a TS source string, verify exports found
- Budget enforcement: verify truncation and cap behavior
- Concurrent DB access: two connections writing, verify no errors (WAL)

---

## Phase 2: LLM Summarization

**Goal:** The cheap model is called to produce summaries. The prompt
is constructed from extracted exports. Summaries are stored in the DB.

### 2.1 Summarizer

**New file:** `ovim-core/src/knowledge/summarizer.rs`

Implement `Summarizer`:
- `build_intrinsic_prompt(path, language, exports: &BudgetedExports) -> String`
- `parse_intrinsic_response(response: &str) -> Result<IntrinsicSummary>`
- Uses the configured AI profile (from `vim.ai.knowledge.profile`)
- Validates JSON response structure, retries once on parse failure
- Single-export files get merged file+symbol summary

### 2.2 Wire to AI provider

The summarizer needs to make API calls through the existing AI provider
infrastructure. It creates a request using the profile from
`vim.ai.knowledge.profile`, sends it through the same async path as
edit requests, and parses the response.

This should NOT go through the edit pipeline — it's a simple
completion request with no extraction/application step.

### 2.3 Integrate pass 1 end-to-end

**In `knowledge/mod.rs`:**

Implement `KnowledgeGraph::ensure_intrinsic(path) -> Result<FileInsight>`:
1. Check DB: if `needs_update(path, hash)` is false, return cached
2. Check `should_attempt(path)` — respect cooldown (safety layer 2)
3. Check `rate_limiter.try_acquire()` — respect circuit breaker (safety layer 1)
4. `record_attempt(path)` — BEFORE the LLM call, not after (safety layer 3)
5. Get parse tree from syntax highlighter (borrow, don't re-parse)
6. Extract exports via `ExportExtractor`
7. Budget the exports
8. Build prompt, call LLM
9. On success: parse response, store in DB with `index_state = "intrinsic"`
10. On failure: store tombstone with `index_state = "failed"` (safety layer 3)
11. Return the `FileInsight` (or cached data if LLM failed)

### Verification

- Integration test: mock LLM response, verify full pipeline stores correctly
- Prompt format test: verify prompt stays within token budget
- JSON parse test: valid and invalid LLM responses
- Single-export merge test: file with one export gets one combined summary

---

## Phase 3: `<Space>d` Panel

**Goal:** Pressing `<Space>d` in normal mode shows the knowledge panel
for the current file. This is the first user-visible feature.

### 3.1 Keybinding

**File:** `ovim-core/src/editor/input/leader.rs`

Add `'d'` case to `handle_first_leader_key`:
```rust
'd' => {
    editor.show_file_insight();
    editor.reset_input_state();
}
```

### 3.2 Panel rendering

**File:** `ovim/src/editor/mod.rs` (or wherever floating panels are handled)

Implement `show_file_insight()`:
1. Get current file path
2. Query `KnowledgeGraph::describe(path)`
3. If not indexed: trigger pass 1 (async), show "Indexing..." placeholder
4. Format as floating panel (reuse hover/diagnostic float infrastructure)
5. Display file summary, symbol list with summaries, caller context if available

Panel content structure:
```
{file_summary}

Exports:
  {symbol_signature}
    {symbol_summary}
  ...

Used in:
  {caller_file} - {caller_summary}
  ...
  ... and N more files (M total references)
```

### 3.3 Lua configuration hookup

**File:** `ovim-core/src/lua/ai_api.rs`

Add `vim.ai.knowledge` table to the Lua API. Read during sync cycle:
- `enabled`, `profile`, `auto_discover`, `show_summary`
- `discovery_budget`, `reference_depth`, `caller_budget`
- `max_input_bytes`, `max_symbol_bytes`, `max_symbols`

### 3.4 Builtin defaults

**File:** `ovim-core/src/lua/builtin.lua`

Add default knowledge config:
```lua
vim.ai.knowledge = {
    enabled = true,
    profile = "openai_fast",
    auto_discover = true,
    show_summary = false,
    discovery_budget = 100,
    reference_depth = 2,
    caller_budget = 15,
    max_input_bytes = 8192,
    max_symbol_bytes = 2048,
    max_symbols = 20,
}
```

### Verification

- Manual: open a Rust file, press `<Space>d`, see the panel
- Manual: open a TS file, press `<Space>d`, see the panel
- Manual: open an unsupported language file, see file-level summary only
- Panel closes on Esc or any key (same as hover)

---

## Phase 4: Background Crawl (Pass 2)

**Goal:** On file enter, a background crawl indexes nearby files via
LSP references. `<Space>d` shows caller context when available.

### 4.1 Crawler

**New file:** `ovim-core/src/knowledge/crawler.rs`

Implement `Crawler`:
- `crawl(start_path, config, lsp, cancel_token) -> Result<CrawlResult>`
- BFS with depth and budget limits
- Skips already-indexed files with current hashes
- Calls `ensure_intrinsic` for each new file
- Queries LSP for references, samples and stores callers

### 4.2 Cancellation

Use `tokio_util::sync::CancellationToken` (or equivalent).

- `on_file_enter`: cancel any active crawl, start new one
- `on_file_leave` / `on_buffer_switch`: cancel active crawl
- Crawl loop checks `cancel.is_cancelled()` before each LLM call and
  LSP request

### 4.3 Reference sampling

Implement `sample_references(refs, budget) -> Vec<Reference>`:
- Deduplicate by file
- Group by parent directory
- Round-robin across groups to maximize diversity
- Return sampled refs + total count

### 4.4 Auto-discover trigger wiring

Wire up the trigger with all safety layers:

- Trigger point: main editor buffer changes to a different file path
- NOT triggered by: edits, saves, floating panels, files read by crawl
- Deduplicated: same file as last trigger → skip
- Pass 1: immediate (one LLM call, bounded by cooldown + circuit breaker)
- Pass 2 crawl: debounced with 2s delay (only if user stays on the file)
- On buffer switch: cancel pending crawl, cancel debounce timer
- All LLM calls go through rate limiter and per-file cooldown

```
on_main_buffer_changed(new_path):
    if new_path == last_trigger: return             # dedup
    last_trigger = new_path
    ensure_intrinsic_if_needed(new_path)            # pass 1, immediate
    cancel_pending_crawl()
    schedule_crawl(new_path, delay=2s)              # pass 2, debounced
```

### 4.5 Safety integration tests

Dedicated tests for runaway prevention:
- Rate limiter trips after N calls, subsequent calls return false
- Per-file cooldown prevents re-indexing within window
- Tombstone stored on LLM failure, prevents retry until cooldown
- Hard loop bound terminates crawl even with broken budget
- Wall-clock timeout terminates crawl
- Debounce: rapid buffer switches cancel intermediate crawls

### 4.5 Caller summarization prompt

Implement the pass 2 summarizer prompt (one call per symbol with callers):
- Input: symbol info + sampled caller summaries + usage snippets
- Output: one-sentence usage description
- Store in callers table

### Verification

- Integration test: mock LSP references, verify crawl indexes correct files
- Budget test: crawl stops at budget limit
- Depth test: crawl stops at depth limit
- Cancellation test: crawl stops promptly on cancel
- Deduplication test: already-indexed files don't spend budget
- Sampling test: diverse directory representation in sampled refs

---

## Phase 5: Additional Languages + Polish

**Goal:** Export queries for more languages. Summary line mode.
Programmatic Lua API for agents.

### 5.1 More export queries

Add `exports.scm` for:
- Python (module-level defs, `__all__`)
- JavaScript (same as TypeScript minus type exports)
- Go (capitalized = exported)
- Java (public classes/methods)

Each new language: write query, add extraction test, verify with
real files.

### 5.2 `show_summary` mode

When `vim.ai.knowledge.show_summary = true`:
- Render a one-line breadcrumb above the buffer content
- Format: `{filename} -- {file_summary}`
- Update on file change or new knowledge availability

### 5.3 Programmatic Lua API

Wire up the query functions for agent use:
- `vim.ai.knowledge.describe(path?)`
- `vim.ai.knowledge.related(path?, opts?)`
- `vim.ai.knowledge.search(query)`
- `vim.ai.knowledge.refresh(path?)`
- `vim.ai.knowledge.stats()`

### 5.4 Session cleanup integration

Add knowledge DB cleanup to `ovim session cleanup --max-age N`:
- Remove knowledge DBs for projects not accessed in N days
- Use directory mtime for age detection

### Verification

- Manual: each new language extracts exports correctly
- Manual: `show_summary` displays and updates
- Integration: Lua API returns correct data from DB
- Cleanup test: old DBs are removed, recent DBs are kept

---

## Phase Summary

| Phase | Deliverable | Depends On |
|-------|-------------|------------|
| 1 | SQLite DB + tree-sitter extraction + safety primitives | — |
| 2 | LLM summarization pipeline (with safety wiring) | 1 |
| 3 | `<Space>d` panel + Lua config | 2 |
| 4 | Background crawl + trigger wiring + safety integration tests | 3 |
| 5 | More languages + polish + agent API | 4 |

```
Phase 1 (Storage + Extraction + Safety Primitives)
    |
Phase 2 (LLM Summarization + Safety Wiring)
    |
Phase 3 (<Space>d Panel)             <-- first user-visible feature
    |
Phase 4 (Background Crawl + Trigger Discipline)
    |
Phase 5 (Languages + Polish + Agent API)
```

Phases are sequential. Each builds on the previous and ends with
something testable. Phase 3 is the first user-visible milestone.

Safety primitives are built in Phase 1 (before any LLM calls exist)
and wired in Phase 2. This ensures there's never a state where LLM
calls can be made without all safety layers active.
