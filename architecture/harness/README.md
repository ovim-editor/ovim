# Harness Architecture

The harness is everything between the user's intent and the applied edit.
It's the single biggest lever on AI edit quality — bigger than model
selection, bigger than prompt engineering.

## What This Folder Covers

```
README.md            ← You are here. Overview and principles.
edit-pipeline.md     ← The full request/response pipeline.
format-strategy.md   ← Which edit format for which model × context.
routing.md           ← How tasks route to models, configs, and strategies.
```

## Why "Harness"

The term comes from test harnesses — the scaffolding around the thing being
tested. An LLM harness is the scaffolding around the model: how you
construct the prompt, what format you ask for, how you extract the result,
how you recover from errors.

Recent research (The Harness Problem, Feb 2026) demonstrated that changing
only the edit format — zero model changes — swung success rates from 6.7%
to 68.3% on the same model. The harness matters as much as the model.

## Core Principles

### 1. Match complexity to task

Selection edits are simple. Chat-driven refactors are complex. The harness
should give each the right amount of machinery:

| Context | Reasoning | Format | Context Budget | Error Recovery |
|---------|-----------|--------|---------------|----------------|
| Selection edit | none/low | codeblock | 2,500-8,000 tokens | Single retry |
| Chat edit | low/medium | provider-adaptive | 24,000+ tokens | Layered fallback |
| Chat explanation | medium/high | freeform text | Full window | N/A |

### 2. Universal defaults, provider-specific optimization

Codeblock extraction works everywhere. It's the safe default. But GPT
models perform measurably better with apply_patch, and Claude models are
trained on str_replace. The harness should use provider-optimal formats
when available and fall back to universal formats otherwise.

### 3. Fail fast, fail informatively

When an edit fails to parse or match, the error message should tell the
model (on retry) or the user (on display) exactly what went wrong.
"No match found" is useless. "No match for old_str. Closest match is
line 42: `let x = process(...)`" is recoverable.

### 4. Context is a budget, not a firehose

Every token competes for the model's attention. Research shows context rot
degrades quality as volume increases. The harness should:
- Include only what's relevant to the specific task type
- Prune aggressively when the budget is tight
- Use observation masking (not summarization) for chat history

### 5. The edit format is not the extraction strategy

Ovim has three extraction strategies (Json, Codeblock, Raw) that determine
how the model's output is parsed. The edit format is what the model is
asked to produce. These are related but separable:

```
System prompt: "Return code in a fenced block"  ← edit format instruction
Extraction:     find first ```...``` block       ← parsing strategy
Application:    replace selection with extracted  ← edit application
```

The format instruction must match the extraction strategy. A mismatch
(asking for JSON but extracting codeblock) is a common failure mode.

## Current State

Ovim's harness today:

```
User selects text → enters AiPrompt mode → types instruction
    ↓
Profile resolved (from contexts config or default)
    ↓
Context pack built:
  - Selected text
  - ±6 lines surrounding window
  - LSP symbols (≤12)
  - LSP diagnostics (≤12)
  - Related slices (Hybrid/ReactOnly modes)
  - Pruned to token budget
    ↓
Prompt constructed:
  - System prompt (per extraction strategy)
  - User prompt (instruction + context)
    ↓
API call (OpenAI/Anthropic/Ollama)
  - Provider-specific body format
  - response_format: json_object (if Json extraction)
  - max_completion_tokens (OpenAI) / max_tokens (others)
    ↓
Response extracted (Json / Codeblock / Raw)
    ↓
Edit applied:
  - Delete old selection
  - Insert replacement
  - Insert top_insertions (imports)
  - Normalize indentation
  - Push undo entry
```

## Research Foundation

The architecture in this folder is grounded in:

- **Diff-XYZ** (Dec 2025): search-replace is the best format for large
  models; no single format dominates universally.
- **The Harness Problem** (Feb 2026): format alone swings success rates
  10×; avoid line numbers; delimit old vs new clearly.
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
