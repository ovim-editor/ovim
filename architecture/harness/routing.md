# Routing Architecture

How Ovim routes different tasks to different models, configurations, and
strategies. From static per-context routing to adaptive complexity-based
routing.

## Routing Levels

Ovim's routing operates at three levels, from coarse to fine:

```
Level 1: Context routing    (selection vs chat vs query)
Level 2: Complexity routing (simple vs moderate vs complex)
Level 3: Parameter tuning   (reasoning effort, verbosity, temperature)
```

### Level 1: Context Routing (Implemented)

The user configures which profile handles each context:

```lua
contexts = {
    selection = "gpt_4_1_mini",  -- cheap, fast
    chat = "gpt_5_2",           -- capable, slower
    query = "gpt_5_2",          -- capable (for :AI queries)
}
```

Each profile carries its full configuration: model, provider, temperature,
max_tokens, extraction strategy, context policy, agent mode.

**This is the Anthropic "routing" pattern** — classify input type and
direct to a specialized handler. The research validates it as one of the
most effective agent patterns.

### Level 2: Complexity Routing (Proposed)

Within each context, route based on task complexity signals.

**Signals available at routing time:**

| Signal | Source | Cost |
|--------|--------|------|
| Selection size (lines) | Editor state | Free |
| Diagnostic count | LSP state | Free |
| Prompt word count | User input | Free |
| Prompt keywords | User input | Free (regex) |
| Symbol count in scope | LSP state | Free |
| File language | Editor state | Free |

**No LLM call needed for classification.** These are all available before
the API request. A simple heuristic function suffices:

```rust
fn estimate_complexity(
    selection_lines: usize,
    diagnostic_count: usize,
    prompt_words: usize,
    has_refactor_keyword: bool,
) -> Complexity {
    if has_refactor_keyword || selection_lines > 30 || prompt_words > 20 {
        Complexity::High
    } else if diagnostic_count > 2 || selection_lines > 10 {
        Complexity::Medium
    } else {
        Complexity::Low
    }
}
```

### Level 3: Parameter Tuning (Proposed)

Based on complexity, adjust API parameters without changing the model:

| Parameter | Low Complexity | Medium | High |
|-----------|---------------|--------|------|
| reasoning_effort | none | none | low |
| verbosity | low | low | medium |
| context_budget | 2,500 | 4,000 | 8,000 |
| agent_mode | FastPath | FastPath | Hybrid |

This is the cheapest form of routing — same model, different parameters.
The inference-time scaling research shows this is often sufficient:
simple tasks gain nothing from reasoning tokens, complex tasks benefit
measurably.


## Routing Architecture

### Current: Static Lookup

```
User action → context name → profiles[contexts[context_name]]
```

Simple, predictable, zero overhead. The user controls everything.

### Proposed: Layered Resolution

```
User action
    ↓
Context name (selection / chat / query)
    ↓
Context config
    ├── Simple string → profile name (backward compatible)
    └── Object → {default, escalation rules}
    ↓
Complexity estimation (heuristic, no LLM call)
    ↓
Profile selection
    ↓
Parameter overlay (reasoning_effort, verbosity from complexity)
    ↓
Final AiProfileConfig (sent to pipeline)
```

### Config Schema

**Backward compatible — simple string still works:**

```lua
contexts = {
    selection = "gpt_4_1_mini",  -- simple string: use this profile always
}
```

**Extended form with complexity routing:**

```lua
contexts = {
    selection = {
        default = "gpt_4_1_mini",
        escalation = {
            profile = "gpt_5_2",
            when = {
                selection_lines_gt = 30,
                diagnostic_count_gt = 3,
                prompt_contains = { "refactor", "extract", "redesign" },
            },
        },
    },
    chat = "gpt_5_2",  -- chat doesn't need sub-routing
}
```

**Even simpler — just use parameter overlays:**

```lua
contexts = {
    selection = {
        profile = "gpt_4_1_mini",
        params = {
            -- Applied when complexity > threshold
            complex = {
                reasoning_effort = "low",
                context_budget = 8000,
            },
        },
    },
}
```


## Provider-Specific Parameters

### OpenAI (GPT-5.2+)

```rust
struct OpenAiParams {
    reasoning_effort: Option<ReasoningEffort>, // none|low|medium|high|xhigh
    verbosity: Option<Verbosity>,              // low|medium|high
    response_format: Option<ResponseFormat>,    // json_object (only with Json extraction)
}

enum ReasoningEffort { None, Low, Medium, High, XHigh }
enum Verbosity { Low, Medium, High }
```

**Constraints:**
- `temperature` only works when `reasoning_effort = none`
- `response_format: json_object` conflicts with high reasoning effort
- `max_tokens` → `max_completion_tokens` for GPT-5+ models

### Anthropic

```rust
struct AnthropicParams {
    // Anthropic uses extended thinking, not reasoning_effort
    // No direct equivalent — controlled via system prompt
    max_tokens: u32, // Required field
}
```

### Ollama

```rust
struct OllamaParams {
    // Temperature goes in options.temperature
    // No reasoning effort — model-dependent
    num_predict: Option<u32>,
}
```

### Abstraction

Rather than provider-specific structs, extend AiProfileConfig:

```rust
pub struct AiProfileConfig {
    // ... existing fields ...

    // New: provider-specific parameters (optional)
    pub reasoning_effort: Option<String>,  // "none"|"low"|"medium"|"high"|"xhigh"
    pub verbosity: Option<String>,         // "low"|"medium"|"high"
}
```

Applied in `apply_optional_params()`:

```rust
fn apply_optional_params(body: &mut Value, profile: &AiProfileConfig, ...) {
    // ... existing temperature/max_tokens logic ...

    // Reasoning effort (OpenAI only)
    if profile.provider == OpenAi {
        if let Some(ref effort) = profile.reasoning_effort {
            body["reasoning"] = json!({ "effort": effort });
            if effort != "none" {
                // temperature/top_p not allowed with reasoning
                body.as_object_mut().unwrap().remove("temperature");
                body.as_object_mut().unwrap().remove("top_p");
            }
        }
    }

    // Verbosity (OpenAI GPT-5.2+ only)
    if profile.provider == OpenAi {
        if let Some(ref verbosity) = profile.verbosity {
            body["text"] = json!({ "verbosity": verbosity });
        }
    }
}
```


## Cost Model

### Why routing matters for cost

| Model | Cost per 1M tokens (approx) | Latency |
|-------|----------------------------|---------|
| gpt-4.1-mini | $0.40 input / $1.60 output | ~500ms |
| gpt-5.2 (none) | $2 input / $8 output | ~800ms |
| gpt-5.2 (medium) | $2 input / $8 output + reasoning | ~2-4s |
| gpt-5.2 (high) | $2 input / $8 output + reasoning | ~5-10s |
| claude-sonnet | $3 input / $15 output | ~1-2s |

A simple rename on gpt-4.1-mini costs ~0.002 cents and takes 500ms.
The same rename on gpt-5.2 with medium reasoning costs ~0.02 cents and
takes 3 seconds. 10× cost, 6× latency, for identical quality.

The routing architecture exists to prevent this waste.

### Cascade routing (future optimization)

Start with the cheap model. If the result fails validation (syntax error,
extraction failure), escalate to the expensive model.

```
gpt-4.1-mini → apply edit → syntax check
    ├── pass → done (cheap and fast)
    └── fail → gpt-5.2 with reasoning → apply edit
```

This is the "cascade" pattern from RouteLLM (ICLR 2025). 85% cost
reduction with 95% quality retention.

**For Ovim:** This requires post-edit validation (tree-sitter syntax check).
Not a priority now, but the architecture should support it.


## Multi-Model Diversity (Future)

The test-time scaling research shows that running the same task on different
models and picking the best result achieves the highest quality:

| Configuration | Pass@4 |
|--------------|--------|
| GPT-4.1 only | 55.76% |
| GPT-4.1 + Claude-3-5 | 64.24% |
| 4 different models | 74.55% |

**For Ovim:** This is too expensive for selection edits but viable for
high-stakes chat operations. A "best-of-2" mode that runs the same prompt
on two configured models and picks the result that passes validation would
be a powerful feature.


## Implementation Plan

### Phase 1: Parameter Plumbing

1. Add `reasoning_effort` and `verbosity` to `AiProfileConfig`
2. Parse from Lua config (`editor_bridge.rs`)
3. Parse from TOML config (`config.rs`)
4. Apply in `apply_optional_params()` with provider guards
5. Handle the temperature/reasoning_effort constraint

**Files:**
- `ovim-core/src/ai/config.rs`
- `ovim-core/src/ai/types.rs`
- `ovim-core/src/ai/provider.rs`
- `ovim-core/src/lua/editor_bridge.rs`

### Phase 2: Complexity Heuristic

1. Add `estimate_complexity()` function
2. Wire into profile resolution path
3. Support parameter overlays in context config

**Files:**
- `ovim-core/src/editor/ai_integration.rs`
- `ovim-core/src/ai/config.rs`

### Phase 3: Cascade Routing

1. Add post-edit syntax validation via tree-sitter
2. Implement cascade: cheap model → validate → escalate if needed
3. Add `escalation` config support

**Files:**
- `ovim-core/src/editor/ai_integration.rs`
- `ovim-core/src/ai/config.rs`
