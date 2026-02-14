# Harness Implementation Roadmap

From the current codebase to the full harness described in `builtin-sketch.lua`.
Each phase builds on the previous one and ends with a shippable state.

This is greenfield — ovim hasn't been released yet. No migration concerns.

## Gap Analysis

What `builtin-sketch.lua` describes vs what the codebase has today:

| Feature | Sketch | Codebase | Gap |
|---------|--------|----------|-----|
| `api_key` (named key) | `api_key = "openai"` | `api_key_env = "OVIM_..."` | New field + registry |
| `vim.api_keys` registry | Full API | Nothing | New Lua API |
| `vim.ai.prompts` table | Convention-based lookup | Hardcoded in Rust | New Lua API |
| `vim.ai.formats.register()` | Extensible format system | Nothing | New Lua API |
| `vim.ai.context_policies` | Plain tables | `ContextPolicy` struct | Replace struct with tables |
| `vim.ai.chat` | Observation masking config | Nothing | New Lua API + serializer |
| `vim.ai.agent` | Safety rail config | Inside ContextPolicy | Extract to own config |
| `builtin.lua` via include_str! | Runs before init.lua | No builtin file | New mechanism |
| `EditFormat` enum | Codeblock/Json/Raw/ApplyPatch/StrReplace/Lua | `ExtractionStrategy` enum | Replace enum |
| `ContextGatheringPolicy` | Flat table with budget, symbols, etc | `ContextPolicy` bundle | Split + simplify |
| `edit_prompt` / `chat_prompt` | Per-profile prompt overrides | `system_prompt` only | New fields |
| `chat_edit_format` | Per-profile field | Not on struct | New field |
| `context` as table | Inline on profile | `ContextPolicy` struct | New field shape |
| `reasoning_effort` | OpenAI param | Not on struct | New field |
| `verbosity` | OpenAI 5.2+ param | Not on struct | New field |
| `syntax_check` | Boolean flag | Nothing | New field + engine |
| `retry` | `{ max, fallback }` | Nothing | New field + protocol |
| `edit_format` as Lua fn | `function(response)` | String only | New variant |
| `new_import_statements` | In JSON prompt | `top_insertions` | Rename |
| apply_patch parser | Engine spec | Nothing | New parser |
| str_replace parser | Engine spec | Nothing | New parser |
| hashline format | Lua-implemented format | Nothing | New Lua format |
| Elision detection | Regex patterns | Nothing | New check |
| Project context files | .ovim.md / AGENTS.md / CLAUDE.md | Nothing | New loader + injection |
| Prompt resolution chain | 4-layer: profile → prompts → format → Rust | profile → Rust | New resolution |

### Types to Remove

These current types are replaced by the new design:

| Current Type | Replacement |
|-------------|-------------|
| `CapabilityTier` | `vim.ai.context_policies` tables |
| `AgentMode` | `vim.ai.context_policies` tables |
| `ContextPolicy` | `ContextGatheringPolicy` + `AgentLoopConfig` |
| `ExtractionStrategy` | `EditFormat` enum |
| `EditMode` | Implicit (selection uses edit_format, chat uses tools) |

---

## Phase 1: Type System Overhaul

**Goal:** Replace the old type system with the new design. All fields parse
from Lua. Existing behavior preserved where possible, cleaned up where not.

### 1.1 Replace `ExtractionStrategy` with `EditFormat`

**Files:** `ovim-core/src/ai/types.rs`, `ovim-core/src/ai/config.rs`,
`ovim-core/src/ai/extract.rs`, `ovim-core/src/ai/provider.rs`,
`ovim-core/src/lua/editor_bridge.rs`, `ovim-core/src/lua/ai_api.rs`

```rust
enum EditFormat {
    Codeblock,
    Json,
    Raw,
    ApplyPatch,     // wired in Phase 6
    StrReplace,     // wired in Phase 6
    Lua(String),    // wired in Phase 3
}
```

- Remove `ExtractionStrategy` enum
- Remove the vestigial `edit_format: String` field on `AiProfileConfig`
- Update `extract_response()` to dispatch on `EditFormat`
- ApplyPatch/StrReplace/Lua variants return clear "not yet implemented" errors
- Update all Lua parsing to read `edit_format` as the new enum

### 1.2 Replace `ContextPolicy` with `ContextGatheringPolicy` + `AgentLoopConfig`

**Files:** `ovim-core/src/ai/types.rs`, `ovim-core/src/ai/config.rs`,
`ovim-core/src/editor/ai_context.rs`, `ovim-core/src/editor/ai_agent.rs`

```rust
struct ContextGatheringPolicy {
    surrounding_lines: u16,         // default: 6
    symbols: u16,                   // default: 12
    diagnostics: DiagnosticScope,   // Overlapping | File
    related_slices: bool,           // default: true
    budget: usize,                  // token ceiling
}

struct AgentLoopConfig {
    max_tool_calls: u16,            // default: 50
}
```

- Remove `ContextPolicy`, `CapabilityTier`, `AgentMode` enums
- Remove `EditMode` enum (implicit from context: selection vs chat)
- Add `context: ContextGatheringPolicy` to `AiProfileConfig`
- The context gathering pipeline uses `ContextGatheringPolicy` fields
  directly instead of matching on `AgentMode`
- `symbols` replaces `retrieval_k`, `related_slices` replaces the
  `AgentMode::Hybrid` check, `budget` replaces `context_budget_tokens`
- Hardcode drop order: related_slices → symbols → diagnostics → surrounding

### 1.3 Expand `AiProfileConfig`

**File:** `ovim-core/src/ai/config.rs`

Add fields (all optional, defaulted to `None` / sensible defaults):
```rust
pub api_key: Option<String>,              // named key reference
pub chat_edit_format: Option<EditFormat>,  // for chat edits
pub edit_prompt: Option<String>,          // per-profile prompt override
pub chat_prompt: Option<String>,          // per-profile prompt override
pub chat_edit_prompt: Option<String>,     // per-profile prompt override
pub reasoning_effort: Option<String>,     // OpenAI only
pub verbosity: Option<String>,           // OpenAI 5.2+
pub syntax_check: Option<bool>,          // post-edit tree-sitter check
pub retry: RetryPolicy,                  // extraction failure recovery
```

Remove fields:
```rust
pub api_key_env: Option<String>,  // replaced by api_key + registry
pub extraction: ExtractionStrategy, // replaced by edit_format: EditFormat
pub edit_format: String,          // replaced by edit_format: EditFormat
pub context_policy: ContextPolicy, // replaced by context: ContextGatheringPolicy
pub edit_mode: EditMode,          // implicit from context type
```

### 1.4 Parse new fields from Lua

**File:** `ovim-core/src/lua/ai_api.rs`

Update `parse_lua_profile()` to read: `api_key`, `edit_format` (as EditFormat),
`chat_edit_format`, `edit_prompt`, `chat_prompt`, `chat_edit_prompt`,
`context` (as ContextGatheringPolicy table), `reasoning_effort`, `verbosity`,
`syntax_check`, `retry` (as table with `max` and `fallback`).

### 1.5 Rename `top_insertions` → `new_import_statements`

**Files:** `ovim-core/src/ai/extract.rs`, `ovim-core/src/ai/types.rs`,
`ovim-core/src/editor/ai_integration.rs`

- JSON extraction accepts `new_import_statements` (primary) and
  `top_insertions` (backward compat alias)
- System prompt uses `new_import_statements`
- Internal struct field renamed
- Insertion logic unchanged

### Verification
- `cargo build` — compiles
- `cargo test` — all existing tests pass
- Existing init.lua configs work (new fields are all optional, old fields
  produce clear errors pointing to the new field names)

---

## Phase 2: Provider Parameters

**Goal:** `reasoning_effort` and `verbosity` are wired to API requests with
correct provider guards.

### 2.1 Apply reasoning_effort (OpenAI only)

**File:** `ovim-core/src/ai/provider.rs` — `apply_optional_params()`

```rust
if profile.provider == OpenAi {
    if let Some(ref effort) = profile.reasoning_effort {
        if effort != "none" {
            body["reasoning"] = json!({ "effort": effort });
            body.as_object_mut().unwrap().remove("temperature");
            body.as_object_mut().unwrap().remove("top_p");
        }
    }
}
```

Also: when `reasoning_effort` is set and not "none", use `max_completion_tokens`
instead of `max_tokens` (already partially handled — verify).

### 2.2 Apply verbosity (OpenAI 5.2+ only)

**File:** `ovim-core/src/ai/provider.rs` — `apply_optional_params()`

```rust
if profile.provider == OpenAi {
    if let Some(ref verbosity) = profile.verbosity {
        body["text"] = json!({ "verbosity": verbosity });
    }
}
```

### 2.3 Remove extraction strategy from user prompt

**File:** `ovim-core/src/ai/provider.rs` — `build_user_prompt()`

Remove the line `Extraction strategy: {}\n\n\` — it's redundant with the
system prompt and wastes tokens.

### Verification
- `cargo test` — unit tests for `apply_optional_params` updated
- Manual test: profile with `reasoning_effort = "low"` sends correct body

---

## Phase 3: API Keys, Prompts & Formats

**Goal:** `vim.api_keys`, `vim.ai.prompts`, `vim.ai.formats`, and
`vim.ai.context_policies` work as described in the sketch. Prompt resolution
chain is live. Hashline ships as a Lua-implemented format.

### 3.1 API key registry

**File:** `ovim-core/src/lua/editor_bridge.rs`

Add to `EditorBridgeInner`:
```rust
api_key_registry: HashMap<String, ApiKeyConfig>,
```

**File:** `ovim-core/src/lua/ai_api.rs`

Implement `vim.api_keys.register(name, { env_var?, file? })`.

### 3.2 API key resolution

**File:** `ovim-core/src/ai/provider.rs` — `read_api_key()`

New resolution path when `profile.api_key` is set:
1. Look up name in `api_key_registry` (passed through `AiState`)
2. Try `config.env_var` → `std::env::var`
3. Try `config.file` → read file, trim
4. Error with clear message

### 3.3 Prompt templates table

**File:** `ovim-core/src/lua/ai_api.rs`

Implement `vim.ai.prompts` as a plain Lua table that gets read during the
sync cycle and stored in `AiState.prompt_templates: HashMap<String, String>`.

### 3.4 Prompt resolution chain

**File:** `ovim-core/src/ai/provider.rs` — system prompt selection

Four-layer resolution for selection edits:
1. `profile.edit_prompt` (per-profile override)
2. `ai_state.prompt_templates["selection_{edit_format}"]`
3. `ai_state.format_registry[format_name].prompt` (if Lua format)
4. `system_prompt_for_extraction()` (Rust fallback)

For chat:
1. `profile.chat_prompt`
2. `ai_state.prompt_templates["chat"]`
3. Hardcoded chat prompt

For chat edits:
1. `profile.chat_edit_prompt`
2. `ai_state.prompt_templates["chat_edit_{chat_edit_format}"]`
3. `ai_state.format_registry[format_name].prompt` (if Lua format)
4. Rust fallback per format

### 3.5 Format registry

**File:** `ovim-core/src/lua/ai_api.rs`

Implement `vim.ai.formats.register(name, { tag?, extract, prompt? })`.

Formats are stored in the bridge as:
```rust
struct LuaFormatRegistration {
    tag: Option<mlua::RegistryKey>,     // fn(lines) → tagged_string
    extract: mlua::RegistryKey,          // fn(response) → edits, error
    prompt: Option<String>,              // fallback prompt for this format
}
```

When a profile's `edit_format` is a Lua function (not a string), the
Lua-side `vim.ai.profiles.register()` auto-registers it under a generated
name (`__anon_1`, `__anon_2`, ...) and stores `Lua("__anon_N")`. One code
path on the Rust side.

### 3.6 Context policies table

**File:** `ovim-core/src/lua/ai_api.rs`

Implement `vim.ai.context_policies` as a plain Lua table. Pre-populated
with `fast`, `hybrid`, `full` tables. Read during sync to validate but
primarily consumed by reference from profile `context` fields.

### 3.7 Sync all new state

**File:** `ovim-core/src/editor/mod.rs` — `sync_ai_config_from_bridge()`

Add to sync cycle:
- `api_key_registry` → `ai_state.api_key_registry`
- `prompt_templates` → `ai_state.prompt_templates`
- `format_registry` → `ai_state.format_registry`

Phase 4 adds to the sync cycle:
- `chat_context_config` → `ai_state.chat_context_config`
- `agent_loop_config` → `ai_state.agent_loop_config`
- `project_context_config` → `ai_state.project_context_config`

### Verification
- `cargo test` — new tests for key resolution (env var, file, missing)
- `cargo test` — new tests for prompt resolution chain (4 layers)
- `cargo test` — format registry round-trip
- Manual: `vim.api_keys.register("openai", { env_var = "MY_KEY" })` works

---

## Phase 4: builtin.lua, Chat Context & Project Context

**Goal:** The harness policy ships inside the binary and runs before init.lua.
Chat context management is configured and operational. Project context files
(.ovim.md, AGENTS.md, CLAUDE.md) are loaded and injected into prompts.

### 4.1 Create the builtin.lua file

**File:** `ovim-core/src/lua/builtin.lua` (new)

Translate `builtin-sketch.lua` to real Lua that exercises the APIs from
Phases 1-3. This IS the sketch minus the commentary appendix: API keys,
prompts, hashline format registration, context policies, profiles, chat
config, agent config.

### 4.2 Load via include_str!

**File:** `ovim-core/src/lua/mod.rs` (or wherever the Lua VM initializes)

```rust
const BUILTIN_LUA: &str = include_str!("builtin.lua");

fn init_lua_vm() {
    lua.load(BUILTIN_LUA).set_name("builtin.lua").exec()?;
    // Then load user's init.lua (which overrides builtin defaults)
}
```

### 4.3 Order guarantee

Ensure the execution order is:
1. Rust creates `vim.ai`, `vim.api_keys`, `vim.ai.formats` APIs
2. `builtin.lua` runs (registers defaults)
3. User's `init.lua` runs (overrides what they want)
4. Sync cycle picks up final state

### 4.4 Chat context management

**File:** `ovim-core/src/lua/ai_api.rs`

Implement `vim.ai.chat` as a plain Lua table. Sync to `AiState`:

```rust
struct ChatContextConfig {
    observation_window: usize,      // default: 10
    mask_template: String,
    max_context_tokens: usize,      // default: 100_000
}
```

**File:** `ovim-core/src/ai/chat_types.rs` (or new file)

Implement observation masking in the chat message serializer:
```rust
fn serialize_for_api(
    turns: &[ChatTurn],
    config: &ChatContextConfig,
    provider: AiProviderKind,
) -> Vec<ApiMessage> {
    let window_start = turns.len().saturating_sub(config.observation_window);
    // Turns before window_start: replace tool results with mask_template
    // If total tokens > max_context_tokens: drop oldest masked turns
}
```

The full conversation is always kept in memory for display. Only the
serialized-for-API version gets masked.

### 4.5 Agent loop config

**File:** `ovim-core/src/lua/ai_api.rs`

Implement `vim.ai.agent` as a plain Lua table. Sync to `AiState`:

```rust
struct AgentLoopConfig {
    max_tool_calls: u16,    // default: 50
}
```

### 4.6 Project context files

**File:** `ovim-core/src/ai/project_context.rs` (new)

Load project context from `.ovim.md`, `AGENTS.md`, `CLAUDE.md` files.

```rust
struct ProjectContextConfig {
    files: Vec<String>,         // default: [".ovim.md", "AGENTS.md", "CLAUDE.md"]
    hierarchical: bool,         // default: true
    budget: usize,              // default: 2000 tokens
    enabled: bool,              // default: true
}
```

**Loading logic:**
1. Walk from current file's directory up to repo root (detected via
   `.git`, or cwd if no repo)
2. At each level, check for files in `config.files` order
3. Load all matching files, deeper files first (more specific context
   takes priority)
4. Concatenate with `\n---\n` separators
5. Truncate to `config.budget` tokens (chars / 4 estimate)
6. Cache result per directory — invalidate on file change or `:AI reload`

**File:** `ovim-core/src/lua/ai_api.rs`

Implement `vim.ai.project_context` as a plain Lua table. Sync to `AiState`.

**File:** `ovim-core/src/ai/provider.rs` — prompt construction

Inject project context between system prompt and user prompt:
- For selection edits: project context competes with code context for
  the profile's `context.budget`. If project context is 1,500 tokens
  and budget is 2,500, only 1,000 tokens remain for code context.
- For chat: project context is injected once in the system prompt.
  It doesn't count against per-turn context budgets.

Format in the prompt:
```
{system_prompt}

## Project Context
{project_context_content}
```

### Verification
- `cargo test` — builtin.lua loads without errors
- A user with no init.lua gets the local Ollama profile
- A user with init.lua overrides work correctly
- Chat observation masking serializes correctly in tests
- Agent loop respects max_tool_calls
- Project context files are found and loaded from repo root
- Hierarchical loading merges files from multiple directories
- Budget truncation works correctly
- Missing project context files are silently skipped

---

## Phase 5: Retry & Elision

**Goal:** Extraction failures get retried with error feedback. Elision
patterns are detected and warned about.

This phase has two sub-phases because they have different architectural
implications.

### Phase 5a: Retry on extraction failure

**File:** `ovim-core/src/ai/provider.rs` or new `ovim-core/src/ai/retry.rs`

The retry protocol runs in the async task. After `extract_response()` fails:

1. Construct error feedback: "Your response could not be parsed. Error:
   {detail}. Please respond with {format_instructions}."
2. Re-call the API with the error appended as a user message.
3. Extract again. If still fails and `retry.fallback` is set, re-prompt
   with the fallback format's system prompt.
4. If all fails, report to user.

**Architectural note:** The async task needs to loop with modified prompts.
This means the provider call path changes from "single request, return
result" to "request loop with retry state." The retry state is:

```rust
struct RetryState {
    attempts: u8,
    max_attempts: u8,
    fallback_format: Option<EditFormat>,
    errors: Vec<String>,
}
```

The system prompt for the fallback format is needed inside the async task.
This means the prompt resolution chain result must be passed into the async
side (already snapshotted in AiState — no new mechanism needed).

### Phase 5b: Elision detection

**File:** `ovim-core/src/ai/extract.rs` (new function)

After successful extraction, scan the replacement for elision patterns:
```
/^\s*\/\/\s*\.\.\./
/^\s*\/\/\s*rest of/i
/^\s*\/\/\s*remaining/i
/^\s*\/\/\s*unchanged/i
```

If detected in a selection edit: re-prompt with anti-elision instruction.
This requires another API call, which feeds back into the retry loop from
Phase 5a.

If detected in a chat edit: set a warning flag on the result for the
hover panel. No re-prompt — the user decides.

### 5c Wire retry to the edit pipeline

**File:** `ovim-core/src/editor/ai_integration.rs`

Update the async edit path to use the retry protocol when the profile's
`retry.max > 0`.

### Verification
- `cargo test` — retry protocol (mock response that fails then succeeds)
- `cargo test` — retry with fallback format
- `cargo test` — elision detection (positive and negative patterns)
- `cargo test` — elision triggers re-prompt in selection mode

---

## Phase 6: Chat Edit Formats

**Goal:** apply_patch, str_replace, and hashline parsers work for chat-driven
file edits. Layered matching handles whitespace mismatches.

This is the largest phase. It deserves its own test fixture set before
implementation begins.

### 6.1 apply_patch parser

**File:** `ovim-core/src/ai/formats/apply_patch.rs` (new)

Parse `*** Begin Patch` / `*** End Patch` envelope:
- Extract file operations (`*** Update File:`, `*** Add File:`, `*** Delete File:`)
- Parse hunks with `@@` context headers
- Parse `-` / `+` / ` ` line prefixes
- Return structured `Vec<FileEdit>` with search context + replacement

### 6.2 str_replace parser

**File:** `ovim-core/src/ai/formats/str_replace.rs` (new)

Parse `<<<<<<< SEARCH` / `=======` / `>>>>>>> REPLACE` blocks:
- Extract pairs of (old_text, new_text)
- Return structured `Vec<SearchReplace>`

### 6.3 Matching engine

**File:** `ovim-core/src/ai/formats/matching.rs` (new)

Two-layer fallback (keeping it simple):
1. **Exact string match** — `str::find`
2. **Whitespace-normalized match** — trim trailing whitespace on both sides,
   normalize line endings, match

**Indentation handling:** Models generally produce code without leading
indentation. ovim computes the correct indentation using the same rules
as `o` and `O` (based on surrounding buffer context). The model's relative
indentation between lines is preserved; only the leading indent level is
adjusted. This replaces the more complex "indentation-normalized matching"
approach — we don't try to match the model's indent, we just reindent.

Fuzzy matching (Levenshtein) is deferred. If the exact and whitespace
layers don't find a match, the edit fails and enters the retry protocol
from Phase 5. In practice, the retry with error feedback ("closest match
is on line 42: `...`") is more reliable than fuzzy matching.

### 6.4 hashline application

**File:** `ovim-core/src/ai/formats/hashline.rs` (new)

The Lua `extract` function returns structured edit operations. The Rust
side receives the structured table and applies it:
- Match hashes to actual buffer lines
- Apply edits using the same reverse-order, single-undo-entry approach
  as apply_patch and str_replace

### 6.5 Chat edit format dispatch

**File:** `ovim-core/src/ai/provider.rs` or `ovim-core/src/editor/ai_integration.rs`

When processing chat edits:
1. Read `profile.chat_edit_format` (or infer from provider)
2. Look up system prompt via resolution chain
3. After response: dispatch to the correct parser
4. On parse/match failure: retry with error feedback (Phase 5)
5. On exhausted retries: fall back to codeblock

### 6.6 Apply parsed edits to buffer

**File:** `ovim-core/src/editor/ai_integration.rs`

For multi-hunk edits from apply_patch/str_replace/hashline:
- Apply in reverse order (bottom-up) to keep byte offsets stable
- Track all modified regions
- Push single undo entry for the batch

### Test Fixtures

Before implementing, create test fixtures with real model outputs:
- `tests/fixtures/apply_patch/` — real apply_patch responses from GPT
- `tests/fixtures/str_replace/` — real str_replace responses from Claude
- `tests/fixtures/hashline/` — real hashline responses
- Each fixture: input file + model response + expected result

### Verification
- `cargo test` — apply_patch parser with real fixtures
- `cargo test` — str_replace parser with real fixtures
- `cargo test` — matching engine (exact, whitespace-normalized)
- `cargo test` — hashline Lua extract + Rust application
- `cargo test` — fallback from apply_patch → codeblock on failure
- Manual: chat mode generates an apply_patch edit on OpenAI, applies correctly

---

## Phase 7: Post-Edit Quality

**Goal:** Tree-sitter syntax validation after edits. Edit format as Lua
function.

### 7.1 Post-edit syntax check

**File:** `ovim-core/src/editor/ai_integration.rs`

After applying an AI edit (if `profile.syntax_check == true`):
1. Get syntax error count before the edit (cached from existing tree)
2. Run tree-sitter incremental parse on the modified region
3. Get syntax error count after
4. If new errors > old errors: set `AiEditRegion.has_syntax_warnings = true`
5. Show in hover panel: "Edit introduced N syntax error(s)"

### 7.2 Intent classification (heuristic)

**File:** `ovim-core/src/editor/ai_integration.rs`

Before building the request:
```rust
fn classify_intent(prompt: &str, selection_lines: usize, diag_count: usize) -> Intent {
    // Keyword + signal based, no LLM call
}
```

Used to:
- Append task-specific prompt hints (AutoPrompter finding)
- Feed into future complexity-based routing

### Verification
- `cargo test` — syntax check detects new errors vs existing
- `cargo test` — intent classifier matches expected patterns

---

## Phase Summary

| Phase | Theme | Key Deliverable | Depends On |
|-------|-------|----------------|------------|
| 1 | Type System Overhaul | EditFormat, ContextGatheringPolicy, clean profile | — |
| 2 | Provider Params | reasoning_effort + verbosity work | 1 |
| 3 | API Keys, Prompts & Formats | vim.api_keys + vim.ai.prompts + vim.ai.formats | 1 |
| 4 | builtin.lua, Chat & Project Context | Harness ships in binary, observation masking, .ovim.md/AGENTS.md | 3 |
| 5a | Retry | Extraction failures recover | 1 |
| 5b | Elision | Elision detected and re-prompted | 5a |
| 6 | Chat Edit Formats | apply_patch + str_replace + hashline parsers | 4, 5 |
| 7 | Post-Edit Quality | syntax_check, intent classification | 1 |

Phases 2, 3, 5a, and 7 are independent of each other and can be worked in
parallel after Phase 1. Phase 4 depends on Phase 3. Phase 5b depends on 5a.
Phase 6 depends on Phases 4 and 5.

```
         ┌──── Phase 2 (Provider Params) ──────────────────────────┐
         │                                                          │
Phase 1 ─┼──── Phase 3 (Keys, Prompts, Formats) ── Phase 4 ───────┬── Phase 6
         │                                      (builtin.lua +     │  (Chat Edit
         │                                       Chat Context)     │   Formats)
         ├──── Phase 5a (Retry) ── Phase 5b (Elision) ────────────┘
         │
         └──── Phase 7 (Post-Edit Quality) ────────────────────────
```

## What We're NOT Doing

These are described in the reference docs but explicitly deferred:

- **Complexity-based sub-routing** (routing.md Phase 2) — wait for intent
  classification data to validate the heuristic
- **Cascade routing** (routing.md Phase 3) — requires post-edit validation
  infrastructure from Phase 7
- **Multi-model diversity** (routing.md future) — too expensive for now
- **Auto-format selection** based on historical performance — need usage
  data first
- **Diff preview before apply** (edit-pipeline.md 7b) — nice but not critical
- **Fuzzy matching** (Levenshtein) in the matching engine — retry with
  error feedback is more reliable; add fuzzy later if retry failure rate
  is high
- **Cost-based agent limits** — requires token counting infrastructure
  and provider price tables; max_tool_calls is the safety rail for now
- **`on_context_overflow` Lua hook** — the config shape for `vim.ai.chat`
  is stable; custom overflow logic can be added later without API changes
- **ROCODE-style mid-generation error detection** — requires control over
  the decoding process (only possible with local models)
- **Constrained decoding / CFG constraints** — GPT-5.2 supports this but
  the API surface is too new; revisit when stable
