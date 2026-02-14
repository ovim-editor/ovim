# Pre-Plan: Getting the Core Ready for the Harness

Before building the configurable harness, the Rust core needs a few
structural changes. This document lists them in dependency order.

## Design Principles

1. **Rust is the engine, Lua is the policy.** Rust provides HTTP, parsing,
   matching, rope ops. Lua decides which engine to use, what prompts to
   send, what params to set.

2. **Batteries included.** A user with `OVIM_OPENAI_API_KEY` set should
   get a working, well-tuned AI experience with zero config. The built-in
   Lua file provides research-backed defaults.

3. **Overridable at every layer.** Users can override profiles, prompts,
   extractors, routing rules — all from init.lua. The built-in defaults
   are just Lua that runs first.

4. **Snapshot, don't callback.** Lua config is snapshotted into Rust state
   during the sync cycle. The async request path reads from the snapshot,
   never from the Lua VM.


## Phase 0: Profile Config Expansion

**Goal:** `AiProfileConfig` can express everything the harness needs.

### 0a. Add fields to LuaProfileConfig

```rust
// editor_bridge.rs
pub struct LuaProfileConfig {
    // ... existing fields ...

    // New: provider-specific parameters
    pub reasoning_effort: Option<String>,     // "none"|"low"|"medium"|"high"|"xhigh"
    pub verbosity: Option<String>,            // "low"|"medium"|"high"
    pub chat_system_prompt: Option<String>,   // separate from selection system_prompt
}
```

### 0b. Parse new fields from Lua

```rust
// ai_api.rs, in parse_lua_profile()
reasoning_effort: tbl.get::<_, String>("reasoning_effort").ok(),
verbosity: tbl.get::<_, String>("verbosity").ok(),
chat_system_prompt: tbl.get::<_, String>("chat_system_prompt").ok(),
```

### 0c. Add fields to AiProfileConfig

```rust
// config.rs
pub struct AiProfileConfig {
    // ... existing fields ...
    pub reasoning_effort: Option<String>,
    pub verbosity: Option<String>,
    pub chat_system_prompt: Option<String>,
}
```

### 0d. Wire through into_profile_config()

```rust
// editor_bridge.rs, in into_profile_config()
reasoning_effort: self.reasoning_effort,
verbosity: self.verbosity,
chat_system_prompt: self.chat_system_prompt,
```

### 0e. Apply in provider.rs

```rust
// provider.rs, in apply_optional_params()
fn apply_optional_params(body: &mut Value, profile: &AiProfileConfig, ...) {
    // Temperature
    if let Some(temp) = profile.temperature {
        // OpenAI: temperature only works with reasoning_effort=none
        let reasoning = profile.reasoning_effort.as_deref().unwrap_or("none");
        if profile.provider == AiProviderKind::OpenAi && reasoning != "none" {
            // Skip temperature — incompatible with reasoning
        } else {
            match profile.provider {
                AiProviderKind::Ollama => body["options"] = json!({"temperature": temp}),
                _ => body["temperature"] = json!(temp),
            }
        }
    }

    // Max tokens (existing: max_completion_tokens for OpenAI)
    if let Some(max_tokens) = profile.max_tokens {
        let key = match profile.provider {
            AiProviderKind::OpenAi => "max_completion_tokens",
            _ => "max_tokens",
        };
        body[key] = json!(max_tokens);
    }

    // Reasoning effort (OpenAI only)
    if let Some(ref effort) = profile.reasoning_effort {
        if profile.provider == AiProviderKind::OpenAi {
            body["reasoning"] = json!({ "effort": effort });
        }
    }

    // Verbosity (OpenAI GPT-5.2+ only)
    if let Some(ref verbosity) = profile.verbosity {
        if profile.provider == AiProviderKind::OpenAi {
            body["text"] = json!({ "verbosity": verbosity });
        }
    }

    // Tools (existing)
    if let Some(tools) = tools {
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
    }
}
```

### 0f. Use chat_system_prompt in streaming chat

```rust
// provider.rs, in stream_openai_chat()
let sys = system_prompt
    .or(profile.chat_system_prompt.as_deref())
    .or(profile.system_prompt.as_deref());
```

**Files:**
- `ovim-core/src/lua/editor_bridge.rs`
- `ovim-core/src/lua/ai_api.rs`
- `ovim-core/src/ai/config.rs`
- `ovim-core/src/ai/provider.rs`

**Tests:** Existing provider tests + new test for temperature/reasoning
guard, reasoning_effort body injection, verbosity body injection.


## Phase 1: Built-in Lua File

**Goal:** Default configuration lives in Lua, not hardcoded Rust.

### 1a. Create the built-in Lua file

```
ovim-core/src/lua/builtin.lua
```

Contents (see §Appendix A for full draft):
- `vim.ai.prompts` table — system prompt templates
- `vim.ai.provider_defaults` table — per-provider parameter conventions
- `vim.ai.extractors` table — extraction strategy registry (data only)
- Default `vim.ai.setup()` call with local Ollama profile

### 1b. Include and execute before user config

```rust
// lua/mod.rs
const BUILTIN_LUA: &str = include_str!("builtin.lua");

// In load_config(), before loading user's init.lua:
impl LuaContext {
    pub fn load_builtin(&self) -> Result<()> {
        self.lua.load(BUILTIN_LUA)
            .set_name("[builtin]")
            .exec()?;
        Ok(())
    }
}
```

```rust
// lua_integration.rs, in enable_lua():
crate::lua::setup_vim_api(context.lua(), bridge.clone())?;
context.load_builtin()?;   // ← NEW: defaults first
match context.load_config() {
    // ... user config overrides ...
}
```

### 1c. Move hardcoded prompts to Lua

Currently `system_prompt_for_extraction()` in config.rs returns hardcoded
strings. After this change:

```rust
// config.rs — keep as ultimate fallback
pub fn system_prompt_for_extraction(strategy: ExtractionStrategy) -> &'static str {
    // Same as before — used only when Lua prompts aren't loaded
}
```

The provider.rs resolution chain becomes:
```
profile.system_prompt                          ← explicit per-profile
  → ai_state.prompt_templates[context_format]  ← from Lua via snapshot
  → system_prompt_for_extraction(strategy)     ← Rust fallback
```

**Files:**
- `ovim-core/src/lua/builtin.lua` (new)
- `ovim-core/src/lua/mod.rs`
- `ovim-core/src/editor/lua_integration.rs`

**Tests:** `enable_lua()` with no user config → built-in profiles loaded.
Override in init.lua → user profiles win.


## Phase 2: Prompt Template Snapshot

**Goal:** Lua prompt templates are readable from the async request path.

### 2a. Add prompt_templates to AiState

```rust
// ai_state.rs
pub struct AiState {
    // ... existing fields ...
    pub prompt_templates: HashMap<String, String>,
}
```

### 2b. Snapshot during sync

```rust
// editor_bridge.rs — add method
impl EditorBridge {
    pub fn take_ai_prompts_if_dirty(&self) -> Option<HashMap<String, String>> {
        // Read vim.ai.prompts table, return as HashMap
    }
}
```

Alternative (simpler): Store prompts in the bridge alongside profiles.
The `vim.ai.prompts` table uses a metatable that writes to the bridge
on `__newindex`, same pattern as `vim.ai.contexts`.

```rust
// lua_integration.rs, in sync_ai_config_from_bridge():
if let Some(prompts) = bridge.take_ai_prompts_if_dirty() {
    self.ai_state.prompt_templates = prompts;
}
```

### 2c. Use in provider.rs

```rust
// provider.rs, in request_openai():
fn resolve_system_prompt(
    profile: &AiProfileConfig,
    request: &AiRequest,
    templates: &HashMap<String, String>,
) -> String {
    // 1. Explicit profile system_prompt
    if let Some(ref sp) = profile.system_prompt {
        return sp.clone();
    }

    // 2. Lua template: "selection_codeblock", "selection_json", etc.
    let key = format!("selection_{}", profile.edit_format);
    if let Some(sp) = templates.get(&key) {
        return sp.clone();
    }

    // 3. Rust fallback
    system_prompt_for_extraction(request.extraction).to_string()
}
```

This requires threading `prompt_templates` through to the request
functions. The cleanest way: pass it in `AiRequest` or as a separate
param to `request_ai_edit()`.

**Files:**
- `ovim-core/src/editor/ai_state.rs`
- `ovim-core/src/lua/editor_bridge.rs`
- `ovim-core/src/lua/ai_api.rs`
- `ovim-core/src/editor/lua_integration.rs`
- `ovim-core/src/ai/provider.rs`


## Phase 3: Extractor Registry (Data Layer)

**Goal:** `vim.ai.extractors` is a Lua table that controls which Rust
extraction engine runs for each format name.

### 3a. Define the Lua-side data structure

In `builtin.lua`:
```lua
vim.ai.extractors = {
    codeblock = {
        engine = "codeblock",
        prompt_key = "selection_codeblock",
    },
    json = {
        engine = "json",
        prompt_key = "selection_json",
    },
    raw = {
        engine = "raw",
        prompt_key = "selection_raw",
    },
}
```

### 3b. Snapshot into Rust

```rust
// New struct
pub struct ExtractorConfig {
    pub engine: String,       // "codeblock", "json", "raw"
    pub prompt_key: String,   // key into prompt_templates
}

// In AiState:
pub extractors: HashMap<String, ExtractorConfig>,
```

### 3c. Resolution in the pipeline

When a profile has `edit_format = "codeblock"`:
1. Look up `extractors["codeblock"]` → `ExtractorConfig { engine: "codeblock", prompt_key: "selection_codeblock" }`
2. Resolve system prompt from `prompt_templates["selection_codeblock"]`
3. Resolve extraction strategy from engine name → `ExtractionStrategy::Codeblock`
4. Build request, extract response

This replaces the current `match edit_format.as_str()` → `ExtractionStrategy`
mapping in `into_profile_config()` with a registry lookup.

### 3d. Bring the stub to life

`vim.ai.edit_formats.register()` currently does nothing. After this phase,
it writes to the `vim.ai.extractors` table (or a parallel registry):

```lua
vim.ai.edit_formats.register("str_replace", {
    engine = "str_replace",  -- when the Rust engine exists
    prompt_key = "selection_str_replace",
})
```

**Files:**
- `ovim-core/src/lua/builtin.lua`
- `ovim-core/src/lua/ai_api.rs`
- `ovim-core/src/lua/editor_bridge.rs`
- `ovim-core/src/editor/ai_state.rs`
- `ovim-core/src/editor/lua_integration.rs`
- `ovim-core/src/ai/provider.rs`


## Phase 4: API Key Resolution Chain

**Goal:** Keys resolve through a well-defined chain with OVIM_ prefix
as the convention.

### 4a. Resolution order

```
1. Profile's explicit api_key_env → env var
2. secrets.toml → provider-keyed value (future, Phase 4b)
3. OVIM_{PROVIDER}_API_KEY → env var
4. {PROVIDER}_API_KEY → env var (with one-time notice)
```

### 4b. Secrets file (near-term, not blocking)

```toml
# ~/.config/ovim/secrets.toml (mode 0600)
[api_keys]
openai = "sk-..."
anthropic = "sk-ant-..."
```

```rust
fn read_api_key(profile: &AiProfileConfig) -> Result<String> {
    // 1. Explicit env var from profile
    if let Some(ref env_var) = profile.api_key_env {
        if let Ok(key) = std::env::var(env_var) {
            return Ok(key);
        }
    }

    // 2. Secrets file (future)
    // if let Some(key) = read_secrets_file(profile.provider) { return Ok(key); }

    // 3. OVIM-prefixed env var
    let ovim_var = format!("OVIM_{}_API_KEY", provider_env_name(profile.provider));
    if let Ok(key) = std::env::var(&ovim_var) {
        return Ok(key);
    }

    // 4. Standard env var (with notice)
    if let Some(standard_var) = standard_api_key_env(profile.provider) {
        if let Ok(key) = std::env::var(&standard_var) {
            // Log one-time notice
            crate::log_info!(
                "ai",
                "Using {} (set {} for isolation)",
                standard_var, ovim_var
            );
            return Ok(key);
        }
    }

    Err(anyhow!("No API key found. Set {} or configure api_key_env.", ovim_var))
}

fn provider_env_name(provider: AiProviderKind) -> &'static str {
    match provider {
        AiProviderKind::OpenAi => "OPENAI",
        AiProviderKind::Anthropic => "ANTHROPIC",
        AiProviderKind::Ollama => unreachable!(), // Ollama doesn't need keys
    }
}
```

### 4c. `:SetApiKey` command (near-term polish)

```
:SetApiKey openai
Enter API key: ************************************
API key saved to ~/.config/ovim/secrets.toml
```

Implementation:
- New ex command `SetApiKey` with provider argument
- Masked input (don't echo characters)
- Write to secrets.toml with 0600 permissions
- Reload API key resolution cache

**Files:**
- `ovim-core/src/ai/provider.rs` (key resolution)
- `ovim-core/src/ai/secrets.rs` (new, secrets file I/O)
- `ovim-core/src/commands.rs` (`:SetApiKey` command)


## Phase 5: Update builtin.lua with Default Profiles

**Goal:** Out-of-the-box profiles that work with OVIM_ env var convention.

```lua
-- builtin.lua (excerpt)
vim.ai.setup({
    default_profile = "local",

    contexts = {
        selection = "local",
        chat = "local",
        query = "local",
    },

    profiles = {
        -- Local inference (works without API keys)
        local = {
            provider = "ollama",
            model = "qwen2.5-coder:7b",
            temperature = 0.2,
            max_tokens = 2048,
            edit_format = "codeblock",
        },

        -- OpenAI: fast selection edits
        openai_fast = {
            provider = "openai",
            model = "gpt-4.1-mini",
            api_key_env = "OVIM_OPENAI_API_KEY",
            temperature = 0.2,
            max_tokens = 2048,
            edit_format = "codeblock",
        },

        -- OpenAI: capable model for chat
        openai = {
            provider = "openai",
            model = "gpt-4.1",
            api_key_env = "OVIM_OPENAI_API_KEY",
            temperature = 0.2,
            max_tokens = 4096,
            edit_format = "codeblock",
            reasoning_effort = "none",
        },

        -- OpenAI: frontier model
        openai_frontier = {
            provider = "openai",
            model = "gpt-5.2",
            api_key_env = "OVIM_OPENAI_API_KEY",
            temperature = 0.2,
            max_tokens = 4096,
            edit_format = "codeblock",
            reasoning_effort = "none",
            verbosity = "low",
        },

        -- Anthropic: capable model
        anthropic = {
            provider = "anthropic",
            model = "claude-sonnet-4-5-20250929",
            api_key_env = "OVIM_ANTHROPIC_API_KEY",
            max_tokens = 4096,
            edit_format = "codeblock",
        },

        -- Anthropic: frontier model
        anthropic_frontier = {
            provider = "anthropic",
            model = "claude-opus-4-6",
            api_key_env = "OVIM_ANTHROPIC_API_KEY",
            max_tokens = 4096,
            edit_format = "codeblock",
        },
    },
})
```

The user's init.lua then becomes much simpler:

```lua
-- User init.lua: just set contexts and override what they want
vim.ai.default_profile = "openai"
vim.ai.contexts.selection = "openai_fast"
vim.ai.contexts.chat = "openai_frontier"
```


## Dependency Graph

```
Phase 0: Profile config expansion
    │
    ├──→ Phase 1: Built-in Lua file (depends on 0 for new fields)
    │       │
    │       ├──→ Phase 2: Prompt template snapshot (depends on 1 for Lua prompts)
    │       │
    │       └──→ Phase 5: Default profiles in builtin.lua (depends on 1)
    │
    ├──→ Phase 3: Extractor registry (depends on 0 + 2)
    │
    └──→ Phase 4: API key chain (independent, can run in parallel)
```

Phases 0 and 4 can start immediately in parallel.
Phase 1 depends on 0.
Phases 2, 3, and 5 depend on 1.


## Appendix A: Full builtin.lua Draft

See separate file: `ovim-core/src/lua/builtin.lua` (created during
Phase 1 implementation).


## Appendix B: User init.lua After This Work

```lua
-- Minimal init.lua for an OpenAI user:
vim.ai.default_profile = "openai"
vim.ai.contexts.selection = "openai_fast"
vim.ai.contexts.chat = "openai_frontier"

-- That's it. Profiles, prompts, extractors all come from builtin.lua.
-- Override anything you want:
-- vim.ai.prompts.selection_codeblock = "Your custom prompt here"
```

Compare to today's init.lua which requires defining every profile from
scratch. The batteries-included approach removes that boilerplate.


## Appendix C: What This Enables (Post-Harness)

Once the core is ready, the harness architecture docs
(format-strategy.md, routing.md, edit-pipeline.md) become implementable:

- **Provider-adaptive formats:** `vim.ai.extractors` maps format names
  to Rust engines. Adding str_replace or apply_patch is a new Rust engine
  + a Lua extractor entry.

- **Complexity routing:** Profile resolution checks complexity signals
  and selects between `contexts.selection.default` and
  `contexts.selection.complex`. Defined in Lua, executed in Rust.

- **Retry policy:** `vim.ai.extractors.codeblock.on_error` defines retry
  behavior. Snapshot into Rust, executed in the pipeline.

- **Community extractors:** A plugin can register a new extractor:
  ```lua
  vim.ai.edit_formats.register("hashline", {
      engine = "custom",
      prompt_key = "selection_hashline",
      parse = function(response) ... end,  -- Lua parser (v2)
  })
  ```
