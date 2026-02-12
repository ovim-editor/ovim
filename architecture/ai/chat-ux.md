# Chat UX Design

The chat UI is a split-panel interface that shows the buffer (with diff overlays) alongside a conversational message stream. It supports multi-turn editing with streaming responses, branching conversations, and per-hunk accept/reject.

## Screen Layout

```
┌──────────────────────────────────────────────────────────────┐
│ fn main() {                                                  │
│     let greeting = "Hello, world!";                          │
│-    println!("{}", greeting);                                │
│+    println!("Greeting: {}", greeting);                      │
│     process::exit(0);                                        │
│ }                                                            │
│                                                     [1 of 2] │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  ╭─ You ────────────────────────────────────────╮            │
│  │ prefix "Greeting:" to the println output     │            │
│  ╰──────────────────────────────────────────────╯            │
│                                                              │
│         ╭─ thinking ────────────────────────────────────╮    │
│         │ ▸ The println currently just prints the...    │    │
│         ╰───────────────────────────────────────────────╯    │
│                                                              │
│         ╭─ Claude ──────────────────────────────────────╮    │
│         │ Done. Added "Greeting: " prefix to the        │    │
│         │ println format string.                        │    │
│         ╰───────────────────────────────────────────────╯    │
│                                                              │
│  ╭─ You ────────────────────────────────────────╮            │
│  │ █                                            │            │
│  ╰──────────────────────────────────────────────╯            │
│                                                              │
│ ▸ claude-sonnet-4-5        context: 3.2k tokens    [⏎ send] │
└──────────────────────────────────────────────────────────────┘
```

**Split ratio**: Buffer ~40% top, chat ~60% bottom. Adjustable with `<C-Up>/<C-Down>` (future). Auto-sizes based on content when there are few messages.

## Navigation Zones

Five focus zones navigated with arrow keys:

```
              ┌─────────────────┐
    ↑         │  Buffer Edits   │  Navigate between diff hunks
              ├─────────────────┤
    ↑         │  Message History│  Scroll messages, Enter to fork
              ├─────────────────┤
 [DEFAULT] →  │  Text Input     │  Type your message
              ├─────────────────┤
    ↓         │  Model Selector │  Choose AI profile
              └─────────────────┘

 (C-t toggles)┌─────────────────┐
              │  Tree Panel     │  Navigate conversation branches
              └─────────────────┘
```

### Zone Behaviors

| Zone | Arrow Up | Arrow Down | Enter | Other keys |
|------|----------|------------|-------|------------|
| **Buffer Edits** | Previous diff hunk | Next hunk / → Messages | — | `y` accept, `n` reject, `a` accept all |
| **Message History** | Older message | Newer message / → Input | Fork from this message | — |
| **Text Input** | → Messages (or prev line if multi-line) | → Model Selector (or next line if multi-line) | Send message | Type characters |
| **Model Selector** | → Text Input | (stays) | Confirm selection | Left/Right cycle profiles |
| **Tree Panel** | Parent node | Child node | Load branch | Left/Right navigate siblings |

### Multi-line Input

The text input box supports multi-line via `<Shift-Enter>`. When multi-line:
- Up/Down move within the input text
- Arrow Up at the first line → escapes to Message History zone
- Arrow Down at the last line → escapes to Model Selector zone

The input box grows to fit content (up to ~5 visible lines), then scrolls internally.

### Full-Screen Message Editor (`<C-g>`)

When the input box isn't enough, `<C-g>` opens a full-screen scratch buffer for composing the message. This is the `$EDITOR` pattern — like `<C-x><C-e>` in bash or `q:` in Vim.

**Trigger**: `<C-g>` while focus is on the Text Input zone.

**Flow**:

```
1. User is typing in the chat input box
2. Presses <C-g>
3. Editor creates a scratch buffer:
   - Filetype: markdown (code fences get syntax highlighting)
   - Content: current input text (cursor position preserved)
   - Mode: Normal (full editor at your disposal)
4. User edits freely — motions, text objects, yank/paste, :read, etc.
5. User finishes:
   - ZZ or :wq  → content goes back to chat input, return to chat mode
   - :q! or :bd! → discard, return to chat with original input
```

**Visual**:

```
┌──────────────────────────────────────────────────────────────┐
│  1│ Refactor the authentication module:                      │
│  2│                                                          │
│  3│ 1. Extract the JWT validation into a separate function   │
│  4│ 2. Add rate limiting to the login endpoint               │
│  5│ 3. Here's the current signature I want to keep:          │
│  6│                                                          │
│  7│ ```rust                                                  │
│  8│ pub async fn validate_token(token: &str) -> Result<Claims│
│  9│ ```                                                      │
│ 10│                                                          │
│ 11│ Make sure the error types are consistent with the rest   │
│ 12│ of the codebase.                                         │
│~                                                             │
│~                                                             │
│~                                                             │
│ NORMAL  [AI Message]  ──────────────────────  ZZ to send 12:1│
└──────────────────────────────────────────────────────────────┘
```

**Why this matters**:
- Long, detailed prompts with code examples
- Yank code from the buffer, `<C-g>`, paste into the prompt, annotate
- Use `:read src/auth.rs` to inline file content into the prompt
- Full search/replace on prompt text before sending
- Vim motions for editing complex multi-paragraph instructions

**Implementation**: A scratch buffer with a `BufWriteCmd` handler that captures content on `:w`/`ZZ` and feeds it back to the chat input. The buffer name shows `[AI Message]` and the status line shows `ZZ to send` as a hint. Buffer is cleaned up on close.

## Visual Design

### Chat Bubbles

Rounded box-drawing characters with left/right placement:

```
User messages (left-aligned, cyan):
  ╭─ You ─────────────────────────────╮
  │ Make the function async            │
  ╰────────────────────────────────────╯

Agent messages (right-aligned, green):
       ╭─ Claude (sonnet-4.5) ────────╮
       │ I've converted the function   │
       │ to async and added awaits.    │
       ╰──────────────────────────────╯

Thinking blocks (right-aligned, dim, collapsed by default):
       ╭─ thinking ───────────────────╮
       │ ▸ The function currently...   │
       ╰──────────────────────────────╯

Error messages (right-aligned, red):
       ╭─ error ──────────────────────╮
       │ API request failed: 429      │
       ╰──────────────────────────────╯
```

### Color Scheme

| Element | Border | Header | Body | Background |
|---------|--------|--------|------|------------|
| User message | Cyan | Bold Cyan | White | Default |
| Agent message | Green | Bold Green | White | Default |
| Thinking | DarkGray | Dim Gray | Dim Gray | Default |
| Error | Red | Bold Red | Red | Default |
| Selected message | Yellow border | — | — | — |

### Left/Right Placement

User messages align to the **left margin** (indent 2 columns). Agent messages align to the **right margin** (right-edge minus message width minus 2). This iMessage-style layout creates instant visual distinction without reading the header.

Message width: max 70% of the panel width, with word wrapping within the bubble.

### Thinking Blocks

Thinking (chain-of-thought) blocks are **collapsed by default**, showing one line with a `▸` toggle indicator:

```
       ╭─ thinking ───────────────────╮
       │ ▸ The function currently...   │
       ╰──────────────────────────────╯
```

Press `Enter` on a focused thinking block (in Message History zone) to expand:

```
       ╭─ thinking ───────────────────╮
       │ ▾ The function currently      │
       │ takes a sync reference. I     │
       │ need to consider the call     │
       │ sites and ensure they all     │
       │ handle the Future correctly.  │
       ╰──────────────────────────────╯
```

### Streaming Animation

During streaming:
- Thinking tokens appear in a dim, expanding thinking bubble
- A subtle `···` animation at the end shows generation is ongoing
- Content tokens appear in a growing agent bubble
- When the response includes tool calls, they render as compact one-liners: `⚡ edit_diff(src/main.rs)` with a spinner while executing
- The input zone is disabled (grayed out) until the response completes

### Model Selector Bar

Bottom of the chat panel:

```
 ▸ claude-sonnet-4-5  │  gpt-4o  │  local/qwen     context: 3.2k  [⏎ send]
   ~~~~~~~~~~~~~~~
   highlighted
```

- Left/Right arrows cycle between available profiles
- `▸` marks the active selection
- `context: N.Nk` shows token usage (yellow when >75% budget, red when >90%)
- `[⏎ send]` is a visual hint (in chat), `[?]` in query mode

## Buffer Area (Top)

The buffer renders normally (syntax highlighting, line numbers, gutter) with additional overlays for AI edits:

### Diff Highlighting

When the AI produces edits, deleted lines show with red background, added lines with green background:

```
  10│ fn main() {
  11│-    println!("{}", greeting);          ← red tint, strikethrough
  12│+    println!("Greeting: {}", greeting); ← green tint
  13│ }
```

### Edit Counter

When navigating edits in the Buffer Edits zone, a badge shows position:

```
                                                     [1 of 3]
```

Right-aligned in the buffer area, dim style.

### Edit Navigation Keys (Buffer Edits zone)

| Key | Action |
|-----|--------|
| `↑` / `k` | Previous diff hunk |
| `↓` / `j` | Next diff hunk |
| `y` | Accept this hunk (edit stays, highlighting removed) |
| `n` | Reject this hunk (revert to original) |
| `a` | Accept all remaining hunks |
| `r` | Reject all remaining hunks |
| `Esc` / `↓` past last hunk | Return to Message History or Text Input |

## Read-Only Mode (`allow_edits = false`)

When `open_chat` is called with `allow_edits = false` (the built-in `<Space>?` binding, or any custom keybinding), the same layout is used with these modifications:

- Agent bubbles use **blue/lavender** borders instead of green
- Input prompt marker is `?` instead of `▸`
- No diff overlays in the buffer (code is context, not workspace)
- No edit counter, no Buffer Edits zone
- Bottom bar shows `[?]` instead of `[⏎ send]`
- Buffer gets ~60% height (the thing being asked about), chat gets ~40%
- Arrow Up from Text Input goes directly to Message History (no Buffer Edits zone)

This is not a separate mode — it's the same chat UI with mutation tools stripped. All other features work identically: conversation branching, streaming, `<C-g>` editor, tree panel, model selector.

## Tree Panel (`<C-t>`)

Toggled with `<C-t>` from any zone. Appears on the left side, replacing the file tree area:

```
╭─ Chat History ─────────╮┌─────────────────────────────────────┐
│                         ││ [buffer area]                       │
│ ● "prefix Greeting…"   ││                                     │
│ ├── ● Claude: "Done…"  │├─────────────────────────────────────┤
│ │                       ││                                     │
│ ├── ○ "actually make…" ││ [chat messages]                     │
│ │   └── ○ Claude: "…"  ││                                     │
│ │                       ││                                     │
│ └── ● "also add the…"  ││ [text input]                        │
│     └── ● Claude: "…"  ││                                     │
│            ▲ current    ││ [model selector]                    │
╰─────────────────────────╯└─────────────────────────────────────┘
```

- **●** (filled circle) = on the active branch
- **○** (hollow circle) = on an inactive branch
- Tree lines: `├──`, `└──`, `│`
- User nodes: Cyan text, message preview (~20 chars + ellipsis)
- Agent nodes: Green text (or blue for query), compact summary
- Arrow keys navigate, Enter loads a branch (replays/rewinds buffer state)
- `<C-t>` again or `Esc` closes the tree panel

See [conversation-tree.md](conversation-tree.md) for the data model.

## Entry/Exit

**Enter**: Any keybinding that calls `vim.ai.open_chat(opts)`. Built-in: `<Space><Space>` (chat), `<Space>?` (query). Custom: any user-defined keybinding.

**Exit**: Double `<Esc>` (within 300ms).
- First Esc: collapse sub-focus (tree → input, messages → input, buffer edits → input)
- Second Esc: exit to Normal mode

Conversation state is preserved per `(buffer_id, name)`. Re-entering with the same `name` resumes where you left off. See [contexts.md](contexts.md) for how `open_chat` manages conversation identity.

## Status Line

In normal mode, the status line shows active context profiles:

```
 NORMAL  main.rs [+]  ─────  sel:local  chat:opus  qry:sonnet  42:15
```

In chat mode, the status line shows the conversation name and model:

```
 AI CHAT  main.rs  ──────  architecture · opus (claude-opus-4-6)  3.2k  42:15
```

```
 AI CHAT  main.rs  ──────  review · sonnet (claude-sonnet-4-5)  ?  42:15
```

The conversation name (`architecture`, `review`, etc.) distinguishes custom chats. The `?` marker indicates `allow_edits = false`.
