# Context Engineering for Code Editing

How to manage the finite attention budget: what to include, what to exclude,
and how the best coding agents handle context.

Date: 2026-02-13

## 1. The Attention Budget Problem

LLMs have a finite "attention budget" that degrades with token volume.
Even models with 128k+ context windows experience **context rot**: as
tokens increase, recall accuracy for any specific piece of information
decreases. More context ≠ better results.

Anthropic's framing: treat context as "a precious, finite resource."
Every token competes for attention with every other token. The goal isn't
to maximize context — it's to maximize signal density.

Source: https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents


## 2. The Four Strategies (Anthropic, 2025)

### Write: System Prompts and Instructions

Craft at the right "altitude" — specific enough to guide behavior,
flexible enough for the model to apply heuristics.

**Structure:** Use clear section delimiters (XML tags, Markdown headers).
Separate background information, instructions, tool guidance, and output
format into distinct labeled sections.

**Key principle:** Aim for the minimal set of information that fully
outlines expected behavior. Every unnecessary instruction consumes
attention that could be spent on the actual code.

**Ovim's current approach:** System prompts are strategy-specific:
- JSON: "Return JSON with schema: {replacement, top_insertions, log}"
- Codeblock: "Return ONLY code inside a fenced code block"
- Raw: "Return ONLY code, no markdown, no fences"

These are tight and minimal — good. The user prompt template is more
verbose but carries essential context.

### Select: Tools and Examples

Curate a minimal, non-overlapping toolset. If a human can't tell which
tool to use in a given situation, an AI can't either.

**Include diverse, canonical examples rather than exhaustive edge cases.**
A few well-chosen examples are worth more than many similar ones (this
aligns with the ReAct findings — exemplar similarity drives performance).

**Ovim's chat tools:** `read_file`, `list_directory`, `grep_search`,
`diagnostics`, `hover_info`, `buffer_content`. These are well-scoped
and non-overlapping.

### Compress: Long-Horizon Management

Three approaches for when context grows:

1. **Compaction:** Summarize old conversation turns, keeping architecture
   decisions and unresolved issues, discarding redundant tool outputs.

2. **Structured note-taking:** Persistent memory outside the context
   window (like CLAUDE.md files). Enables continuity across sessions.

3. **Sub-agent architectures:** Delegate focused work to specialized
   agents that return condensed summaries (1,000-2,000 tokens) instead
   of keeping everything in one context.

### Isolate: Just-in-Time Retrieval

Maintain lightweight identifiers (file paths, symbol names) and load
data dynamically via tools. Discover context layer-by-layer through
exploration rather than pre-loading.

**Claude Code's model:** CLAUDE.md files load upfront for speed; glob and
grep enable runtime navigation. The filesystem itself serves as a
lightweight index.


## 3. JetBrains Research: Observation Masking vs Summarization (Dec 2025)

A direct comparison of two context management strategies for coding agents:

### Observation Masking

Replace older environment outputs (tool results, file contents) with
placeholders. Keep the agent's reasoning and actions intact.

**Results:**
- 52% cheaper on average (Qwen3-Coder 480B)
- Improved solve rates by 2.6% for largest model
- Outperformed summarization in 4 of 5 settings

### LLM Summarization

Use a separate model to compress older interactions into summaries.

**Results:**
- Summary generation cost: >7% of total cost per instance
- Agents ran ~15% longer (trajectory elongation problem)
- More expensive with worse or equal performance

### Key Finding

**Observation masking > LLM summarization for coding agents.** The added
cost and trajectory elongation of summarization outweigh its benefits.
The agent doesn't need to "remember" old tool outputs — it can re-read
files. What it needs is its own reasoning history and action sequence.

**Optimal config:** 10-turn observation window, most recent turns in full.

Source: https://blog.jetbrains.com/research/2025/12/efficient-context-management/


## 4. SWE-Pruner: Adaptive Context Pruning (Jan 2026)

For multi-turn agent tasks on SWE-Bench:

**Dynamic Goal Hints:** At each turn, the agent generates hints about what
it's trying to do based on its evolving reasoning. These hints guide
context selection — early turns focus on high-level navigation, later turns
on detailed debugging.

**Results:**
- 39% token reduction on SWE-Bench Verified (Claude Sonnet 4.5)
- Interaction rounds reduced by up to 26%
- Performance maintained or improved despite fewer tokens

**Key insight:** Context needs change over the course of a task. What's
relevant in turn 1 (project structure, module overview) is noise in turn 10
(specific function implementation, error trace).

Source: https://arxiv.org/pdf/2601.16746


## 5. Ovim's Context Pipeline: Current State and Analysis

### Selection Edits

Current context pack:
```
┌─────────────────────────────────┐
│ Selected text                   │
├─────────────────────────────────┤
│ Surrounding window (±6 lines)   │
├─────────────────────────────────┤
│ Nearby symbols (≤12)            │
├─────────────────────────────────┤
│ Overlapping diagnostics (≤12)   │
├─────────────────────────────────┤
│ Related slices (Hybrid mode)    │
│  - Symbol definitions nearby    │
│  - Iteratively expanded         │
└─────────────────────────────────┘
```

**Budget:** Pruned to `context_budget_tokens` (2,500 for Small tier,
8,000 for Mid, 24,000 for Frontier). Token estimation: chars / 4.

**Agent modes:**
- FastPath: Local window only. Cheapest, fastest. Good for simple edits.
- Hybrid: Local window + iteratively expanded related slices.
- ReactOnly: Maximum expansion, most related context.

**Drop order during pruning:**
1. Related slices (least specific)
2. Symbol facts
3. Diagnostics
4. Surrounding window (line by line)

### Chat (multi-turn)

Full conversation history serialized per provider format. System prompt
includes tool schemas. No explicit context management yet — relies on
the model's context window.

### Analysis

**What works well:**
- Selection context is tight and relevant
- Diagnostics and symbols provide genuine signal
- Budget enforcement prevents context bloat
- Drop order is reasonable (preserve most-local context)

**Gaps:**
- Chat has no compaction — context grows unbounded within a session
- No observation masking for tool results in chat
- Related slices are proximity-based, not semantically ranked
- No task-adaptive context (the same context for "rename" vs "refactor")


## 6. Prompt Construction Quality

### AutoPrompter Finding (Google, 2025)

27% of failed edits succeed when the prompt is automatically enhanced with
inferred missing information. Five categories of commonly missing info:

1. **Specifics** — concrete names, types, values
2. **Operationalization** — which approach to use
3. **Localization** — where to apply changes
4. **Codebase context** — framework/library conventions
5. **User intent** — what "better" or "simpler" means

**Ovim's template covers 3 of 5:**
- Localization: ✓ (file path, language, selected text with fences)
- Codebase context: ✓ (nearby symbols, diagnostics, surrounding code)
- Specifics: Partially (depends on user prompt quality)
- Operationalization: ✗ (no guidance on *how* to implement)
- User intent: ✗ (passed through verbatim from user)

**Opportunity:** For common operations (rename, extract function, fix lint),
Ovim could augment the user prompt with task-specific instructions.
"Rename" → add "Preserve all references. Do not change behavior."
"Fix" + diagnostics → add "Address the following diagnostic(s)."

Source: https://arxiv.org/html/2504.20196v2


## 7. Recommendations for Ovim

### Selection edits (short-term)

1. **Task-type hints in system prompt.** When diagnostics overlap the
   selection, bias the system prompt toward "fix this diagnostic."
   When the user prompt contains "rename" or "refactor," add operational
   hints. This addresses the AutoPrompter findings cheaply.

2. **Semantic symbol relevance.** Instead of including all nearby symbols,
   rank by reference count within the selection and surrounding window.
   Symbols referenced in the selected code are signal; nearby but
   unreferenced symbols are noise.

### Chat (medium-term)

3. **Observation masking.** After N turns (e.g., 10), replace old tool
   result content with placeholders like `[file content from turn 3 —
   re-read if needed]`. Keep reasoning and actions intact. The JetBrains
   research shows this is strictly better than summarization for cost
   and performance.

4. **Dynamic context budget.** Reduce context sent to the model as
   conversation length grows. Early turns: include full file contents.
   Later turns: include only changed regions and error traces.

### Both contexts (long-term)

5. **Task-adaptive context selection.** Use the user prompt (or a cheap
   classifier) to determine what kind of context is most useful:
   - Rename → current file only, no diagnostics
   - Bug fix → diagnostics + error traces + related definitions
   - Refactor → broader symbol graph + type hierarchy
   - Explain → maximize surrounding context

6. **Pre-prompt context validation.** Before sending the request, check
   if the context actually contains the information needed for the task.
   If the user says "fix the type error" but no type error diagnostic is
   in the context, the model will hallucinate one. Better to surface
   "No type error found in selection" to the user.

Sources:
- https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
- https://blog.jetbrains.com/research/2025/12/efficient-context-management/
- https://arxiv.org/pdf/2601.16746
- https://arxiv.org/html/2504.20196v2
