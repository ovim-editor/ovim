# API Design by Example

Starting from a complete policy definition and working backwards to the
building blocks. The question we're answering: what's the minimum set of
concepts that compose into the full harness?

**Note:** The canonical policy definition is in `builtin-sketch.lua`.
This document explains the design rationale and shows user-facing examples.

## User Init.lua Examples

### Minimal: "I use OpenAI"

```lua
-- 3 lines. Everything else comes from builtin.lua.
vim.ai.contexts.selection = "openai_fast"
vim.ai.contexts.chat = "openai_frontier"
vim.ai.contexts.query = "openai"
```

### Moderate: custom routing + prompt

```lua
-- Override the system prompt for selection edits (global)
vim.ai.prompts.selection_codeblock = [[
You are editing code. Return ONLY the replacement inside a ``` block.
Preserve all type annotations and lifetimes. Never elide code.
]]

-- Switch contexts
vim.ai.contexts.selection = "openai_fast"
vim.ai.contexts.chat = "anthropic"
vim.ai.contexts.query = "openai"
```

### Advanced: custom profile with per-model prompt

```lua
-- Custom context policy for deep Rust work
local rust_deep = vim.tbl_extend("force",
    vim.ai.context_policies.hybrid,
    { budget = 16000, symbols = 20, surrounding_lines = 20 })

vim.ai.profiles.register("claude_rust", {
    provider = "anthropic",
    model = "claude-sonnet-4-5-20250929",
    api_key = "anthropic",
    max_tokens = 4096,
    edit_format = "codeblock",
    chat_edit_format = "str_replace",
    context = rust_deep,
    syntax_check = true,
    retry = { max = 1, fallback = "codeblock" },
    -- Per-profile prompt — different models need different instructions
    edit_prompt = [[
You are editing Rust code. Return ONLY the replacement inside a
```rust block. Preserve all lifetime annotations and trait bounds.
Never use placeholder comments. Include ALL code.]],
})

vim.ai.contexts.selection = "openai_fast"
vim.ai.contexts.chat = "claude_rust"
```

### Self-hosted model

```lua
vim.ai.profiles.register("deepseek", {
    provider = "openai",           -- OpenAI-compatible API
    model = "deepseek-coder-v3",
    base_url = "http://10.0.0.5:8080/v1",
    temperature = 0.1,
    max_tokens = 4096,
    edit_format = "codeblock",
    chat_edit_format = "hashline",     -- works across model families
    context = vim.ai.context_policies.hybrid,
})

vim.ai.contexts.selection = "deepseek"
```

### Power user: custom extraction function

```lua
local function extract_first_rust_block(response)
    local code = response:match("```rust\n(.-)\n```")
    if code then return code end
    code = response:match("```%w*\n(.-)\n```")
    if code then return code end
    return nil, "no code block found"
end

vim.ai.profiles.register("rust_specialist", {
    provider = "openai",
    model = "gpt-4.1",
    api_key = "openai",
    max_tokens = 4096,
    edit_format = extract_first_rust_block,   -- function, not string
    edit_prompt = "Return code in a ```rust block.",
})
```

When `edit_format` is a function, it's auto-registered as a Lua format
under a generated name. The profile's `edit_prompt` provides the matching
system prompt instruction.

### Researcher: custom edit format

```lua
vim.ai.formats.register("my_novel_format", {
    tag = function(lines)
        -- Transform context lines before sending to model
        -- ... your tagging logic ...
    end,
    extract = function(response)
        -- Parse the model's response into structured edits
        -- ... your parsing logic ...
    end,
    prompt = "Instructions for the model on how to use this format...",
})

vim.ai.profiles.register("experimental", {
    provider = "openai",
    model = "gpt-4.1",
    chat_edit_format = "my_novel_format",
})
```

---

## Building Blocks

Working backwards from the examples, these are the concepts:

### 1. Profiles

A profile is a named configuration for an AI request.

```
profile = {
    -- Where to send
    provider:         "openai" | "anthropic" | "ollama"
    model:            string
    base_url:         string?          -- override provider endpoint
    api_key:          string?          -- key name (from vim.api_keys)

    -- Request parameters
    temperature:      number?
    max_tokens:       number?
    reasoning_effort: string?          -- "none"|"low"|"medium"|"high" (OpenAI)
    verbosity:        string?          -- "low"|"medium"|"high" (OpenAI 5.2+)

    -- Response handling
    edit_format:      string | function  -- for selection edits
    chat_edit_format: string?            -- for chat edits (provider default if nil)

    -- Prompt overrides (per-profile, skip global lookup)
    edit_prompt:      string?          -- override for selection edits
    chat_prompt:      string?          -- override for chat
    chat_edit_prompt: string?          -- override for chat edits

    -- Context gathering
    context:          table?           -- inline ContextGatheringPolicy
                                       -- or reference: vim.ai.context_policies.fast
                                       -- defaults to hybrid if omitted

    -- Quality
    syntax_check:     bool?            -- tree-sitter post-edit check
    retry:            table?           -- { max = 1, fallback = "codeblock" }

    -- Existing fields
    tools:            string[]?
    scope:            string?
}
```

### 2. Contexts

Map action names to profile names.

```
selection → profile_name   -- visual selection edits
chat      → profile_name   -- AI chat panel
query     → profile_name   -- :AI command
```

Three entries. Might grow (e.g., `commit_message`, `explain`), but the
pattern stays the same: context name → profile name.

### 3. Prompts

Named strings. `vim.ai.prompts` is a plain Lua table.

```
selection_codeblock       → system prompt for codeblock extraction
selection_json            → system prompt for JSON extraction
selection_raw             → system prompt for raw extraction
chat                      → system prompt for chat context
chat_edit_apply_patch     → system prompt for apply_patch chat edits
chat_edit_str_replace     → system prompt for str_replace chat edits
chat_edit_codeblock       → system prompt for codeblock chat edits
```

Prompt resolution for selection edits (4 layers):
```
profile.edit_prompt                       -- per-profile override
  → prompts["selection_" .. edit_format]  -- global default
  → format.prompt                         -- from vim.ai.formats.register
  → Rust hardcoded fallback               -- safety net
```

For chat:
```
profile.chat_prompt
  → prompts["chat"]
  → Rust hardcoded fallback
```

For chat edits:
```
profile.chat_edit_prompt
  → prompts["chat_edit_" .. chat_edit_format]
  → format.prompt
  → Rust hardcoded fallback
```

### 4. Formats

Extensible edit format engines. Built-in formats (codeblock, json, raw,
apply_patch, str_replace) are implemented in Rust. Custom formats are
registered in Lua via `vim.ai.formats.register()`.

A format has three parts:
- `tag` (optional): transforms context lines before the prompt
- `extract` (required): parses model output into structured edits
- `prompt` (optional): fallback system prompt for this format

Hashline ships as a Lua-registered format inside builtin.lua, demonstrating
the extensibility system.

### 5. Context Policies

Plain Lua tables that control context gathering. Pre-defined as
`vim.ai.context_policies.{fast, hybrid, full}`. Profiles reference
them directly or extend with `vim.tbl_extend()`.

Fields: `surrounding_lines`, `symbols`, `diagnostics`, `related_slices`,
`budget`. No registry, no indirection — Lua variables are the reuse
mechanism.

### 6. API Keys

Named key configurations. Never contain the raw secret.

```lua
vim.api_keys.register(name, {
    env_var = "...",       -- check this environment variable
    -- OR --
    file = "~/.secrets/...",  -- read this file (trimmed)
})
```

Profiles reference keys by name: `api_key = "openai"`.

Resolution at request time (in Rust):
1. Look up key name in registry
2. Try `env_var` → `file`
3. If neither works → clear error message

No magic fallback chain. The builtin.lua registers `OVIM_OPENAI_API_KEY`
— if the user wants the unprefixed form, they override in init.lua:

```lua
vim.api_keys.register("openai", {
    env_var = "OPENAI_API_KEY",    -- their choice
})
```

### 7. Chat & Agent Config

Two plain Lua tables for operational config:

```lua
vim.ai.chat = {
    observation_window = 10,       -- recent turns with full content
    mask_template = "...",         -- placeholder for old tool outputs
    max_context_tokens = 100000,   -- drop oldest masked turns when exceeded
}

vim.ai.agent = {
    max_tool_calls = 50,           -- safety rail, not tuning knob
}
```

### 8. vim.ai.setup()

Sugar that sets profiles + contexts + default_profile in one call.
Equivalent to calling `vim.ai.profiles.register()` + setting contexts
individually. Users can use either style.

---

## edit_format: String vs Function

The `edit_format` field on a profile accepts two types:

**String** — names a built-in or registered format:
- `"codeblock"` → find first ``` block, strip language tag (Rust)
- `"json"` → parse JSON `{replacement, new_import_statements, log}` (Rust)
- `"raw"` → use entire response verbatim (Rust)
- `"apply_patch"` → parse *** Begin/End Patch envelope (Rust)
- `"str_replace"` → parse <<<<<<< SEARCH / >>>>>>> REPLACE (Rust)
- `"hashline"` → content-hash-based addressing (Lua, registered in builtin.lua)
- Any name registered via `vim.ai.formats.register()` (Lua)

**Function** — an inline Lua extractor:
```lua
function(response: string) → (code: string?, error: string?)
```

Returns the extracted code, or nil + error message. When a function is
passed, it's auto-registered under a generated name (`__anon_1`, etc.)
so it flows through the same code path as named Lua formats.

### How Lua formats work with the snapshot model

The profile is snapshotted into `AiState` during the sync cycle. For
built-in Rust formats, the snapshot stores the enum variant. For Lua
formats, it stores `EditFormat::Lua(name)`.

After the async HTTP response arrives, the main loop:
1. Checks if the format is `Lua(name)`
2. If yes: looks up the format in the registry, calls `extract(response)`
   on the main thread
3. If no: calls the Rust extraction engine

This works because edit application already happens on the main thread
(rope mutations aren't Send). The Lua call slots in naturally.

```rust
enum EditFormat {
    Codeblock,
    Json,
    Raw,
    ApplyPatch,
    StrReplace,
    Lua(String),    // name in vim.ai.formats registry
}
```

---

## vim.api_keys

### The Problem

API keys are secrets. They shouldn't appear as string literals in config
files. They shouldn't be logged. They shouldn't show up in error
messages. Current approach (`api_key_env = "OVIM_OPENAI_API_KEY"`) is
already indirect, but the key name lives on every profile that uses it.

### The Design

`vim.api_keys` is a registry of named key configurations. Profiles
reference keys by name. The raw secret is never in Lua-land except
through an explicitly scary function.

```lua
-- Register: defines HOW to find a key
vim.api_keys.register("openai", {
    env_var = "OVIM_OPENAI_API_KEY",
})

-- Reference: used in profiles (just a name string)
{ api_key = "openai" }

-- Programmatic access (for plugins that need raw keys):
local key_id = vim.api_keys.get("openai")              -- opaque handle
local raw    = vim.api_keys.dangerously_get_raw("openai") -- actual secret
```

The `dangerously_` prefix is the pit of success: you can't accidentally
use the raw key without the name screaming at you.

### Resolution (Rust side)

```rust
struct ApiKeyConfig {
    env_var: Option<String>,
    file: Option<PathBuf>,
}

fn resolve_api_key(name: &str, registry: &HashMap<String, ApiKeyConfig>) -> Result<String> {
    let config = registry.get(name)
        .ok_or_else(|| anyhow!("No API key registered for '{name}'"))?;

    if let Some(ref var) = config.env_var {
        if let Ok(key) = std::env::var(var) {
            if !key.is_empty() {
                return Ok(key);
            }
        }
    }

    if let Some(ref path) = config.file {
        let expanded = shellexpand::tilde(&path.to_string_lossy());
        if let Ok(key) = std::fs::read_to_string(expanded.as_ref()) {
            let key = key.trim().to_string();
            if !key.is_empty() {
                return Ok(key);
            }
        }
    }

    Err(anyhow!(
        "API key '{name}' not found. Set {} or configure a key file.",
        config.env_var.as_deref().unwrap_or("an env var")
    ))
}
```

### Secure Storage (Future)

The `env_var` and `file` sources cover 95% of use cases. For deeper
security, a future `keychain` source could tap OS-native storage:

| OS | Backend | CLI equivalent |
|----|---------|---------------|
| macOS | Keychain Services | `security find-generic-password -s ovim -a openai` |
| Linux | libsecret / Secret Service API | `secret-tool lookup service ovim key openai` |
| Windows | Credential Manager | `cmdkey /generic:ovim-openai` |

```lua
-- Future:
vim.api_keys.register("openai", {
    keychain = true,   -- looks up "ovim/openai" in OS keychain
})
```

And a `:SetApiKey openai` command that writes to the keychain with
masked input. But this is a "wow" feature, not a blocker.

For now: `env_var` + `file`. The architecture supports adding
`keychain` later without changing the profile-side API at all.

---

## Concept Count

| Concept | What it is | Required? |
|---------|-----------|-----------|
| **Profile** | Model + params + format + context | Yes |
| **Context** | Action → profile name | Yes |
| **Prompt** | Named system prompt string | Optional (convention) |
| **Format** | Extensible extraction engine | Optional (builtins suffice) |
| **Context Policy** | How much context to gather | Optional (defaults work) |
| **API Key** | Named key config | Yes (for cloud providers) |
| **Chat Config** | Observation masking | Optional (defaults work) |
| **Agent Config** | Safety rails | Optional (defaults work) |

Profiles and contexts are load-bearing. Everything else has sensible
defaults that work without any user configuration.

`vim.ai.setup()` is sugar, not a concept. It's `profiles.register` +
`contexts` assignment in one call.

---

## Implementation Path

See `roadmap.md` for the full phased implementation plan. Summary:

1. **Phase 1: Type System Overhaul** — Replace ExtractionStrategy with
   EditFormat, replace ContextPolicy with ContextGatheringPolicy +
   AgentLoopConfig, expand AiProfileConfig with all new fields.

2. **Phase 2: Provider Parameters** — Wire reasoning_effort and verbosity
   to API requests with provider guards.

3. **Phase 3: API Keys, Prompts & Formats** — vim.api_keys, vim.ai.prompts,
   vim.ai.formats.register(), vim.ai.context_policies. 4-layer prompt
   resolution chain.

4. **Phase 4: builtin.lua & Chat Context** — Harness ships in binary.
   Observation masking for chat. Agent loop config.

5. **Phase 5: Retry & Elision** — Extraction failure retry protocol.
   Elision detection and re-prompting.

6. **Phase 6: Chat Edit Formats** — apply_patch, str_replace, hashline
   parsers. Matching engine. Multi-hunk buffer application.

7. **Phase 7: Post-Edit Quality** — Tree-sitter syntax validation.
   Intent classification heuristic.
