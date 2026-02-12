# AI Architecture

This directory documents the design of ovim's AI subsystem: an integrated conversational code editor with tool calling, branching conversations, and a Lua-configurable permission model.

## Design Principles

1. **Three orthogonal concerns**: Models (how to talk), Tools (what it can do), Scopes (what it can touch). These compose into Profiles. Profiles never leak across concerns.

2. **`open_chat` is the primitive**: Every chat/query interaction is a call to `vim.ai.open_chat(opts)`. The built-in keybindings (`<Space><Space>`, `<Space>?`) are just preset invocations. Users create custom entry points (architecture chat, code review, etc.) with their own keybindings.

3. **Contexts configure defaults, nothing more**: The `contexts` table in `vim.ai.setup()` configures which profile the built-in keybindings use. Contexts are not a runtime concept — they're sugar over `open_chat`.

4. **Plugins provide pieces, users compose them**: Plugins register tools and models. Users wire them into profiles with explicit permissions. A tool never decides its own permissions.

5. **Same edit pipeline, multiple representations**: Whether the model outputs unified diffs, full file replacements, AST transforms, or calls structured tools, all paths produce the same internal `(file, range, content)` tuples. Accept/reject/undo is shared.

## Documents

| Document | Contents |
|----------|----------|
| [scopes.md](scopes.md) | The scope type system, capability model, and runtime enforcement |
| [tools.md](tools.md) | Tool abstraction, built-in registry, side effects, user-defined tools |
| [contexts.md](contexts.md) | `open_chat` primitive, default keybinding contexts, `allow_edits` |
| [chat-ux.md](chat-ux.md) | Chat UI layout, navigation zones, visual design, `<C-g>` editor |
| [conversation-tree.md](conversation-tree.md) | Branching conversation data model, fork/replay |
| [lua-api.md](lua-api.md) | `vim.ai.*` API surface, plugin authoring patterns |
| [provider-layer.md](provider-layer.md) | Multi-turn streaming, edit modes, format extraction |

## Resolution Chain

Every AI interaction follows the same chain:

```
Entry point
  Keybinding calls vim.ai.open_chat(opts) or vim.ai.open_selection(opts)
    |
    v
Resolve opts
  opts.profile -> profiles[name]
  opts.allow_edits -> strip mutation tools if false
    |
    v
Tool filtering
  Remove tools that exceed profile scope
  If allow_edits=false: also remove all mutation/external tools
    |
    v
System prompt assembly
  opts.system_prompt (full override) OR default template + opts.system_prompt_extra
  + filtered tool definitions + buffer context
    |
    v
Conversation lookup
  opts.name -> find or create ConversationTree for (buffer_id, name)
    |
    v
Provider dispatch
  selection -> single-shot, chat -> multi-turn with full branch history
    |
    v
Response handling
  Tool calls -> permission check -> scope validation -> execute
  Format parsing -> extract edit tuples
    |
    v
Buffer application
  Diff overlay, accept/reject, undo integration
```

## Current State

The existing AI system (`ovim-core/src/ai/`, `editor/ai_*.rs`) implements single-shot selection edits with the three providers (OpenAI, Anthropic, Ollama), extraction strategies, context budgeting, and buffer region tracking. This architecture extends it with multi-turn chat, branching conversations, a tool permission model, and Lua configurability.
