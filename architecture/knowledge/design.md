# Knowledge Graph

Incremental, attention-driven semantic index of the codebase. Built lazily
as the developer navigates files. Surfaces file and symbol descriptions
on demand (`<Space>d`) and provides a compressed codebase map for AI agents.

## Motivation

When you land in an unfamiliar file, you want to know: what is this and
why does it exist? When an AI agent needs to make a change, the hardest
part is figuring out what to read. The knowledge graph answers both by
maintaining one-sentence descriptions of files and their exported symbols,
linked by reference relationships.

The key insight: **developer attention is a relevance signal.** Files you
open are the files you care about. We index lazily from those entry points,
crawling outward along the reference graph with bounded budget. Over time,
this builds a semantic working set — rich context for the files that matter,
nothing wasted on dusty corners.

## Design Principles

1. **Cache, not source of truth.** The knowledge DB is derived from source
   code. It can be deleted and rebuilt. This means we can be aggressive
   with write strategies and relaxed about durability.

2. **Content-addressed.** Entries are keyed by content hash. If the file
   hasn't changed, the summary is still valid. No timestamp-based staleness.

3. **Bounded computation.** Every crawl has a discovery budget and reference
   depth. No unbounded graph traversals, no surprise API bills.

4. **Cancellable.** User navigates away → crawl cancels. Partial progress
   is kept (it's already in SQLite). Next visit continues where we left off.

5. **Non-blocking.** Pass 1 (intrinsic summary) is fast enough to feel
   synchronous. Pass 2 (caller context) runs in the background. The editor
   never stutters.

## Two-Pass Architecture

### Pass 1: Intrinsic Description

"What does this file do? What does it export?"

- **Input:** File content only (no external dependencies)
- **Method:** Tree-sitter export extraction → cheap LLM summarization
- **Cache key:** `(relative_path, content_hash)`
- **Invalidation:** Re-index when content hash changes on file enter
- **Cost:** One cheap model call per file, ~2-4K input tokens

Produces: file summary + per-symbol summaries.

### Pass 2: Relational Description

"Who uses this and why?"

- **Input:** LSP references for exported symbols + pass 1 summaries of callers
- **Method:** LSP `textDocument/references` → sample callers → cheap LLM call
- **Cache key:** `(symbol_file, symbol_name, set_of_caller_hashes)`
- **Invalidation:** When any caller's content hash changes (checked lazily)
- **Cost:** One LSP request per exported symbol + one LLM call per symbol
- **Requires:** LSP ready for the file's language

Pass 2 is deferred until LSP is ready. If LSP never becomes available
(no server configured, server crashed), pass 1 alone is still useful.

## Tree-Sitter Export Extraction

### Sharing the Parse Tree

The syntax highlighting system already parses every open file with
tree-sitter. The knowledge graph does NOT re-parse. Instead, it borrows
the existing parse tree and runs separate queries against it.

```
tree-sitter parse (owned by SyntaxHighlighter, already exists)
    |
    +-- highlights.scm queries  --> syntax highlighting (existing)
    |
    +-- exports.scm queries     --> knowledge graph extraction (new)
```

The highlighter exposes `pub fn tree() -> Option<&Tree>`. The knowledge
extractor takes a borrowed `&Tree` + language + source, runs its own
query, returns structured data. Zero parsing overhead. Zero coupling
between highlight queries and export queries.

### Export Query Files

Separate `.scm` files per language in `ovim-core/src/knowledge/queries/`.
These capture exported/public symbols — completely different node types
from highlighting.

**Rust (`exports.scm`):**
```scheme
;; Public functions
(function_item
  (visibility_modifier) @vis
  name: (identifier) @name
  parameters: (parameters) @params
  return_type: (_)? @return_type) @definition

;; Public structs
(struct_item
  (visibility_modifier) @vis
  name: (type_identifier) @name) @definition

;; Public enums
(enum_item
  (visibility_modifier) @vis
  name: (type_identifier) @name) @definition

;; Public trait definitions
(trait_item
  (visibility_modifier) @vis
  name: (type_identifier) @name) @definition

;; Public type aliases
(type_item
  (visibility_modifier) @vis
  name: (type_identifier) @name) @definition

;; Impl methods (pub fn inside impl blocks)
(impl_item
  type: (_) @impl_type
  body: (declaration_list
    (function_item
      (visibility_modifier) @vis
      name: (identifier) @name
      parameters: (parameters) @params
      return_type: (_)? @return_type) @definition))
```

**TypeScript/TSX (`exports.scm`):**
```scheme
;; Named exports: export function foo() {}
(export_statement
  declaration: [
    (function_declaration name: (identifier) @name) @definition
    (lexical_declaration
      (variable_declarator name: (identifier) @name)) @definition
    (type_alias_declaration name: (type_identifier) @name) @definition
    (interface_declaration name: (identifier) @name) @definition
    (class_declaration name: (identifier) @name) @definition
    (enum_declaration name: (identifier) @name) @definition
  ])

;; Re-exports: export { foo } or export { foo } from './bar'
(export_statement
  (export_clause
    (export_specifier name: (identifier) @name)))

;; Default exports: export default function() {}
(export_statement "default" @default
  declaration: (_) @definition)
```

**Python (`exports.scm`):**
```scheme
;; Module-level function definitions (Python has no export keyword;
;; convention is: non-underscore-prefixed = public)
(module
  (function_definition
    name: (identifier) @name
    parameters: (parameters) @params
    return_type: (_)? @return_type) @definition)

;; Module-level class definitions
(module
  (class_definition
    name: (identifier) @name) @definition)

;; __all__ assignments (explicit export list)
(module
  (expression_statement
    (assignment
      left: (identifier) @all_name
      right: (list (string) @export_name))))
```

**Graceful degradation:** Languages without an exports query get a
file-level summary only — no symbol breakdown. This is still useful.
New languages can be added incrementally.

### What Gets Sent to the Summarizer

The extraction pipeline:

1. Run exports query against the parse tree
2. For each captured symbol, extract:
   - Name and kind (function, type, class, etc.)
   - Signature (the declaration line(s), not the body)
   - Doc comment (if present, from preceding comment nodes)
   - Body byte size (for budgeting)
3. Apply size budget (see below)
4. Format as structured prompt → send to cheap model

**Size budgeting uses byte count, not line count.** Line count breaks on
minified files — a 50KB single-line JS file is just as huge as a 2000-line
file. Budget thresholds:

| Parameter | Default | Purpose |
|-----------|---------|---------|
| `max_input_bytes` | 8192 | Total bytes sent to summarizer per file |
| `max_symbol_bytes` | 2048 | Per-symbol body truncation threshold |
| `max_symbols` | 20 | Per-file symbol count cap |

When a file exceeds the input budget:
- Sort symbols by byte size ascending (small types/interfaces first — they're
  usually the most informative-per-byte)
- Include symbols until budget is reached
- For symbols exceeding `max_symbol_bytes`: keep signature + doc comment,
  truncate body to first ~256 bytes
- Remaining symbols collapsed: "...and 14 additional utility functions"

When a file has a single export (common for React components, Rust modules
with one pub struct), the file summary and symbol summary are merged into
one description. No redundant repetition.

### The Summarizer Prompt

**Pass 1 (intrinsic):**
```
File: {relative_path}
Language: {language}

Exported symbols:

1. function DatePicker(props: DatePickerProps): JSX.Element
   /** A controlled date input with calendar dropdown */
   {first ~256 bytes of body...}

2. type DatePickerProps = { value: Date; onChange: (d: Date) => void; locale?: string }

3. function formatDateForDisplay(date: Date, locale: string): string

---
Describe this file in one sentence, then describe each exported symbol
in one sentence. Respond in this exact JSON format:
{
  "file": "one sentence file description",
  "symbols": {
    "DatePicker": "one sentence",
    "DatePickerProps": "one sentence",
    "formatDateForDisplay": "one sentence"
  }
}
```

Structured output. No ambiguity. The model fills in slots.

**Pass 2 (callers):**
```
Symbol: DatePicker (from src/components/DatePicker.tsx)
Summary: "A controlled date input component with calendar dropdown"

Used in:
- src/pages/ProfilePage.tsx: "User profile editing page"
  {~5 lines around the call site}
- src/pages/EventForm.tsx: "Event creation form with date/time fields"
  {~5 lines around the call site}
- src/components/FilterBar.tsx: "Search filter toolbar"
  {~5 lines around the call site}

Describe how this symbol is used across these callers in one sentence.
```

## LSP Reference Handling

### The Problem Cases

LSP `textDocument/references` can return:
- **Nothing** — LSP not ready, or doesn't support it → skip pass 2
- **A handful** — common case, use all of them
- **Hundreds** — widely-used utility, core type, React context

### Sampling Strategy for Heavy References

When references exceed `caller_budget` (default 15):

1. **Deduplicate by file** — one reference per file
2. **Maximize directory diversity** — prefer references from different
   directories to get architectural breadth, not 10 usages from the
   same feature folder
3. **Store total count** — "Used in 200+ files" is itself useful information

The sampling algorithm:
```
refs = deduplicate_by_file(all_refs)
if len(refs) <= caller_budget:
    return refs

# Group by parent directory
groups = group_by(refs, |r| r.path.parent())
# Round-robin across groups
sampled = round_robin_take(groups, caller_budget)
return sampled
```

### Slow LSP / Cancellation

The crawl runs as a background task with a cancellation token. On file leave:

1. Cancel the token
2. Pending LLM calls and LSP requests check the token before starting
3. Everything already written to SQLite stays (partial progress)
4. New file entry triggers a fresh crawl

LSP readiness gating: don't attempt pass 2 until the language server
reports ready for the file's language. If LSP isn't ready after a timeout
(e.g., 10s), do pass 1 only and mark the file as "pass 2 pending" in the
DB. Next file enter retries.

```rust
enum IndexState {
    Intrinsic,              // Pass 1 complete, pass 2 not attempted
    IntrinsicPendingCallers, // Pass 1 complete, pass 2 attempted but LSP wasn't ready
    Complete,               // Both passes complete
}
```

## Storage

### Per-Project SQLite Database

Each project gets its own knowledge DB, stored in the user's cache directory:

```
~/.cache/ovim/knowledge/<project_id>/knowledge.db     # Linux
~/Library/Caches/ovim/knowledge/<project_id>/knowledge.db  # macOS
```

Where `project_id` is `<dir_name>-<hash_prefix>`:
```
myapp-a1b2c3d4/knowledge.db
ovim-e5f6g7h8/knowledge.db
```

The hash is derived from the **canonicalized working directory root**, not
the git repo. This is critical for worktrees: two worktrees of the same
repo on different branches have different file contents at the same
relative paths. Sharing a DB between them would cause constant thrashing
as each invalidates the other's summaries. Separate DBs, separate worlds.

**Project root detection:** Walk up from the current file looking for `.git`
(which is a directory for normal repos, a file for worktrees — doesn't matter,
we just want the directory containing it). For files outside any git repo,
use the file's parent directory.

**Paths in the DB are relative to the project root.** This keeps entries
short, readable, and independent of where the project lives on disk.

### Schema

```sql
CREATE TABLE files (
    path TEXT PRIMARY KEY,          -- relative to project root
    content_hash TEXT NOT NULL,
    file_summary TEXT,
    byte_size INTEGER NOT NULL,
    index_state TEXT NOT NULL DEFAULT 'intrinsic',  -- intrinsic | pending_callers | complete | failed
    indexed_at INTEGER NOT NULL,
    last_attempt_at INTEGER NOT NULL  -- for per-file cooldown (safety layer 2)
);

CREATE TABLE symbols (
    id INTEGER PRIMARY KEY,
    file_path TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,             -- function | type | class | constant | trait | enum | interface
    signature TEXT NOT NULL,
    summary TEXT,
    body_byte_size INTEGER NOT NULL,
    UNIQUE(file_path, name, kind)
);

CREATE TABLE callers (
    id INTEGER PRIMARY KEY,
    symbol_file TEXT NOT NULL,      -- file containing the symbol
    symbol_name TEXT NOT NULL,
    caller_file TEXT NOT NULL,      -- file containing the reference
    caller_file_hash TEXT NOT NULL, -- for staleness detection
    context_snippet TEXT,           -- ~5 lines around usage site
    summary TEXT,                   -- how the caller uses this symbol
    indexed_at INTEGER NOT NULL
);

CREATE INDEX idx_symbols_file ON symbols(file_path);
CREATE INDEX idx_callers_symbol ON callers(symbol_file, symbol_name);
CREATE INDEX idx_callers_caller ON callers(caller_file);
CREATE INDEX idx_files_hash ON files(content_hash);
```

### Concurrency

Multiple editor instances may access the same project DB simultaneously
(e.g., a TUI session and a headless agent session).

**SQLite WAL mode** handles this:
- Readers never block writers, writers never block readers
- Two concurrent writers: one waits (up to busy_timeout), then succeeds
- Write contention is minimal — the LLM call is the slow part (seconds),
  the DB write is microseconds

**Connection setup:**
```sql
PRAGMA journal_mode=WAL;
PRAGMA busy_timeout=5000;
PRAGMA synchronous=NORMAL;   -- safe with WAL for a cache DB
```

**Write strategy:** `INSERT OR REPLACE` keyed on the natural keys.
If two instances index the same file simultaneously, the second write
overwrites with identical data (same content → same hash → same summary).
One wasted LLM call, no correctness issue. Acceptable for V1; could add
an in-memory advisory lock per path later if this becomes measurable.

**Transaction granularity:** One file's worth of inserts per transaction
(file row + its symbol rows). Not the whole crawl batch. Keeps the write
lock held for microseconds, not seconds.

### Lifecycle

```lua
vim.ai.knowledge.stats()       -- DB size, file/symbol counts
vim.ai.knowledge.clear()       -- Drop current project's DB
vim.ai.knowledge.clear_all()   -- Drop all knowledge DBs
```

Age-based cleanup in session cleanup:
```bash
ovim session cleanup --max-age 30
# Also removes knowledge DBs for projects not accessed in 30 days
```

## The Crawl Algorithm

Incorporates all safety layers. Comments mark which safety layer
each check belongs to.

```
on_main_buffer_changed(new_path):
    if new_path == last_trigger_path: return      # [safety 6: dedup]
    last_trigger_path = new_path

    # Pass 1: immediate, one LLM call max
    ensure_intrinsic_if_needed(new_path)

    # Pass 2: debounced — only if user stays for 2s
    cancel_pending_crawl()
    schedule_crawl(new_path, delay=2s)


ensure_intrinsic_if_needed(path):
    hash = content_hash(path)
    if not needs_update(path, hash): return       # Hash match → cached
    if not should_attempt(path): return           # [safety 2: cooldown]
    if not rate_limiter.try_acquire(): return      # [safety 1: circuit breaker]

    record_attempt(path)                          # [safety 3: always record]
    exports = tree_sitter_extract_exports(path)
    result = cheap_llm_summarize(path, exports)

    if result.is_ok():
        store(path, hash, result.summary, exports, state="intrinsic")
    else:
        store(path, hash, summary=NULL, state="failed")  # [safety 3: tombstone]


crawl(start_path, cancel_token):
    deadline = now + 120s                         # [safety 5: wall-clock]
    budget = discovery_budget
    iterations = 0
    MAX_ITERATIONS = 500                          # [safety 4: hard bound]

    if not lsp_ready(language):
        mark_pending_callers(start_path)
        increment_lsp_attempt(start_path)         # [safety 7: LSP limit]
        return

    # Gather references for the entry file's exports
    queue = []
    for symbol in get_exports(start_path):
        if cancel.is_cancelled(): return
        if now > deadline: return                  # [safety 5]
        refs = lsp.find_references(start_path, symbol)
        sampled = sample_references(refs, caller_budget)
        store_callers(symbol, sampled)
        for ref in sampled:
            queue.push((ref.file, depth=1))

    # BFS outward from callers
    visited = {start_path}
    while queue and budget > 0:
        iterations += 1
        if iterations > MAX_ITERATIONS: break     # [safety 4]
        if cancel.is_cancelled(): return
        if now > deadline: break                   # [safety 5]

        (file, depth) = queue.pop_front()
        if depth > reference_depth: continue
        if file in visited: continue
        visited.add(file)

        file_hash = content_hash(file)
        if not needs_update(file, file_hash):
            continue                              # Skip, don't spend budget

        if not should_attempt(file): continue     # [safety 2: cooldown]
        if not rate_limiter.try_acquire(): break   # [safety 1: circuit breaker]

        record_attempt(file)                      # [safety 3]
        exports = tree_sitter_extract_exports(file)
        result = cheap_llm_summarize(file, exports)

        if result.is_ok():
            store(file, file_hash, result.summary, exports)
        else:
            store(file, file_hash, summary=NULL, state="failed")  # [safety 3]

        budget -= 1                               # AFTER the store, not before

        if depth < reference_depth:
            for symbol in exports:
                refs = lsp.find_references(file, symbol)
                for ref in sample_references(refs, caller_budget):
                    queue.push((ref.file, depth + 1))

    mark_complete(start_path)
```

Already-indexed files with current hashes are skipped without spending
budget. This means repeated file visits are nearly free, and multiple
editor instances filling in the same project complement each other
rather than duplicating work.

## Configuration

### Lua API

```lua
-- In init.lua
vim.ai.knowledge = {
    enabled = true,
    profile = "openai_fast",        -- AI profile for summarization calls
    auto_discover = true,           -- Crawl on file enter
    show_summary = false,           -- Persistent one-liner at buffer top

    discovery_budget = 100,         -- Max files to index per crawl
    reference_depth = 2,            -- Max hops outward from entry file
    caller_budget = 15,             -- Max callers to sample per symbol
    max_input_bytes = 8192,         -- Per-file summarization input budget
    max_symbol_bytes = 2048,        -- Per-symbol body truncation
    max_symbols = 20,               -- Per-file symbol count cap
}
```

### Programmatic Access

```lua
-- Get full description for current or specified file
vim.ai.knowledge.describe(path?)
-- Returns: { file = "...", symbols = { ... }, callers = { ... }, ref_count = N }

-- Get neighbor nodes with summaries
vim.ai.knowledge.related(path?, { depth = 1 })
-- Returns: { { path = "...", summary = "...", relation = "caller|dependency" }, ... }

-- Full-text search over summaries
vim.ai.knowledge.search(query)
-- Returns: { { path = "...", summary = "...", score = N }, ... }

-- Force re-index (ignores cache)
vim.ai.knowledge.refresh(path?)

-- Database statistics
vim.ai.knowledge.stats()
-- Returns: { files = N, symbols = N, callers = N, db_size_bytes = N }
```

## Keybinding: `<Space>d`

In normal mode, `<Space>d` opens a floating panel showing the knowledge
graph description for the current file:

```
+-  src/components/DatePicker.tsx  ----------------------------+
|                                                              |
| Date input component with calendar dropdown, supporting      |
| controlled value and locale-aware formatting.                |
|                                                              |
| Exports:                                                     |
|   DatePicker(props) -> JSX.Element                           |
|     Controlled date input with calendar popup                |
|                                                              |
|   DatePickerProps                                            |
|     Props type with value, onChange, and locale               |
|                                                              |
|   formatDateForDisplay(date, locale) -> string               |
|     Formats a Date for display using locale settings         |
|                                                              |
| Used in:                                                     |
|   ProfilePage.tsx - user profile editing form                |
|   EventForm.tsx - event creation with date fields            |
|   FilterBar.tsx - search filter toolbar                      |
|   ... and 11 more files (23 total references)                |
|                                                              |
+--------------------------------------------------------------+
```

If the file hasn't been indexed yet, pass 1 runs on demand (fast —
tree-sitter extraction is instant, one cheap LLM call takes ~1s).
Pass 2 data appears when available; the panel shows what we have.

### `show_summary` mode

When `vim.ai.knowledge.show_summary = true` in init.lua, a one-line
summary is shown persistently at the top of the buffer (like a
breadcrumb or winbar):

```
DatePicker.tsx -- Date input with calendar dropdown, used in navigation and forms
```

This updates when the file changes or when new knowledge becomes available.
Toggled by config flag, not by default — it's opt-in visual noise.

## Module Structure

```
ovim-core/src/knowledge/
+-- mod.rs              # KnowledgeGraph public API
+-- db.rs               # SQLite connection, schema, CRUD operations
+-- extractor.rs        # Tree-sitter export extraction (queries borrowed tree)
+-- summarizer.rs       # LLM prompt construction and response parsing
+-- crawler.rs          # Background crawl orchestration (BFS, budgeting, cancellation)
+-- queries/            # Export query files per language
|   +-- rust.scm
|   +-- typescript.scm
|   +-- python.scm
|   +-- javascript.scm
|   +-- go.scm
|   +-- (others added incrementally)
```

No dependency on the syntax highlighting pipeline beyond borrowing
the parse tree. The `extractor` compiles its own `Query` objects from
the exports.scm files (loaded via `include_str!()`, same pattern as
highlight queries).

## Use Cases

### 1. `<Space>d` Panel (V1)

Developer opens a file, presses `<Space>d`, sees what it does and where
it's used. Immediate value, justifies the infrastructure.

### 2. Agent Context Priming

AI agents query the knowledge graph before reading files:
```lua
local ctx = vim.ai.knowledge.describe("src/auth/login.tsx")
local related = vim.ai.knowledge.related("src/auth/login.tsx")
```
In ~500 tokens, the agent knows which files to actually open. Dramatically
more efficient than grep-based exploration.

### 3. Semantic Search

```lua
vim.ai.knowledge.search("authentication")
```
Full-text search over summaries. Returns files that are *about*
authentication, not just files containing the string "auth". Much
higher signal than grep for conceptual queries.

### 4. Change Impact Estimation

Query the callers table to see how many files depend on a symbol
before refactoring it. Surface this in `<Space>d` or as an on-demand
query for agents.

### 5. Smart File Suggestions

"You're editing UserPrefsApi.ts. Related files: useUserPrefs.ts (hook
wrapper), Settings.tsx (main consumer), prefsSchema.ts (validation)."
Falls out of the graph with zero additional infrastructure.

## Runaway Prevention (Safety)

The crawl makes LLM API calls that cost money. A bug in the hash check,
budget counter, or trigger logic could cause unbounded API calls. The
design relies on hash correctness and budget decrementing for normal
operation, but bugs happen. We need defense in depth — multiple
independent safety layers so no single bug causes a runaway.

### Threat Model

| Bug | Consequence | Without safety |
|-----|------------|----------------|
| `needs_update` always returns true (bad hash compare) | Every file enter re-indexes | `discovery_budget` LLM calls per buffer switch |
| LLM fails, no tombstone stored | Same file retried every visit | Infinite failed calls on flaky API |
| Budget doesn't decrement (early `continue` before `-= 1`) | BFS loop never terminates | Unbounded LLM calls until OOM or timeout |
| `<Space>d` panel triggers "file enter" event | Panel open → crawl → panel → crawl | Tight infinite loop |
| Pass 2 retries when LSP is perpetually "almost ready" | Repeated LSP request attempts | LSP request storm |
| Crawl reads file, read event re-triggers crawl | Self-triggering loop | Stack overflow or tight loop |

### Safety Layer 1: Circuit Breaker (Rate Limiter)

Hard cap on LLM calls per time window, per project. Independent of all
other logic — it counts calls, nothing else.

```
max_calls_per_window = 200    (configurable)
window_duration = 10 minutes
```

When tripped:
- All knowledge graph LLM calls stop immediately
- Warning logged: "Knowledge graph rate limit exceeded — disabled"
- Stays off until editor restart or explicit `:AI knowledge reset`
- `<Space>d` still works (reads from DB), just no new indexing

200 calls per 10 minutes is generous for normal use (heavy crawl of
100 files is 100 calls). A runaway loop hits this in seconds.

### Safety Layer 2: Per-File Cooldown

After ANY attempt to index a file (success or failure), record
`last_attempt_at`. Don't re-attempt the same file within 5 minutes,
regardless of what `needs_update` says.

```sql
-- Added to files table
last_attempt_at INTEGER
```

```
should_attempt(path) =
    last_attempt_at is NULL
    OR (now - last_attempt_at) > cooldown_duration
```

This catches: broken hash check, missing tombstone. Even if
`needs_update` is lying, each file can only be indexed once per
cooldown window.

### Safety Layer 3: Store Failures as Tombstones

On LLM failure (timeout, invalid response, provider error), ALWAYS
write to the DB:

```
upsert_file(path, hash, summary=NULL, index_state="failed", last_attempt_at=now)
```

A failed file with the current content hash + recent attempt timestamp
won't be retried until:
- Content hash changes (file was actually edited)
- Cooldown expires (5 minutes)
- User explicitly calls `vim.ai.knowledge.refresh()`

No silent retry loops. Every failure is visible in `knowledge.stats()`.

### Safety Layer 4: Hard Loop Bound on BFS

Independent of budget, cap the BFS iteration count:

```
MAX_ITERATIONS = 500   (absolute, not configurable)

iterations = 0
while queue and budget > 0:
    iterations += 1
    if iterations > MAX_ITERATIONS:
        log_warning("crawl hit iteration safety limit")
        break
    ...
```

Even if budget never decrements due to a bug, the loop terminates
after 500 iterations. 500 is far above any reasonable crawl
(budget default is 100) but prevents true infinite loops.

### Safety Layer 5: Wall-Clock Timeout

The crawl task has a hard deadline:

```
crawl_deadline = now + 120 seconds
```

Checked before each LLM call and LSP request. If exceeded, the crawl
stops and logs. Two minutes is generous — most crawls complete in
under 30 seconds.

### Safety Layer 6: Trigger Discipline

This is the most important safety layer because it prevents the
problem at the source. The question is: when do we index?

**Indexing triggers — the ONLY events that cause LLM calls:**

| Event | Triggers indexing? | Why / why not |
|-------|-------------------|---------------|
| Open a different file | YES | Core use case — new file entered |
| Switch to existing buffer | YES (with dedup) | Re-entering a file, but dedup prevents rapid switching loops |
| `:e` / picker / goto def | YES | Same as opening a different file |
| Editing the current file | NO | Every keystroke changes the hash — re-indexing would be insane |
| Saving the current file | NO | Saving doesn't change what the user is looking at |
| `<Space>d` pressed | Maybe pass 1 only | On-demand, not auto. Only if not already indexed with current hash |
| `vim.ai.knowledge.refresh()` | YES | Explicit user/agent request, bypasses cooldown |
| Floating panel opened | NO | Panels are not file navigation |
| File read by crawl | Handled by crawl budget | Already bounded by crawl safety layers |

**The critical rule: edits NEVER trigger re-indexing.**

A file you're actively editing has a changing hash on every keystroke.
Re-indexing on edit would mean: type a character → hash changes →
`needs_update` returns true → LLM call → type another character →
repeat. Even with the per-file cooldown (safety layer 2), you'd get
one call per 5 minutes while editing, which is wasteful and
produces descriptions of half-finished code.

Instead, re-indexing happens on the NEXT file enter after the file
was modified. The stale summary is fine — it describes the file as
of the last time you entered it, which is close enough until you
come back to it.

**When does the current file's index become stale?**

```
1. User opens foo.rs (hash A)     → index with hash A
2. User edits foo.rs              → hash changes to B, C, D...
3. User switches to bar.rs        → index bar.rs
4. User switches back to foo.rs   → hash is now Z
5. needs_update("foo.rs", Z)      → true (stored hash was A)
6. Re-index foo.rs with hash Z    → one LLM call
```

This is the right granularity: one re-index per file-visit after
modification. Not per-edit, not per-save, not per-keystroke.

**Deduplication for rapid buffer switching:**

Users sometimes rapidly switch between buffers (e.g., comparing two
files). Without dedup, switching A→B→A→B→A fires 5 crawl triggers.

```
on_main_buffer_changed(new_path):
    if new_path == last_knowledge_trigger_path:
        return    # Same file as last trigger, skip
    last_knowledge_trigger_path = new_path
    trigger_crawl(new_path)
```

This deduplicates consecutive visits to the same file. The A→B→A→B→A
pattern fires 5 triggers but only 2 unique crawls (one for A, one for
B). And if A and B are already indexed with current hashes, both crawls
are no-ops (just a hash check, no LLM call).

**Floating panels, hover, diagnostics:**

These must NOT count as "file enter." If the knowledge panel is
implemented as a buffer (some UI patterns do this), opening it would
trigger a crawl for... what file? The panel isn't a file. The trigger
must be scoped to **main editor buffer path changes only.**

**The near-runaway that's still possible:**

A user opens 50 files in quick succession (e.g., grep results,
clicking through search hits). Each is a legitimate file enter.
If none are indexed, that's 50 pass-1 LLM calls in rapid succession.
This is technically correct behavior, but might surprise the user.

Mitigations:
- The circuit breaker (safety layer 1) caps total calls
- Pass 1 for the current file runs on-demand; the crawl (which fans
  out to related files) is the expensive part and is cancelled on
  each file switch
- Consider: only auto-crawl if the user stays on a file for >2 seconds
  (debounce the crawl trigger, not the pass-1 trigger)

```
on_main_buffer_changed(new_path):
    # Pass 1: immediate (one LLM call, answers <Space>d)
    ensure_intrinsic_if_needed(new_path)

    # Pass 2 crawl: debounced (only if user stays for 2s)
    cancel_pending_crawl()
    schedule_crawl(new_path, delay=2s)
```

This way, rapidly clicking through search results only fires pass-1
calls (one per file, bounded by cooldown). The expensive crawl only
starts when the user settles on a file. This feels right — you don't
need caller context for a file you glanced at for 500ms.

### Safety Layer 7: LSP Attempt Limit for Pass 2

Don't retry LSP readiness indefinitely. Per-file, per-session:

```
max_lsp_attempts_per_file = 3
```

After 3 attempts to get LSP references for a file's symbols (across
file visits), stop trying for that file until the session restarts.
Prevents the "LSP never ready" storm.

### Summary of Safety Layers

| Layer | Protects Against | Scope |
|-------|-----------------|-------|
| Circuit breaker | Any runaway LLM calls | Per project, per 10min window |
| Per-file cooldown | Re-indexing the same file | Per file, 5min cooldown |
| Failure tombstones | Retry-on-failure loops | Per file, stored in DB |
| Hard loop bound | Budget counter bugs | Per crawl invocation |
| Wall-clock timeout | Slow/stuck crawls | Per crawl invocation |
| Narrow trigger | Self-triggering loops | Per editor instance |
| LSP attempt limit | LSP-never-ready storm | Per file, per session |

A single bug can defeat at most one layer. A runaway requires
multiple independent failures, which is the goal.

## Edge Cases and Mitigations

| Case | Strategy |
|------|----------|
| Huge file (>50KB) | Send export skeleton only (signatures + doc comments), body truncated |
| Huge symbol (>2KB body) | Signature + doc comment + first ~256 bytes of body |
| Many exports (>20) | Summarize top 20, collapse rest with counts by kind |
| Many references (>15) | Sample 15, deduplicate by file, maximize directory diversity |
| LSP not ready | Do pass 1 only, mark pass 2 pending, retry on next visit |
| LSP slow | Background crawl with cancellation, show what we have |
| User leaves file quickly | Cancel pending work, keep partial results |
| No export query for language | File-level summary only (no symbol breakdown) |
| Model returns invalid JSON | Validate structure, retry once, then store "summary unavailable" |
| Binary/generated file | Detect (null bytes, generated-file markers), skip with a marker |
| File outside any git repo | Use file's parent directory as project root |
| Minified file (huge single line) | Byte-based budgeting, not line-based |
| Single export | Merge file and symbol summary into one description |

## What's NOT in V1

- **Embedding-based semantic search** — full-text over summaries is enough
- **Symbol-level caller tracking** — V1 tracks at file level. Call hierarchy
  (which function in that file calls this symbol) is deferred.
- **Cross-project knowledge** — each project is isolated
- **Filesystem watcher for auto-invalidation** — re-check on file enter
- **Automatic `exports.scm` generation** — hand-written per language
- **Fuzzy/semantic search** — exact substring matching on summaries for V1
