# API Design by Example

Starting from a complete policy definition and working backwards to the
building blocks. The question we're answering: what's the minimum set of
concepts that compose into the full harness?

## The Complete Policy (builtin.lua)

This is what ships with ovim. It runs before user init.lua via
`include_str!`. A user who never writes a line of config gets this.

```lua
-- ============================================================
-- ovim builtin.lua — research-backed defaults
-- ============================================================

-- ── API Keys ────────────────────────────────────────────────
--
-- Keys are registered by name. Profiles reference a key name,
-- never a raw string. Resolution: env_var → file → OS keychain.

vim.api_keys.register("openai", {
    env_var = "OVIM_OPENAI_API_KEY",
})

vim.api_keys.register("anthropic", {
    env_var = "OVIM_ANTHROPIC_API_KEY",
})

-- ── Prompts ─────────────────────────────────────────────────
--
-- Named strings. Profiles reference these by key.
-- Override any of them in init.lua.

vim.ai.prompts = {
    selection_codeblock = [[
You are a code editing assistant.
Return ONLY the replacement code inside a single fenced code block (```).
Do not include any explanation outside the code block.
Do not use placeholder comments like "// rest of function" — include ALL code.]],

    selection_json = [[
You are a code editing assistant.
Return a JSON object: {"replacement": string, "top_insertions": string[], "log": string[]}.
Only output valid JSON, no explanation.]],

    selection_raw = [[
You are a code editing assistant.
Return ONLY the replacement code. No markdown, no fences, no explanation.]],

    chat = [[
You are an AI assistant integrated into a code editor called ovim.
Help the user with their code. Be concise.
When showing code changes, use fenced code blocks with the language tag.]],
}

-- ── Profiles ────────────────────────────────────────────────
--
-- A profile is: where to send the request + how to handle the response.
-- That's it. Provider, model, params, format, prompt.

vim.ai.setup({
    default_profile = "local",

    contexts = {
        selection = "local",
        chat = "local",
        query = "local",
    },

    profiles = {
        -- Local inference: works without API keys
        ["local"] = {
            provider = "ollama",
            model = "qwen2.5-coder:7b",
            temperature = 0.2,
            max_tokens = 2048,
            edit_format = "codeblock",
        },

        -- OpenAI: fast/cheap for selection edits
        openai_fast = {
            provider = "openai",
            model = "gpt-4.1-mini",
            api_key = "openai",               -- name, not the secret
            temperature = 0.2,
            max_tokens = 2048,
            edit_format = "codeblock",
        },

        -- OpenAI: balanced
        openai = {
            provider = "openai",
            model = "gpt-4.1",
            api_key = "openai",
            temperature = 0.2,
            max_tokens = 4096,
            edit_format = "codeblock",
        },

        -- OpenAI: frontier
        openai_frontier = {
            provider = "openai",
            model = "gpt-5.2",
            api_key = "openai",
            max_tokens = 4096,
            edit_format = "codeblock",
            reasoning_effort = "none",
            verbosity = "low",
        },

        -- Anthropic: balanced
        anthropic = {
            provider = "anthropic",
            model = "claude-sonnet-4-5-20250929",
            api_key = "anthropic",
            max_tokens = 4096,
            edit_format = "codeblock",
        },

        -- Anthropic: frontier
        anthropic_frontier = {
            provider = "anthropic",
            model = "claude-opus-4-6",
            api_key = "anthropic",
            max_tokens = 4096,
            edit_format = "codeblock",
        },
    },
})
```

That's the entire built-in policy. ~90 lines of Lua. Five concepts:
`api_keys`, `prompts`, `profiles`, `contexts`, `setup`.

---

## User Init.lua Examples

### Minimal: "I use OpenAI"

```lua
-- 3 lines. Everything else comes from builtin.lua.
vim.ai.contexts.selection = "openai_fast"
vim.ai.contexts.chat = "openai_frontier"
vim.ai.contexts.query = "openai"
```

### Moderate: custom routing

```lua
-- Override the system prompt for selection edits
vim.ai.prompts.selection_codeblock = [[
You are editing code. Return ONLY the replacement inside a ``` block.
Preserve all type annotations and lifetimes. Never elide code.
]]

-- Switch contexts
vim.ai.contexts.selection = "openai_fast"
vim.ai.contexts.chat = "anthropic"
vim.ai.contexts.query = "openai"
```

### Advanced: custom profile

```lua
-- Register a profile for a self-hosted model
vim.ai.profiles.register("deepseek", {
    provider = "openai",           -- OpenAI-compatible API
    model = "deepseek-coder-v3",
    base_url = "http://10.0.0.5:8080/v1",
    temperature = 0.1,
    max_tokens = 4096,
    edit_format = "codeblock",
})

vim.ai.contexts.selection = "deepseek"
```

### Power user: custom extraction function

```lua
-- A Lua function that extracts code from the response.
-- Receives the raw response string. Returns (code, error).
local function extract_first_rust_block(response)
    local code = response:match("```rust\n(.-)\n```")
    if code then return code end
    -- Fallback: try any code block
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
    system_prompt = [[
You are a Rust code editing assistant. Return the replacement code
inside a ```rust block. Preserve lifetimes, derive macros, and
trait bounds exactly.
]],
})
```

When `edit_format` is a function, the profile's `system_prompt` is used
directly (no prompt table lookup needed — the user controls both sides).

When `edit_format` is a string like `"codeblock"`, the system prompt is
resolved as: `profile.system_prompt → vim.ai.prompts["selection_" .. edit_format] → Rust fallback`.

---

## Building Blocks

Working backwards from the examples, these are the concepts:

### 1. Profiles

A profile is a named configuration for an AI request.

```
profile = {
    -- Where to send
    provider:       "openai" | "anthropic" | "ollama"
    model:          string
    base_url:       string?          -- override provider endpoint
    api_key:        string?          -- key name (from vim.api_keys)

    -- Request parameters
    temperature:    number?
    max_tokens:     number?
    reasoning_effort: string?        -- "none"|"low"|"medium"|"high" (OpenAI)
    verbosity:      string?          -- "low"|"medium"|"high" (OpenAI 5.2+)

    -- Response handling
    edit_format:    string | function  -- extraction engine or Lua function
    system_prompt:  string?            -- override; otherwise looked up from prompts

    -- Context (existing fields, already working)
    edit_mode:      string?
    tools:          string[]?
    scope:          string?
}
```

That's it. Everything about a request is on the profile.

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
selection_codeblock → system prompt for codeblock extraction
selection_json      → system prompt for JSON extraction
selection_raw       → system prompt for raw extraction
chat                → system prompt for chat context
```

Prompt resolution for selection edits:
```
profile.system_prompt           -- explicit on the profile
  → prompts["selection_" .. edit_format]  -- convention: context_format
  → Rust hardcoded fallback     -- system_prompt_for_extraction()
```

For chat:
```
profile.system_prompt
  → prompts["chat"]
  → Rust hardcoded fallback
```

### 4. API Keys

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

No magic fallback chain. No "try OPENAI_API_KEY if OVIM_OPENAI_API_KEY
isn't set." The builtin.lua registers `OVIM_OPENAI_API_KEY` — if the
user wants the unprefixed form, they override in init.lua:

```lua
vim.api_keys.register("openai", {
    env_var = "OPENAI_API_KEY",    -- their choice
})
```

### 5. vim.ai.setup()

Sugar that sets profiles + contexts + default_profile in one call.
Equivalent to calling `vim.ai.profiles.register()` + setting contexts
individually. Users can use either style.

---

## edit_format: String vs Function

The `edit_format` field on a profile accepts two types:

**String** — names a Rust-implemented extraction engine:
- `"codeblock"` → find first ``` block, strip language tag
- `"json"` → parse JSON `{replacement, top_insertions, log}`
- `"raw"` → use entire response verbatim
- Future: `"apply_patch"`, `"str_replace"`

**Function** — a Lua function for custom extraction:
```lua
function(response: string) → (code: string?, error: string?)
```

Returns the extracted code, or nil + error message. Ovim calls this on
the main thread after receiving the HTTP response, before applying the
edit.

### How Lua functions work with the snapshot model

The profile is snapshotted into `AiState` during the sync cycle. For
string engines, the snapshot stores the string. For Lua functions, the
snapshot stores an `mlua::RegistryKey` — a reference to the function in
the Lua VM.

After the async HTTP response arrives, the main loop:
1. Checks if the profile's edit_format is a RegistryKey
2. If yes: calls the Lua function with the response string
3. If no: calls the Rust extraction engine

This works because edit application already happens on the main thread
(rope mutations aren't Send). The Lua call slots in naturally.

```rust
// In Rust:
enum EditFormat {
    Builtin(String),                // "codeblock", "json", "raw"
    LuaExtractor(mlua::RegistryKey), // reference to Lua function
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

## What We Removed

Compared to the pre-plan, this design drops several concepts:

### Dropped: Extractor Registry (`vim.ai.extractors`)

The pre-plan had a separate `vim.ai.extractors` table mapping format
names to `{engine, prompt_key}` objects. But `edit_format` on the
profile already names the engine, and prompt lookup is just a naming
convention (`"selection_" .. format`). The indirection added a concept
without adding capability.

**Instead:** `edit_format` IS the engine reference (string for Rust
builtins, function for Lua). Prompt lookup is a convention, not a
registry.

### Dropped: Magic API key fallback chain

The pre-plan had a 4-step chain: explicit env → secrets.toml →
OVIM_PROVIDER → PROVIDER. Too much magic. If the user's key isn't
found, tell them clearly — don't silently try other variables.

**Instead:** Each key registration says exactly where to look. One or
two sources, checked in order. Explicit beats clever.

### Dropped: `api_key_env` on profiles

Replaced by `api_key = "name"` referencing the key registry. This
centralizes key configuration: change it once in `vim.api_keys.register`,
all profiles that reference "openai" pick up the change.

### Dropped: secrets.toml

Replaced by `vim.api_keys.register("openai", { file = "..." })`.
The user picks the file location and manages permissions. No new
config file format to maintain. If they want `~/.secrets/openai.key`
they point to it. If they want the OS keychain, we add that later.

---

## Backward Compatibility

### Current: `api_key_env` on profiles

The current Lua API uses `api_key_env = "OVIM_OPENAI_API_KEY"` directly
on profiles. To avoid a breaking change:

```rust
// When syncing a profile from Lua:
// If api_key is set → use as key name
// If api_key_env is set (legacy) → create an anonymous key registration
//   with that env_var and reference it
```

This means existing init.lua files keep working. New configs use
`api_key = "name"`.

### Current: `edit_format` as string

Already works. Adding function support is purely additive — string
values work exactly as before, functions are a new capability.

---

## Concept Count

| Concept | What it is | Required? |
|---------|-----------|-----------|
| **Profile** | Model + params + format | Yes |
| **Context** | Action → profile name | Yes |
| **Prompt** | Named system prompt string | Optional (convention) |
| **API Key** | Named key config | Yes (for cloud providers) |

Four concepts. Profiles and contexts are load-bearing — can't remove
them. Prompts are a convenience for reuse and override. API keys are
a security boundary.

`vim.ai.setup()` is sugar, not a concept. It's `profiles.register` +
`contexts` assignment in one call.

---

## Implementation Path

What needs to change in the Rust core to support this:

### Step 1: `api_key` field on profiles

Add `api_key: Option<String>` to `LuaProfileConfig` and
`AiProfileConfig`. Parse from Lua. In `provider.rs`, resolve through
the key registry instead of reading `api_key_env` directly.

### Step 2: `vim.api_keys` Lua API

Create the `vim.api_keys` table in `setup_vim_api()`:
- `.register(name, opts)` → stores in bridge
- `.get(name)` → returns opaque string (just the name)
- `.dangerously_get_raw(name)` → resolves and returns the secret

Store the registry in `EditorBridge`, snapshot into `AiState`.

### Step 3: `vim.ai.prompts` table

A plain Lua table stored on the `vim.ai` namespace. Snapshot into
`AiState.prompt_templates` during sync. Used in prompt resolution.

### Step 4: `edit_format` as function

Extend `LuaProfileConfig` to store an `mlua::RegistryKey` when the
Lua value is a function. Map to `EditFormat::LuaExtractor` in
`AiProfileConfig`. Call on the main thread after HTTP response.

### Step 5: builtin.lua

Create `ovim-core/src/lua/builtin.lua` with the policy above.
`include_str!` it. Execute before user's init.lua.

### Step 6: Profile config expansion

Add `reasoning_effort`, `verbosity` to profile structs. Apply in
`provider.rs` with provider guards.

Steps 1-3 and 6 are independent. Step 4 depends on having the
profile plumbing from step 1. Step 5 depends on all prior steps.
