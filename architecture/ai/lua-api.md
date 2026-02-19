# Lua API

The `vim.ai` namespace exposes the AI subsystem to Lua configuration and plugins. The API follows the principle: **plugins provide pieces, users compose them**.

## Namespace Overview

```lua
-- The two primitives
vim.ai.open_chat({...})          -- Open/resume a chat session
vim.ai.edit_selection({...})     -- Single-shot visual selection edit

-- Configuration
vim.ai.setup({...})              -- One-time configuration (profiles, contexts, etc.)
vim.ai.contexts                  -- Table: read/write default keybinding profiles
vim.ai.default_profile           -- String: read/write the default profile name

-- Registration (for plugins)
vim.ai.models.register(...)      -- Register a model/provider endpoint
vim.ai.tools.register(...)       -- Register a tool
vim.ai.edit_formats.register(...)-- Register a custom edit format
vim.ai.profiles.register(...)    -- Register a profile (can also be done in setup())

-- Hooks
vim.ai.on_before_tool            -- Hook: called before tool execution
vim.ai.on_response               -- Hook: called after each AI response
```

## vim.ai.open_chat()

The core primitive. Every chat/query interaction is a call to this function. See [contexts.md](contexts.md) for the full design rationale.

```lua
vim.ai.open_chat({
    name = "architecture",          -- conversation key (resume or create)
    profile = "opus",               -- which model/tools/scope to use
    allow_edits = true,             -- false strips all mutation tools
    system_prompt = "...",          -- full system prompt override
    system_prompt_extra = "...",    -- appended to default (ignored if system_prompt is set)
    initial_message = "...",        -- sent as first user message on new conversations
})
```

### Parameters

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | `"chat"` | Conversation key. Same name + same buffer = resume. |
| `profile` | string | `default_profile` | Profile name to resolve model, tools, scope |
| `allow_edits` | boolean | `true` | When false, strips mutation/external tools |
| `system_prompt` | string | `nil` | Full override of the system prompt template |
| `system_prompt_extra` | string | `nil` | Appended to the default template |
| `initial_message` | string | `nil` | Sent as the first user message on new conversations |

### Conversation identity

Each `(buffer_id, name)` pair maps to an independent conversation tree. Multiple conversations can coexist on the same buffer:

```lua
vim.ai.open_chat({ name = "chat" })           -- general editing
vim.ai.open_chat({ name = "architecture" })    -- design discussion
vim.ai.open_chat({ name = "review" })          -- code review
```

### Custom keybindings

Users create custom entry points by binding `open_chat` with preset options:

```lua
vim.keymap.set('n', '<Space>arch', function()
    vim.ai.open_chat({
        name = "architecture",
        profile = "opus",
        allow_edits = true,
        system_prompt = "This is an architectural discussion. "
            .. "Save plans in the @architecture folder as markdown files.",
    })
end)

vim.keymap.set('n', '<Space>rev', function()
    vim.ai.open_chat({
        name = "review",
        profile = "opus",
        allow_edits = false,
        system_prompt = "Review the code critically for bugs and design issues.",
    })
end)
```

These are just as capable as the built-in keybindings — same chat UI, branching, streaming, everything.

## vim.ai.edit_selection()

The second primitive. Single-shot visual selection editing — the fast path. See [contexts.md](contexts.md) for the design rationale on why this is separate from `open_chat`.

```lua
vim.ai.edit_selection({
    profile = "local",              -- which model to use
})
```

### Parameters

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `profile` | string | from `contexts.selection` | Profile name to resolve model, tools, scope |

Must be called from visual mode (the selection is captured from the active visual selection). Enters `Mode::AiPrompt` where the user types an instruction and presses Enter.

### Custom selection keybindings

```lua
-- Quick edit with local model
vim.keymap.set('v', '<Space>l', function()
    vim.ai.edit_selection({ profile = "local" })
end)

-- Careful edit with opus
vim.keymap.set('v', '<Space>o', function()
    vim.ai.edit_selection({ profile = "opus" })
end)
```

## vim.ai.setup()

The primary configuration entry point. Called once in `init.lua`:

```lua
vim.ai.setup({
  default_profile = "sonnet",

  contexts = {
    selection = "local",
    chat = "opus",
    query = "sonnet",
  },

  profiles = {
    ["local"] = {
      model = "qwen2.5-coder:7b",
      tools = { "read_selection", "edit_selection" },
      scope = { files = "file" },
      edit_mode = "format",
      edit_format = "codeblock",
      temperature = 0.1,
      max_tokens = 1024,
    },

    sonnet = {
      model = "claude-sonnet-4-5",
      tools = {
        "read_file", "read_diagnostics", "read_symbols",
        "search_project", "edit_diff",
      },
      scope = { files = "project" },
      edit_mode = "tools",
      temperature = 0.3,
      max_tokens = 8192,
    },

    opus = {
      model = "claude-opus-4-6",
      tools = {
        "read_file", "read_diagnostics", "read_symbols", "read_ast",
        "search_project", "list_files",
        "edit_diff", "edit_ast",
      },
      scope = { files = "project", shell = true },
      edit_mode = "tools",
    },
  },
})
```

### Profile Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `model` | string | (required) | Registered model name or inline model ID |
| `tools` | string[] | `{}` | List of tool names available to this profile |
| `scope` | string or table | `"file"` | Capability scope (see below) |
| `edit_mode` | string | `"format"` | `"tools"` (function calling) or `"format"` (text parsing) |
| `edit_format` | string | `"codeblock"` | Format name when `edit_mode = "format"` |
| `permissions` | table | `{}` | Per-tool permission overrides |
| `temperature` | number | (from model) | Sampling temperature |
| `max_tokens` | number | (from model) | Maximum response tokens |
| `system_prompt_extra` | string | `nil` | Appended to the context-specific system prompt |

### Scope Shorthand

```lua
-- String shorthand (shell=false, network=false)
scope = "file"
scope = "project"

-- Table for full control
scope = { files = "project", shell = true, network = false }
```

## vim.ai.models.register()

Register a model (provider endpoint). Models are referenced by name in profiles.

```lua
vim.ai.models.register("deepseek-r1", {
  provider = "openai",                  -- API wire format: "openai" | "anthropic" | "ollama"
  model = "deepseek-reasoner",          -- Model ID sent to the API
  base_url = "https://api.deepseek.com/v1",
  api_key_env = "DEEPSEEK_API_KEY",     -- Environment variable name
  supports_tools = true,                -- Can this model handle function calling?
  supports_streaming = true,            -- Can this model stream responses?
  max_context_tokens = 128000,          -- Context window size (for budget display)
})
```

### Built-in Models

The editor ships with implicit registrations for common providers:

- `"openai"` provider: any model ID works if `OPENAI_API_KEY` is set
- `"anthropic"` provider: any model ID works if `ANTHROPIC_API_KEY` is set
- `"ollama"` provider: any model ID works if Ollama is running locally

So `model = "claude-sonnet-4-5"` in a profile works without explicit registration — the system infers the provider from the model ID prefix. Explicit registration is for custom endpoints, fine-tuned models, or non-standard providers.

### Model ID Resolution

```
1. Check registered models by exact name
2. If not found, try to infer provider:
   - "claude-*" or "claude_*" → anthropic
   - "gpt-*" or "o1-*" or "o3-*" → openai
   - anything else → ollama (local)
3. If inference fails, error
```

## vim.ai.tools.register()

Register a custom tool. See [tools.md](tools.md) for the full tool model.

```lua
vim.ai.tools.register("cargo_test", {
  description = "Run cargo tests with optional filter pattern",
  scope = "shell",
  side_effect = "external",
  params = {
    filter = {
      type = "string",
      optional = true,
      description = "Test name filter pattern",
    },
    release = {
      type = "boolean",
      optional = true,
      description = "Build in release mode",
    },
  },
  handler = function(params)
    local cmd = "cargo test"
    if params.filter then cmd = cmd .. " " .. params.filter end
    if params.release then cmd = cmd .. " --release" end
    return vim.fn.system(cmd)
  end,
})
```

### Parameter Types

| Type | Lua type | JSON Schema type | Scope-validated? |
|------|----------|-----------------|-----------------|
| `"string"` | string | string | No |
| `"integer"` | number | integer | No |
| `"boolean"` | boolean | boolean | No |
| `"filepath"` | string | string | **Yes** — validated against scope |
| `"line_number"` | number | integer | No |
| `"json"` | table | object | No |

### Handler Return Values

Handlers return a string (displayed to the model) or a table:

```lua
-- Simple: return a string
handler = function(params)
  return "test passed: 42 tests, 0 failures"
end

-- Structured: return a table (serialized to JSON for the model)
handler = function(params)
  return {
    output = "...",
    exit_code = 0,
    duration_ms = 1234,
  }
end

-- Error: return nil + error message
handler = function(params)
  return nil, "command failed: permission denied"
end
```

## vim.ai.edit_formats.register()

Register a custom edit format for text-based models (when `edit_mode = "format"`).

```lua
vim.ai.edit_formats.register("xml_edit", {
  -- Injected into the system prompt to instruct the model
  instruction = [[
When making code changes, express them as XML edit blocks:
<edit path="relative/path.rs" start="10" end="15">
replacement content here
</edit>
You may include multiple <edit> blocks in a single response.
]],

  -- Parser: extracts edits from the model's response text
  -- Returns a list of edit tables
  parser = function(response_text)
    local edits = {}
    for path, start_line, end_line, content in response_text:gmatch(
      '<edit path="(.-)" start="(%d+)" end="(%d+)">\n(.-)\n</edit>'
    ) do
      table.insert(edits, {
        file = path,
        start_line = tonumber(start_line),
        end_line = tonumber(end_line),
        content = content,
      })
    end
    return edits
  end,
})
```

### Built-in Edit Formats

| Name | Description | Best for |
|------|-------------|----------|
| `"diff"` | Unified diff blocks in fenced code | Models trained on git data |
| `"codeblock"` | Fenced code block = replacement | Simple single-region edits |
| `"range"` | JSON `{start, end, content}` blocks | Structured text models |
| `"full_file"` | Complete file content | Small files, simple models |
| `"search_replace"` | `{search, replace}` pairs | Rename-style edits |

## vim.ai.contexts

Reactive table for reading/writing the profiles used by the built-in keybindings. Contexts configure defaults — they are not a runtime concept. See [contexts.md](contexts.md).

```lua
-- Read current context profiles
print(vim.ai.contexts.selection)  -- "local"
print(vim.ai.contexts.chat)       -- "opus"
print(vim.ai.contexts.query)      -- "sonnet"

-- Switch at runtime (takes effect on next open_chat call)
vim.ai.contexts.selection = "opus"
vim.ai.contexts.chat = "gpt-codex"
```

Changes are reflected in the status line. The contexts table only affects the built-in keybindings (`<Space><Space>`, `<Space>?`, `<Space><Space>` in visual mode for chat, and `<Space>ai` in visual mode for inline edit). Custom keybindings that call `vim.ai.open_chat()` directly are not affected.

## vim.ai.profiles.register()

Register a profile outside of `setup()`. Useful for plugins that provide complete profiles:

```lua
-- plugins/my-ai-preset/init.lua
vim.ai.models.register("my-endpoint", { ... })
vim.ai.tools.register("my-tool", { ... })
vim.ai.profiles.register("my-preset", {
  model = "my-endpoint",
  tools = { "read_file", "edit_diff", "my-tool" },
  scope = { files = "project" },
  edit_mode = "tools",
})
```

The user can then reference this profile in their config:

```lua
-- init.lua
vim.ai.contexts.chat = "my-preset"
```

## Hooks

### vim.ai.on_before_tool

Called before every tool invocation. Can veto execution:

```lua
vim.ai.on_before_tool = function(call)
  -- call.tool: string (tool name)
  -- call.params: table (tool parameters)
  -- call.profile: string (profile name)
  -- call.chat_name: string (conversation name, e.g. "chat", "architecture")
  -- call.allow_edits: boolean

  -- Return true to allow, false + reason to block
  if call.tool == "cargo_test" and not call.params.filter then
    return false, "Please specify a test filter"
  end
  return true
end
```

### vim.ai.on_response

Called after each AI response (after edits are applied):

```lua
vim.ai.on_response = function(response)
  -- response.content: string (full response text)
  -- response.model: string (model that generated it)
  -- response.profile: string (profile name)
  -- response.chat_name: string (conversation name)
  -- response.allow_edits: boolean
  -- response.edits: table (list of edits applied)
  -- response.tool_calls: table (list of tool calls made)
  -- response.tokens_used: number (approximate)
end
```

## Plugin Authoring Pattern

A well-structured AI plugin:

```lua
-- plugins/rust-ai-tools/init.lua

-- 1. Register tools (capabilities)
vim.ai.tools.register("cargo_test", { ... })
vim.ai.tools.register("cargo_clippy", { ... })
vim.ai.tools.register("cargo_build", { ... })

-- 2. Optionally register a preset profile
vim.ai.profiles.register("rust-dev", {
  model = "claude-sonnet-4-5",
  tools = {
    "read_file", "read_diagnostics", "read_symbols",
    "edit_diff",
    "cargo_test", "cargo_clippy", "cargo_build",
  },
  scope = { files = "project", shell = true },
  edit_mode = "tools",
})

-- 3. Never set contexts or default_profile — that's the user's choice
```

The user then opts in:

```lua
-- init.lua
vim.ai.contexts.chat = "rust-dev"
```

## Backward Compatibility

The existing `~/.config/ovim/ai.toml` configuration continues to work. TOML profiles are loaded as the baseline, and Lua `vim.ai.setup()` can override or extend them. If both exist:

1. Load `ai.toml` profiles
2. Apply `vim.ai.setup()` on top (Lua wins on conflicts)
3. Lua-registered models/tools are available alongside TOML-configured ones

This allows gradual migration from TOML to Lua without breaking existing configs.
