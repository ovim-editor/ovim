# AI Implementation Plan

## Goal

Implement the architecture documented in `architecture/ai/` by evolving the current single-shot AI prompt system into a multi-turn, tool-capable, scoped, Lua-configurable chat subsystem while preserving existing editing behavior during migration.

## Current Baseline

- Existing system supports visual-selection prompt editing (`Mode::AiPrompt`) and async single-shot provider calls.
- AI-generated region tracking, accept/revert/retry/cancel, undo integration, and lock-based edit protection are already implemented.
- Provider support exists for OpenAI, Anthropic, and Ollama in non-streaming single-shot form.
- Current config is TOML profile-based and does not yet model tools/scopes/contexts as described in architecture docs.

## Desired End State

- Two primitives:
  - `vim.ai.open_chat(opts)` for multi-turn chat/query sessions
  - `vim.ai.edit_selection(opts)` for fast single-shot selection edits
- Profiles composed from orthogonal concerns: model endpoint, tool set, scope/capabilities, edit mode/format, permission policy.
- Tool execution pipeline with scope enforcement and permission checks.
- Conversation trees with branch/fork navigation per `(buffer_id, name)`.
- Streaming provider layer and unified chat UX (`Mode::AiChat`) with read-only variant.
- Backward compatibility with existing `ai.toml`.

## Implementation Strategy

Deliver in staged milestones so each stage is shippable and reduces risk. Keep `Mode::AiPrompt` and existing tests functional until replacement paths are complete. The key risk mitigation: split the transport layer (streaming) from the UX layer (chat UI) so bugs in one don't block the other.

## Milestones

### M1: Chat Shell (Blocking)

Get the full chat UX loop working with synchronous (non-streaming) responses. This validates the UI, zone navigation, conversation state, and renderer independently of streaming complexity.

#### Deliverables

- **Domain types** (`ovim-core/src/ai/chat_types.rs`):
  - `ChatOpts` (name, profile, allow_edits, system_prompt, etc.)
  - `ChatMessage` (role, content)
  - `ChatNode`, `ConversationTree` (append-only, no branching yet)
  - `ChatFocus` enum (TextInput, MessageHistory, ModelSelector)
- **Chat state** (`ovim-core/src/editor/ai_chat.rs`):
  - `AiChatState` with per-buffer conversation map keyed by `(buffer_id, name)`
  - Input buffer, cursor, focus tracking
  - Conversation resume on re-entry
- **Blocking provider call** (`ovim-core/src/ai/chat_provider.rs`):
  - `request_ai_chat_blocking()` — send full message history, receive complete response
  - Reuse existing `reqwest` client and provider dispatch (OpenAI/Anthropic/Ollama)
  - No streaming, no tool calling — just multi-turn text
- **`Mode::AiChat`** (`ovim-core/src/editor/input/ai_chat_mode.rs`):
  - Key handler dispatching by `ChatFocus` zone
  - Text input: character insertion, backspace, cursor movement, Enter to send
  - Message history: arrow up/down to scroll, highlight selected message
  - Model selector: left/right to cycle profiles
  - Zone transitions: arrow up/down to move between zones
  - `<Esc><Esc>` to exit (double-Esc pattern)
- **Chat panel renderer** (`ovim/src/ui/renderer/ai_chat.rs`):
  - Frame layout: buffer area (top) + chat area (bottom), split ratio
  - Message bubble rendering with box-drawing corners (`╭╮╰╯│─`)
  - Word wrapping within bubbles (max 70% panel width)
  - Left-aligned user bubbles (cyan), right-aligned assistant bubbles (green)
  - Color scheme: cyan/green/gray/red per role
  - Text input box with cursor
  - Model selector bar at bottom
  - Scroll management for message list when it exceeds the visible area
- **Full-screen message editor** (`<C-g>`):
  - Scratch buffer with markdown filetype
  - Content seeded from current input text
  - `ZZ`/`:wq` writes content back to chat input
  - `:q!` discards and returns to chat
  - Status line hint: `[AI Message]  ZZ to send`
- **Entry points**:
  - `<Space><Space>` in leader handler → `open_ai_chat(ChatOpts { name: "chat", allow_edits: true, .. })`
  - `<Space>?` in leader handler → `open_ai_chat(ChatOpts { name: "query", allow_edits: false, .. })`

#### Validation

- Unit tests for `ConversationTree` append and resume by `(buffer_id, name)`.
- Unit tests for `ChatOpts` resolution (profile lookup, `allow_edits` tool filtering).
- Input handler tests for zone transitions and text input.
- Integration test: open chat, type message, receive blocking response, see it rendered.

---

### M2: Streaming

Replace the blocking transport with streaming adapters. The chat UI already works from M1 — this milestone only changes how responses arrive.

#### Risk Note

SSE parsing is the highest-risk technical area. Anthropic's format uses `content_block_start`/`content_block_delta` events with multiple block types (text, thinking, tool_use). OpenAI streams partial `arguments` strings for tool calls that need accumulation. Ollama uses NDJSON. Each needs its own parser with careful error handling for mid-stream disconnects.

#### Deliverables

- **Stream types** (`ovim-core/src/ai/chat_types.rs`):
  - `StreamChunk` enum: `Thinking(String)`, `Content(String)`, `ToolCall{..}`, `ToolCallComplete{..}`, `Done`, `Error(String)`
- **Streaming provider adapters** (`ovim-core/src/ai/chat_provider.rs`):
  - `request_ai_chat_streaming()` — returns `mpsc::Receiver<StreamChunk>`
  - OpenAI SSE parser: handles `choices[0].delta.content`, `choices[0].delta.tool_calls`, `[DONE]`
  - Anthropic SSE parser: handles `content_block_start`, `content_block_delta`, `message_stop`, thinking blocks
  - Ollama NDJSON parser: handles `{"message":{"content":"..."},"done":false}`
  - Timeout and disconnect handling for all three
- **Partial response policy**:
  - On stream disconnect: keep partial content, append an error bubble ("Stream interrupted — partial response above"), enable retry
  - On parse error: same treatment — preserve what we have, surface the error
  - Never silently discard partial responses
- **Streaming render integration**:
  - Event loop polls `chat_rx` in `select!` alongside terminal input and tick
  - Each `StreamChunk` updates the in-progress assistant node and marks dirty
  - `···` animation at end of streaming content
  - Input zone grayed out during streaming
  - Thinking tokens render in a dim bubble (collapsed by default)
- **Thinking block UI**:
  - Collapsed by default (one-line preview with `▸`)
  - Enter on focused thinking block toggles expand/collapse (`▾`)

#### Validation

- Unit tests for each SSE/NDJSON parser with real response fixtures.
- Unit tests for partial response handling (mid-stream disconnect, malformed chunk).
- Integration test: send message, verify tokens arrive incrementally, verify final state.

---

### M3: Profile, Config, and Lua Surface

Introduce architecture-aligned configuration. Preserve existing TOML behavior.

#### Deliverables

- **Internal config model** (`ovim-core/src/ai/config.rs` evolution):
  - Profile struct gains: `tools`, `scope`, `edit_mode`, `edit_format`, `permissions`
  - `Capabilities` struct: `file_scope`, `shell`, `network`
  - Contexts table: `selection`, `chat`, `query` mapping to profile names
- **Compatibility loader**:
  - Existing `ai.toml` loads into new profile model (legacy fields map to equivalents)
  - Missing new fields get sensible defaults (scope=file, edit_mode=format, etc.)
- **Lua API** (`ovim-core/src/lua/ai_api.rs`):
  - `vim.ai.setup(opts)` — profiles, contexts, default_profile
  - `vim.ai.contexts` — reactive table (read/write context profile assignments)
  - `vim.ai.default_profile` — read/write
  - `vim.ai.models.register(name, opts)` — register a model endpoint
  - `vim.ai.profiles.register(name, opts)` — register a profile
  - `vim.ai.edit_formats.register(name, opts)` — register custom edit format
  - `vim.ai.open_chat(opts)` — Lua-callable, queues through EditorBridge
  - `vim.ai.edit_selection(opts)` — Lua-callable, queues through EditorBridge
- **Profile resolution for existing paths**:
  - `<Space>` in visual mode resolves through `vim.ai.contexts.selection`
  - `<Space><Space>` and `<Space>?` resolve through `vim.ai.contexts.chat` / `query`

#### Validation

- Config merge precedence tests (TOML baseline + Lua override).
- Lua API smoke tests (registration, profile resolution, context switching).
- Regression: existing `Mode::AiPrompt` tests still pass with new config layer.

---

### M4: Tool Registry and Scope Enforcement

Add the tool execution pipeline. Read-only tools first, then mutation tools.

#### Deliverables

- **Tool definitions** (`ovim-core/src/ai/tools/`):
  - `ToolDefinition` struct: name, description, required_scope, side_effect, parameters, handler
  - `ToolRegistry` with lookup and filtering
  - Built-in read tools: `read_file`, `read_selection`, `read_diagnostics`, `read_symbols`, `read_ast`, `search_project`, `list_files`
  - Built-in edit tools: `edit_range`, `edit_diff`, `edit_insert`, `edit_delete`, `edit_search_replace`
  - `vim.ai.tools.register()` for Lua-defined tools
- **Scope enforcement** (`ovim-core/src/ai/scope.rs`):
  - `Capabilities` struct with `validate_path()`, `check_shell()`, `check_network()`
  - `ScopeContext` (current_file, project_root, selection range)
  - Automatic `FilePath` parameter validation before handler runs
  - Scope intersection for `allow_edits=false` (strip mutation/external)
- **Permission policy** (`ovim-core/src/ai/permissions.rs`):
  - Side-effect-aware defaults (read=auto, mutation=confirm at project scope, external=confirm)
  - Profile-level overrides
  - `vim.ai.on_before_tool` hook point
- **Tool call loop in provider layer**:
  - Parse tool calls from streaming response (`ToolCallComplete` chunks)
  - Execute through permission → scope → handler pipeline
  - Send tool result back to model as `Tool` message
  - Continue conversation until model sends text (no more tool calls)
- **Provider tool schema generation**:
  - Convert `ToolDefinition` to OpenAI function schema
  - Convert `ToolDefinition` to Anthropic tool schema
  - Filter tools by profile scope before sending to model

#### Validation

- Unit tests for scope intersection, path validation, and edge cases (symlinks, `..` traversal).
- Unit tests for permission resolution (defaults, overrides, hook veto).
- Unit tests for tool filtering (profile scope, `allow_edits` stripping).
- Integration tests for tool call round-trip: model calls tool → execute → result back → model responds.
- Integration tests for denied tool calls: scope violation and permission denial.

---

### M5: Edit Application and Format Parsing

Support both edit modes (`tools` and `format`). Unify all edit outputs into a shared pipeline that flows through the existing undo-aware buffer system.

#### Deliverables

- **Edit tuple pipeline** (`ovim-core/src/ai/edit_apply.rs`):
  - `EditTuple` struct: file, start_line, end_line, new_content
  - Scope validation on each tuple's file path
  - Apply via existing buffer edit + undo group creation
  - Indentation normalization (preserve base indent)
- **Edit format registry** (`ovim-core/src/ai/edit_formats.rs`):
  - Built-in parsers: `diff`, `codeblock`, `range`, `full_file`, `search_replace`
  - Lua parser support via `vim.ai.edit_formats.register()`
  - System prompt injection of format instruction
- **Diff overlay rendering** (`ovim/src/ui/renderer/buffer.rs` extension):
  - Deleted lines: red background tint
  - Added lines: green background tint
  - Hunk navigation in Buffer Edits zone with `[1 of N]` counter
  - Accept/reject per hunk (`y`/`n`/`a`/`r` keys)
- **Buffer Edits zone** in chat mode:
  - Arrow up/down to navigate between hunks
  - Visual focus indicator on current hunk
  - Zone becomes available when `allow_edits=true` and edits exist
- **Hook integration**:
  - `vim.ai.on_before_tool` fires before edit tool execution
  - `vim.ai.on_response` fires after response with edit summary

#### Validation

- Parser tests for all built-in edit formats with realistic model output samples.
- End-to-end tests: chat produces edit → applies to buffer → accept/reject → undo works.
- Regression: `Mode::AiPrompt` selection edit path still works.
- Edge case tests: overlapping edits, out-of-range line numbers, empty diffs.

---

### M6: Conversation Branching and Tree Panel

Add branching tree behavior and the `<C-t>` tree panel. Until this milestone, conversations are linear (append-only).

#### Deliverables

- **Tree operations** (extend `ConversationTree`):
  - `fork_from(node_id)` — set reply point to historical node
  - `switch_to_branch(node_id)` — follow first-child to leaf
  - `active_branch()` — path from root to active leaf
  - `sibling_branches(node_id)` — children of parent
  - `common_ancestor(a, b)` — for branch transition computation
- **Branch transition logic**:
  - `compute_branch_transition()` — returns edits to revert + edits to apply
  - Reverse-apply from current leaf to common ancestor
  - Forward-apply from common ancestor to target leaf
  - Compound undo group for the transition
- **Fork UX in chat panel**:
  - In Message History zone, Enter on a user message sets fork point
  - Branch indicator `⑂ N` on messages with multiple children
  - Next send creates a sibling branch
- **Tree panel** (`ovim/src/ui/renderer/conversation_tree.rs`):
  - `<C-t>` toggles left sidebar (reuses file tree slot)
  - Tree rendering with `├──`, `└──`, `│` connectors
  - `●` (active branch) / `○` (inactive branch) node markers
  - Cyan for user nodes, green for assistant nodes
  - Message preview (~20 chars + ellipsis)
  - Arrow navigation, Enter to load branch
  - `▲ current` marker on active leaf

#### Validation

- Unit tests for tree operations: fork, switch, ancestor, transition path computation.
- Unit tests for branch transition edit sequences (revert + apply ordering).
- Integration tests: fork from message → send new message → switch back → buffer state correct.
- Renderer test: tree panel layout with multiple branches.

---

### M7: Polish and Hardening

Complete documented UX behaviors. Quality gates before considering this feature shippable.

#### Deliverables

- **Read-only visual variant** (`allow_edits=false`):
  - Blue/lavender agent bubbles instead of green
  - `?` prompt marker instead of `▸`
  - No Buffer Edits zone, no diff overlays
  - `[?]` badge in bottom bar
- **Status line integration**:
  - Normal mode: `sel:local  chat:opus  qry:sonnet` in status bar
  - Chat mode: `architecture · opus (claude-opus-4-6)  3.2k` with conversation name
  - `?` marker for read-only conversations
- **Streaming UX refinements**:
  - Tool call status rows: `⚡ edit_diff(src/main.rs)` with spinner
  - Content/thinking visual separation during stream
  - Smooth scroll-to-bottom on new content
- **Multi-line input polish**:
  - `<Shift-Enter>` for newline within input box
  - Input box grows up to ~5 lines, then scrolls
  - Up/Down escape to adjacent zone at first/last line
- **Error UX**:
  - Provider errors render as red error bubbles
  - Tool execution errors render inline with the tool call row
  - Network timeout shows a retry hint
- **Performance**:
  - Long conversation rendering (100+ messages): ensure no frame drops
  - Large streaming responses: ensure render stays responsive
  - Conversation tree with many branches: ensure tree panel doesn't lag

#### Validation

- Renderer snapshot tests for all visual states (streaming, error, read-only, multi-branch).
- Keyboard behavior tests for all zone transitions and edge cases.
- Performance benchmarks for long conversations and large responses.
- Manual QA checklist: full workflow through chat, query, selection, custom keybindings.

---

## Cross-Cutting Requirements

### Compatibility

- Keep current `Mode::AiPrompt` and visual selection flow working through `vim.ai.edit_selection` and existing visual `<Space>` behavior during the entire migration.
- Do not break existing `ai.toml` users. Legacy profiles load into the new config model with sensible defaults.

### Reliability

- Never apply edits from tools or format output without scope validation.
- All provider, parse, and tool failures must degrade gracefully in the UI — show an error bubble, preserve partial content, allow retry.
- Preserve undo semantics for AI edits and branch transitions. Each AI edit creates an undo group. Branch switches create compound undo groups.

### Security

- Enforce capability boundaries before tool handler execution, not after.
- `allow_edits=false` must strip mutation and external tools regardless of profile configuration.
- Shell and network capabilities must be explicitly granted in the profile scope. Default is denied.
- Lua tool handlers run in the Lua sandbox with access only to `vim.fn.*` and `vim.api.*`.

### Observability

- Trace logs for:
  - Profile resolution (which profile, which context, fallback chain)
  - Tool filtering decisions (which tools removed, why)
  - Scope and permission denials (tool name, parameter, violation)
  - Stream lifecycle events (connect, first chunk, disconnect, error, done)
  - Branch transitions (from leaf, to leaf, ancestor, revert count, apply count)

### File Organization

New code should follow existing patterns and the CLAUDE.md guidance on file sizes:

```
ovim-core/src/ai/
├── chat_types.rs           # ChatOpts, ChatMessage, StreamChunk, ChatNode, ConversationTree
├── chat_provider.rs        # Multi-turn blocking and streaming provider calls
├── scope.rs                # Capabilities, FileScope, ScopeContext, validation
├── permissions.rs          # Permission policy, side-effect defaults, hook dispatch
├── edit_apply.rs           # EditTuple normalization and buffer application
├── edit_formats.rs         # Built-in format parsers, Lua parser support
├── tools/
│   ├── mod.rs              # ToolRegistry, ToolDefinition, ToolParam
│   ├── builtin_read.rs     # read_file, read_diagnostics, read_symbols, etc.
│   └── builtin_edit.rs     # edit_range, edit_diff, edit_insert, etc.

ovim-core/src/editor/
├── ai_chat.rs              # AiChatState, per-buffer conversation map, focus tracking

ovim-core/src/editor/input/
├── ai_chat_mode.rs         # Key handler for Mode::AiChat

ovim-core/src/lua/
├── ai_api.rs               # vim.ai.* Lua namespace setup

ovim/src/ui/renderer/
├── ai_chat.rs              # Chat panel renderer (bubbles, input, selector)
├── conversation_tree.rs    # Tree panel sidebar widget
```

## Definition of Done

- `vim.ai.open_chat` and `vim.ai.edit_selection` both functional from Lua and built-in keybindings.
- Keybindings (`<Space><Space>`, `<Space>?`, visual `<Space>`) resolve via `vim.ai.contexts`.
- Custom keybindings work (e.g., `<Space>arch` with custom name/profile/system_prompt).
- Tool calls go through scope validation and permission enforcement.
- Chat supports multi-turn streaming with thinking/content separation.
- Conversation branching works: fork, switch, replay, undo coherence.
- `<C-g>` full-screen editor works for message composition.
- `allow_edits=false` correctly strips mutation tools and uses read-only visual variant.
- Existing `Mode::AiPrompt` selection AI tests pass unchanged.
- New tests pass for: streaming parsers, conversation tree, scope/permissions, edit formats, branch transitions.
- `ai.toml` backward compatibility verified.
