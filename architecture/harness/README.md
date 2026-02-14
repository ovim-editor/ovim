# Harness Architecture

The harness is everything between the user's intent and the applied edit.
It's the single biggest lever on AI edit quality — bigger than model
selection, bigger than prompt engineering.

## What This Folder Covers

```
README.md              <- You are here. Overview and principles.
builtin-sketch.lua     <- The full harness policy in Lua. Design source of truth.
roadmap.md             <- Implementation roadmap: 7 phases from current to sketch.
api-design.md          <- API design rationale (building blocks, decisions, tradeoffs).
edit-pipeline.md       <- Reference: the 8-stage request/response pipeline.
format-strategy.md     <- Reference: edit format research and strategy.
routing.md             <- Reference: routing levels and cost model.
```

**Start with `builtin-sketch.lua`.** It's the complete policy definition
that exercises every building block. The other docs provide research
context and architectural rationale.

## Why "Harness"

The term comes from test harnesses — the scaffolding around the thing being
tested. An LLM harness is the scaffolding around the model: how you
construct the prompt, what format you ask for, how you extract the result,
how you recover from errors.

Recent research (The Harness Problem, Feb 2026) demonstrated that changing
only the edit format — zero model changes — swung success rates from 6.7%
to 68.3% on the same model. The harness matters as much as the model.

## Core Principles

### 1. Lua is the policy, Rust is the engine

Lua decides WHAT to do: which model, which prompt, which format, which
key. Rust does HOW: HTTP, streaming, parsing, matching, rope operations.

The built-in Lua file (`builtin.lua`) ships inside the binary via
`include_str!()`. It runs before the user's init.lua. Everything in it
can be overridden.

### 2. Five building blocks

| Block | What it is | Lua API |
|-------|-----------|---------|
| **API Keys** | Named key configs (env_var/file) | `vim.api_keys.register()` |
| **Prompts** | Named system prompt strings | `vim.ai.prompts` table |
| **Formats** | Extensible edit format engines | `vim.ai.formats.register()` |
| **Context Policies** | How much context to gather | `vim.ai.context_policies` table |
| **Profiles** | Model + params + format + context | `vim.ai.profiles.register()` |

Plus two configuration surfaces:

| Config | What it is | Lua API |
|--------|-----------|---------|
| **Contexts** | Action -> profile mapping | `vim.ai.contexts.{name}` |
| **Chat** | Observation masking config | `vim.ai.chat` table |
| **Agent** | Safety rails for tool loops | `vim.ai.agent` table |

### 3. Batteries included, overridable at every layer

A user who never writes init.lua gets a working local Ollama setup. A
user who sets `OVIM_OPENAI_API_KEY` and adds three lines to init.lua
gets a tuned OpenAI experience. A power user can replace extraction
engines with Lua functions or register entirely new edit formats.

### 4. Match complexity to task

Selection edits are simple. Chat-driven refactors are complex. The harness
should give each the right amount of machinery:

| Context | Reasoning | Format | Context Budget | Error Recovery |
|---------|-----------|--------|---------------|----------------|
| Selection edit | none/low | codeblock | 2,500-8,000 tokens | Single retry |
| Chat edit | low/medium | provider-adaptive | 24,000+ tokens | Layered fallback |
| Chat explanation | medium/high | freeform text | Full window | N/A |

### 5. Context is a budget, not a firehose

Every token competes for the model's attention. Research shows context rot
degrades quality as volume increases. The harness should:
- Include only what's relevant to the specific task type
- Prune aggressively when the budget is tight
- Use observation masking (not summarization) for chat history

### 6. Formats are extensible

Built-in formats (codeblock, json, raw, apply_patch, str_replace) are
implemented in Rust for performance. But anyone can register a new format
in Lua — hashline ships this way, and researchers can experiment with
novel formats without recompiling ovim.

## Architecture Snapshot

```
┌─────────────────────────────────────────────────────────┐
│ Lua VM                                                   │
│                                                          │
│  vim.api_keys.register()   → bridge.api_key_registry     │
│  vim.ai.prompts = {}       → (read during sync)          │
│  vim.ai.formats.register() → bridge.format_registry      │
│  vim.ai.context_policies   → (plain tables, by reference) │
│  vim.ai.setup() / profiles → bridge.ai_profiles          │
│  vim.ai.chat = {}          → (read during sync)          │
│  vim.ai.agent = {}         → (read during sync)          │
│                                                          │
├──────────────────────────────────────────────────────────┤
│ EditorBridge (Mutex) — sync cycle each tick               │
│                                                          │
│  profiles      → ai_state.config.profiles                │
│  contexts      → ai_state.config.contexts                │
│  api_keys      → ai_state.api_key_registry               │
│  prompts       → ai_state.prompt_templates               │
│  formats       → ai_state.format_registry                │
│  chat config   → ai_state.chat_context_config            │
│  agent config  → ai_state.agent_loop_config              │
│                                                          │
├──────────────────────────────────────────────────────────┤
│ AiState (snapshot, read by async tasks)                   │
│                                                          │
│  Request path: profile → context → prompt → API call     │
│  Response path: extract → match → apply → syntax check   │
│  Retry path: error feedback → re-call → fallback format  │
│                                                          │
│  Never touches the Lua VM.                               │
└─────────────────────────────────────────────────────────┘
```

## Key Design Decisions

### Context as inline table, not registry

Context policies are plain Lua tables on the profile. No registry
indirection. Users extend builtins naturally with `vim.tbl_extend()`.
The pre-defined policies (`fast`, `hybrid`, `full`) are just tables
in `vim.ai.context_policies` that profiles reference directly.

### Per-profile prompt overrides

Different models need different prompts. A 7B local model needs terse,
imperative instructions. A frontier model benefits from detailed anti-
elision guidance. Profiles carry `edit_prompt`, `chat_prompt`, and
`chat_edit_prompt` fields that override the global prompt resolution
chain when set.

### Provider-adaptive chat edit formats

Each model family gets the edit format it was trained on:
- OpenAI: `apply_patch` (post-trained on this format)
- Anthropic: `str_replace` (Claude Code's native format)
- Ollama: `codeblock` (safest for local models)

When `chat_edit_format` is omitted, the harness infers the right format
from the provider. Codeblock is the universal fallback.

### Hashline as a Lua-implemented format

Hashline (from "The Harness Problem" research) ships as a registered
Lua format rather than a Rust built-in. This demonstrates the format
extensibility system and lets researchers iterate on the format without
recompiling. Lua does the parsing; Rust does the buffer application.

### Observation masking for chat

Based on JetBrains research (Dec 2025): observation masking beats LLM
summarization for coding agents — 52% cheaper, 2.6% better solve rates.
Old tool outputs are replaced with placeholders in the API serialization.
The full conversation is always kept in memory for display.

### Agent limits as safety rails

`max_tool_calls = 50` is a hard ceiling to prevent runaway loops, not a
tuning knob. The ideal is cost-based limits (future work). The agent
should run until done, bounded by spend, not by arbitrary iteration counts.

### Project context files (.ovim.md, AGENTS.md, CLAUDE.md)

AutoPrompter (Google, 2025) found that 27% of failed edits succeed when
augmented with missing codebase context. Project context files provide
persistent, structured, project-specific knowledge — conventions,
architecture, constraints — that the model can't infer from code alone.

ovim supports `.ovim.md` (ovim-specific), `AGENTS.md` (provider-agnostic),
and `CLAUDE.md` (widely adopted). Files are loaded hierarchically from
the current directory up to the repo root, with deeper files taking
priority. Content is budget-aware and injected into the system prompt.

## Research Foundation

The architecture in this folder is grounded in:

- **Diff-XYZ** (Dec 2025): search-replace is the best format for large
  models; no single format dominates universally.
- **The Harness Problem** (Feb 2026): format alone swings success rates
  10x; avoid line numbers; delimit old vs new clearly. Hashline format.
- **Building Effective Agents** (Anthropic, Dec 2024): start simple, add
  complexity only when needed. Five composable patterns.
- **Context Engineering** (Anthropic, 2025): Write/Select/Compress/Isolate.
  Context is a finite resource.
- **JetBrains Research** (Dec 2025): observation masking > summarization
  for coding agents.
- **AutoPrompter** (Google, 2025): 27% improvement by inferring missing
  prompt information.
- **ROCODE** (ICSE 2025): mid-generation error detection achieves 99.1%
  compilation rate.

Full bibliography: `research/bibliography.md`
