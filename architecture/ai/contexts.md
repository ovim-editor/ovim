# Contexts and `open_chat`

## The Primitive: `vim.ai.open_chat(opts)`

Every chat interaction — built-in or custom — is a call to `open_chat`. This is the only entry point to the chat system. There is no separate "context" runtime concept.

```lua
vim.ai.open_chat({
    name = "architecture",          -- conversation key (resume or create)
    profile = "opus",               -- which model/tools/scope
    allow_edits = true,             -- false strips all mutation tools
    system_prompt = "...",          -- full system prompt override
})
```

### Parameters

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | `"chat"` or `"query"` | Conversation key. Same name + same buffer = resume. |
| `profile` | string | `default_profile` | Profile name to resolve model, tools, scope |
| `allow_edits` | boolean | `true` | When false, strips mutation/external tools from the toolset |
| `system_prompt` | string | `nil` | Full override of the system prompt template |
| `system_prompt_extra` | string | `nil` | Appended to the default template (ignored if `system_prompt` is set) |
| `initial_message` | string | `nil` | Sent as the first user message on new conversations |

### What `name` does

`name` is the conversation key. Each `(buffer_id, name)` pair maps to an independent `ConversationTree`. This is how multiple conversations coexist on the same buffer:

```lua
vim.ai.open_chat({ name = "chat" })           -- general editing conversation
vim.ai.open_chat({ name = "architecture" })    -- design discussion
vim.ai.open_chat({ name = "review" })          -- code review conversation
```

Calling `open_chat` with a name that already has a conversation tree resumes it. Calling with a new name creates a fresh tree.

### What `allow_edits` does

It's not a separate mode — it's a tool filter applied on top of the profile:

1. Strip all tools with `side_effect = "mutation"` or `side_effect = "external"`
2. Use a different default system prompt template (analysis-focused instead of editing-focused)
3. Remove the Buffer Edits zone from the UI (no diff overlays, no accept/reject)
4. Use blue/lavender bubbles instead of green (visual signal: information, not action)

Same chat panel, same conversation tree, same streaming. Just fewer tools and a different color.

## Contexts: Default Keybinding Configuration

Contexts exist only to configure what the built-in keybindings do. They are **not** a runtime concept and have no special behavior beyond resolving to `open_chat` calls.

```lua
vim.ai.setup({
    default_profile = "sonnet",

    -- These configure the built-in keybindings, nothing more
    contexts = {
        selection = { profile = "local" },
        chat = { profile = "opus" },
        query = { profile = "sonnet" },
    },

    profiles = { ... },
})
```

### How built-in keybindings resolve

The built-in keybindings are equivalent to:

```lua
-- <Space><Space> in normal mode
vim.keymap.set('n', '<Space><Space>', function()
    vim.ai.open_chat({
        name = "chat",
        profile = vim.ai.contexts.chat.profile or vim.ai.default_profile,
        allow_edits = true,
    })
end)

-- <Space>? in normal mode
vim.keymap.set('n', '<Space>?', function()
    vim.ai.open_chat({
        name = "query",
        profile = vim.ai.contexts.query.profile or vim.ai.default_profile,
        allow_edits = false,
    })
end)

-- <Space> in visual mode (selection context — uses the existing single-shot system)
-- This is the one context that doesn't use open_chat — it uses the existing
-- AiPrompt mode with the selection-context profile.
```

These are hardcoded in the editor (Rust-side leader handler), but the profile they use is configured through `vim.ai.contexts`.

### Context shorthand

Since contexts just configure a profile, they accept either a string or a table:

```lua
-- String shorthand (just the profile name)
contexts = {
    selection = "local",
    chat = "opus",
    query = "sonnet",
}

-- Table form (for future extensibility)
contexts = {
    selection = { profile = "local" },
    chat = { profile = "opus" },
    query = { profile = "sonnet" },
}
```

### Quick switching

Changing a context's profile at runtime:

```lua
vim.keymap.set('n', '<Leader>msl', function()
    vim.ai.contexts.selection = 'local'
end, "[M]odel [S]election [L]ocal")

vim.keymap.set('n', '<Leader>mco', function()
    vim.ai.contexts.chat = 'opus'
end, "[M]odel [C]hat [O]pus")
```

The status line reflects active context profiles:

```
 NORMAL  main.rs  ─────  sel:local  chat:opus  qry:sonnet  42:15
```

## Custom Entry Points

The power of `open_chat` as the primitive: users create their own entry points with custom keybindings. These are just as capable as the built-in ones.

```lua
-- Architecture discussions with opus
vim.keymap.set('n', '<Space>arch', function()
    vim.ai.open_chat({
        name = "architecture",
        profile = "opus",
        allow_edits = true,
        system_prompt = table.concat({
            "This is an architectural discussion.",
            "We will discuss how the system could work, then decide on a plan",
            "that should be saved in the @architecture folder at the root",
            "of the project as markdown files.",
            "For subsystems store the architecture docs under architecture/[module]/.",
        }, " "),
    })
end)

-- Code review (read-only, critical)
vim.keymap.set('n', '<Space>rev', function()
    vim.ai.open_chat({
        name = "review",
        profile = "opus",
        allow_edits = false,
        system_prompt = "Review the code for bugs, security issues, and "
            .. "design problems. Be critical. Cite specific lines.",
    })
end)

-- Documentation helper
vim.keymap.set('n', '<Space>doc', function()
    vim.ai.open_chat({
        name = "docs",
        profile = "sonnet",
        allow_edits = true,
        system_prompt = "Help write documentation. Focus on clarity and "
            .. "accuracy. Write doc comments in the code, not separate files.",
    })
end)

-- Quick question with a cheap model
vim.keymap.set('n', '<Space>qq', function()
    vim.ai.open_chat({
        name = "quick",
        profile = "haiku",
        allow_edits = false,
    })
end)
```

Each of these creates an independent conversation (keyed by `name`), uses its own profile, and has its own system prompt. The user gets the full chat UI with all features (branching, streaming, tree panel) for each one.

## The Other Primitive: `vim.ai.edit_selection(opts)`

`open_chat` handles conversational interactions. `edit_selection` handles single-shot visual edits. These are the two primitives — everything else is configuration.

```lua
vim.ai.edit_selection({
    profile = "local",              -- which model to use (default: from contexts.selection)
})
```

### Parameters

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `profile` | string | from `contexts.selection` | Profile name to resolve model, tools, scope |

### Why a separate primitive?

Selection edits are fundamentally different from chat:

- **Single-shot** (no conversation history, no branching)
- **Inline UI** (prompt line at the bottom, not a split panel)
- **Selection-scoped** (edits only the selected text)
- **Optimized for speed** (minimal context, no streaming UI overhead)

The user enters visual mode, selects text, types an instruction, and gets the result. No conversation tree, no message history, no navigation zones. This should feel instant — especially with cheap local models.

### How the built-in keybinding resolves

```lua
-- <Space> in visual mode
-- (This is hardcoded Rust-side, but equivalent to:)
vim.keymap.set('v', '<Space>', function()
    vim.ai.edit_selection({
        profile = vim.ai.contexts.selection or vim.ai.default_profile,
    })
end)
```

### Custom selection keybindings

Users can create alternative selection bindings with different profiles:

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

### Codebase integration

`edit_selection` maps to the existing `start_ai_prompt_from_visual()` and `submit_ai_prompt_job()` codepath. The only change: profile resolution goes through the opts table (or falls back to `vim.ai.contexts.selection`) instead of `ai_state.active_profile`.

## Entry Points in the Codebase

### `<Space><Space>` (chat) and `<Space>?` (query)

In `leader.rs`, `handle_first_leader_key`:

```rust
' ' => {
    // Resolve vim.ai.contexts.chat -> profile -> open_chat
    editor.open_ai_chat(ChatOpts {
        name: "chat".into(),
        profile: editor.ai_chat_context_profile("chat"),
        allow_edits: true,
        system_prompt: None,
        ..Default::default()
    })?;
    editor.reset_input_state();
}
'?' => {
    editor.open_ai_chat(ChatOpts {
        name: "query".into(),
        profile: editor.ai_chat_context_profile("query"),
        allow_edits: false,
        system_prompt: None,
        ..Default::default()
    })?;
    editor.reset_input_state();
}
```

### Custom keybindings (via Lua)

Lua calls to `vim.ai.open_chat(opts)` go through the `EditorBridge`, which queues a `ChatOpts` struct. The editor processes it on the next tick, entering `Mode::AiChat` with the resolved options.

### Selection (`<Space>` in visual mode)

Unchanged from current implementation. `visual_mode.rs` calls `editor.start_ai_prompt_from_visual()`, entering `Mode::AiPrompt`. Profile comes from `vim.ai.contexts.selection`.

## Exit

Double `<Esc>` (within 300ms), following the existing `last_escape_time` pattern:

- **First Esc**: Collapse sub-focus (tree panel -> text input, message scroll -> text input)
- **Second Esc**: Exit to Normal mode

State is preserved. Re-entering with the same `name` resumes the conversation.
