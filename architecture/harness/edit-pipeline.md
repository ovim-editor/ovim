# Edit Pipeline Architecture

The full request/response pipeline from user action to applied edit, with
the current implementation and proposed improvements.

## Pipeline Overview

```
┌─────────────┐   ┌──────────────┐   ┌─────────────────┐   ┌──────────────┐
│ 1. Capture   │──→│ 2. Route     │──→│ 3. Build Context │──→│ 4. Construct │
│    Intent    │   │    to Profile│   │    Pack          │   │    Prompt    │
└─────────────┘   └──────────────┘   └─────────────────┘   └──────┬───────┘
                                                                   │
┌─────────────┐   ┌──────────────┐   ┌─────────────────┐   ┌──────▼───────┐
│ 8. Track     │←──│ 7. Apply     │←──│ 6. Extract      │←──│ 5. Call API  │
│    Region   │   │    Edit      │   │    Response     │   │              │
└─────────────┘   └──────────────┘   └─────────────────┘   └──────────────┘
```

## Stage 1: Capture Intent

**Current:** User enters Visual mode, selects text, presses the AI prompt
key, types an instruction, presses Enter.

**Data captured:**
```rust
AiSelectionSnapshot {
    start_line, start_col,
    end_line, end_col,
    start_char, end_char,    // absolute rope offsets
    selected_text: String,
    mode_before_prompt: Mode, // Visual or VisualLine
    anchor_line: usize,
}
```

**Proposed addition — intent classification:**

Before routing, cheaply classify the user's instruction:

| Signal | Rename | Fix | Refactor | Explain |
|--------|--------|-----|----------|---------|
| Prompt contains "rename" | ✓ | | | |
| Diagnostics overlap selection | | ✓ | | |
| Prompt contains "refactor"/"extract" | | | ✓ | |
| Prompt contains "explain"/"why" | | | | ✓ |
| Selection > 20 lines | | | ✓ | |

This classification feeds into routing (Stage 2) and context selection
(Stage 3). It's a heuristic lookup, not an LLM call.

**Files:** `ovim-core/src/editor/ai_integration.rs`


## Stage 2: Route to Profile

**Current:** Static routing via `contexts` config in Lua:

```lua
contexts = {
    selection = "gpt_4_1_mini",  -- cheap/fast for selection edits
    chat = "gpt_5_2",           -- capable for chat
    query = "gpt_5_2",
}
```

The profile determines everything downstream: provider, model, extraction
strategy, context budget, agent mode.

**Proposed improvements:**

### 2a. Complexity-based sub-routing

Within the `selection` context, route based on intent classification:

```
Simple (rename, format):
  → model: gpt-4.1-mini
  → reasoning_effort: none
  → context: FastPath (local window only)

Moderate (fix diagnostic, add error handling):
  → model: gpt-4.1-mini
  → reasoning_effort: none
  → context: Hybrid (local + related slices)

Complex (refactor, extract function):
  → model: gpt-5.2
  → reasoning_effort: low
  → context: Hybrid (expanded budget)
```

### 2b. Config schema extension

```lua
contexts = {
    selection = {
        default = "gpt_4_1_mini",
        complex = "gpt_5_2",       -- escalation profile
        complexity_threshold = 20,  -- lines or diagnostic count
    },
}
```

**Files:** `ovim-core/src/ai/config.rs`, `ovim-core/src/lua/editor_bridge.rs`


## Stage 3: Build Context Pack

**Current implementation:**

```rust
AiContextPack {
    selection: String,               // selected text
    surrounding: Vec<CodeSlice>,     // ±6 lines
    symbol_facts: Vec<SymbolFact>,   // LSP symbols in window (≤12)
    diagnostics: Vec<DiagnosticFact>,// overlapping diagnostics (≤12)
    related_slices: Vec<CodeSlice>,  // expanded related code (Hybrid)
}
```

**Agent modes control expansion:**
- FastPath: Local window only. ~500-2,000 tokens.
- Hybrid: Local + iterative symbol expansion. ~2,000-8,000 tokens.
- ReactOnly: Maximum expansion. ~8,000-24,000 tokens.

**Pruning order (when over budget):**
1. Related slices (drop from end)
2. Symbol facts
3. Diagnostics
4. Surrounding window (line by line)

**Proposed improvements:**

### 3a. Task-adaptive context selection

Based on intent classification from Stage 1:

| Intent | Symbols | Diagnostics | Surrounding | Related |
|--------|---------|-------------|-------------|---------|
| Rename | Referenced only | Skip | Minimal | Skip |
| Fix | Referenced | All overlapping | Standard | Error-related |
| Refactor | All in scope | All | Extended | Type hierarchy |
| Explain | All visible | All | Maximum | Broad |

### 3b. Semantic symbol ranking

Currently: Include all symbols in the surrounding window.
Proposed: Rank by reference count within the selection. Symbols referenced
in the selected code are signal; unreferenced nearby symbols are noise.

### 3c. Pre-prompt context validation

Before constructing the prompt, verify the context matches the intent:
- User says "fix the type error" → check for type error in diagnostics
- User says "rename to foo" → check if the name exists in selection
- If mismatch, surface to user: "No type error found in selection"

This prevents the model from hallucinating problems that don't exist.

**Files:** `ovim-core/src/editor/ai_context.rs`, `ovim-core/src/editor/ai_agent.rs`


## Stage 4: Construct Prompt

**Current system prompts (per extraction strategy):**

```
Json:      "Return JSON: {replacement, top_insertions, log}. Only valid JSON."
Codeblock: "Return ONLY code inside a fenced code block."
Raw:       "Return ONLY code. No markdown, no fences."
```

**Current user prompt template:**

```
Edit the selected text based on the instruction.
Instruction: {prompt}

File: {file_path}
Language: {language}
Extraction strategy: {strategy}

Selected text:
```{language}
{selected_text}
```

Nearby symbols:
- {name} [{kind}] at {line}:{col}

Diagnostics overlapping selection:
- {message} ({severity} at {line}:{range})

Context slice [{label} lines {start}-{end}]:
```{language}
{content}
```
```

**Proposed improvements:**

### 4a. Intent-specific system prompt augmentation

Based on classification from Stage 1, append task-specific instructions:

```
Rename: "Rename consistently. Preserve all references. Do not change behavior."
Fix:    "Address the diagnostic(s). Preserve existing behavior."
Refactor: "Maintain behavior. Improve structure. Keep the same public API."
```

This addresses the AutoPrompter finding that 27% of failures stem from
missing operationalization guidance.

### 4b. Provider-specific prompt optimization

GPT-5.2 supports `text.verbosity` (low/medium/high). For selection edits,
`verbosity: low` reduces commentary. For chat, `verbosity: high` gives
thorough explanations.

### 4c. Remove extraction strategy from user prompt

The current template includes `Extraction strategy: codeblock` in the user
prompt. This is redundant with the system prompt instruction and wastes
tokens. Remove it.

**Files:** `ovim-core/src/ai/provider.rs` (`build_user_prompt`)


## Stage 5: Call API

**Current:** Provider-specific HTTP requests to Chat Completions API.

**Provider-specific body differences:**

| Feature | OpenAI | Anthropic | Ollama |
|---------|--------|-----------|--------|
| System prompt | messages[0].role=system | body.system | messages[0].role=system |
| Max tokens | max_completion_tokens | max_tokens (required) | max_tokens |
| JSON mode | response_format: json_object | N/A | N/A |
| Temperature | Only at reasoning_effort=none | Always | options.temperature |
| Streaming | stream: true, SSE | stream: true, SSE | stream: true, NDJSON |

**Proposed improvements:**

### 5a. Reasoning effort parameter (OpenAI)

```rust
if profile.provider == OpenAi {
    if let Some(effort) = profile.reasoning_effort {
        body["reasoning"] = json!({ "effort": effort });
        // temperature/top_p not allowed unless effort == "none"
        if effort != "none" {
            body.as_object_mut().unwrap().remove("temperature");
        }
    }
}
```

### 5b. Verbosity parameter (OpenAI GPT-5.2+)

```rust
if profile.provider == OpenAi {
    if let Some(verbosity) = profile.verbosity {
        body["text"] = json!({ "verbosity": verbosity });
    }
}
```

### 5c. Better error extraction

Current `send_streaming()` already parses error bodies. For non-streaming
`request_openai()`, the error handling should match:

```rust
let status = response.status();
if !status.is_success() {
    let body = response.text().await.unwrap_or_default();
    let detail = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(String::from))
        .unwrap_or(body);
    anyhow::bail!("{label} returned {status}: {detail}");
}
```

**Files:** `ovim-core/src/ai/provider.rs`


## Stage 6: Extract Response

**Current strategies:**

```
Json:      Parse {"replacement": "...", "top_insertions": [...], "log": [...]}
           Fallback: look for JSON inside a fenced code block
Codeblock: Extract first ```...``` block
Raw:       Use entire response verbatim
```

**Proposed improvements:**

### 6a. Codeblock language tag validation

Currently extracts any ```` block. Should validate that the language tag
(if present) matches the file's language. A model returning a
````markdown` block when editing Rust is probably an explanation, not code.

### 6b. Elision detection

Models sometimes emit `// ... rest of function` or `/* remaining code */`
inside codeblocks. Detect these patterns and either:
- Warn the user ("Model may have elided code")
- Re-prompt with "Do not use placeholder comments. Include all code."

### 6c. Retry on extraction failure

If extraction fails (no code block found, invalid JSON), construct an
error feedback message and retry once:

```
Your previous response could not be parsed.
Error: No fenced code block found in response.
Please respond with ONLY the replacement code inside a ```{language} block.
```

This turns a hard failure into a recoverable retry, following the Aider
pattern of detailed error feedback.

**Files:** `ovim-core/src/ai/extract.rs`


## Stage 7: Apply Edit

**Current:**
1. Remove blocking AI lock
2. Delete old selection text
3. Insert replacement text at selection start
4. Insert top_insertions at file start
5. Normalize indentation (restore base indent if model dedented)
6. Adjust cursor position through edit offsets
7. Push undo entry
8. Add tracking lock (non-blocking, for UI highlights)
9. Refresh syntax and diagnostics

**Normalization:**
- Detect base indentation of original text
- If model dedented (common with codeblock extraction), reindent
- Preserve trailing newline shape

**Proposed improvements:**

### 7a. Post-edit syntax validation

After applying the edit, run tree-sitter incremental parse. If the edit
introduces syntax errors that didn't exist before, mark the region as
"applied with warnings" rather than silently accepting.

### 7b. Diff preview before apply

For edits that change > 10 lines, show a diff preview in the hover panel
before auto-applying. Let the user accept/reject.

**Files:** `ovim-core/src/editor/ai_integration.rs`


## Stage 8: Track Region

**Current:**
```rust
AiEditRegion {
    status: Running | Generated | Failed | Cancelled,
    prompt, original_text, generated_text,
    profile_name, provider_label,
    extraction, reasoning_lines, raw_output,
    created_at, updated_at,
}
```

User can hover (K) on the region to see reasoning, accept, revert, or
retry the edit. Regions persist until manually cleared.

**No changes proposed.** This is well-designed.


## Implementation Priority

| Change | Effort | Impact | Priority |
|--------|--------|--------|----------|
| 5a. Reasoning effort param | Small | Medium | P1 (config plumbing) |
| 5b. Verbosity param | Small | Low | P1 (same plumbing) |
| 4c. Remove extraction from user prompt | Tiny | Low | P1 |
| 6c. Retry on extraction failure | Medium | High | P1 |
| 5c. Better error extraction (non-streaming) | Small | Medium | P1 |
| 3b. Semantic symbol ranking | Medium | Medium | P2 |
| 6b. Elision detection | Medium | Medium | P2 |
| 1. Intent classification | Medium | Medium | P2 |
| 3a. Task-adaptive context | Large | High | P2 (depends on 1) |
| 4a. Intent-specific prompts | Small | Medium | P2 (depends on 1) |
| 7a. Post-edit syntax validation | Medium | High | P3 |
| 6a. Codeblock language validation | Small | Low | P3 |
| 2a. Complexity-based sub-routing | Large | Medium | P3 (depends on 1) |
| 7b. Diff preview | Large | Medium | P3 |
