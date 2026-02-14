# LLM Harness Configuration for Code Editing

Research notes on how LLM configuration choices affect code editing quality,
with recommendations for Ovim's AI integration.

Date: 2026-02-13

## 1. Reasoning Effort: Match to Task Complexity

GPT-5.2 defaults to `reasoning_effort=none`, positioned as equivalent to
gpt-4.1 behavior. The right reasoning level depends on task complexity.

### Levels and Use Cases

| Level | Latency | Best For | Notes |
|-------|---------|----------|-------|
| `none` | Lowest | Quick edits, renames, simple refactors | `temperature`/`top_p` supported |
| `low` | Low | Moderate code reasoning | `temperature`/`top_p` **unsupported** |
| `medium` | Medium | Multi-step reasoning, chat | Default for GPT-5.1 |
| `high` | High | Complex architecture, debugging | |
| `xhigh` | Highest | Hardest problems | GPT-5.2 only |

GPT-5.2 at `none` already substantially outperforms GPT-5.1 and GPT-4.1 on
coding benchmarks. The GPT-5.2 prompting guide recommends starting at `none`
and increasing only if quality is insufficient.

**Key constraint**: `temperature` and `top_p` parameters raise an error at
any reasoning effort other than `none`. Use `text.verbosity` and
`max_output_tokens` as alternatives.

### Benchmark Data

- SWE-Bench Pro: GPT-5.2 Thinking achieves 55.6% vs Claude Opus 4.5 at 52.0%
- GPT-5.2 code passes static analysis and security scans 42% more frequently
  than GPT-5.1
- Gains are most visible in the Thinking tier (medium+ reasoning)

### Recommendation for Ovim

Selection edits are short, focused tasks — `none` is appropriate. Chat
conversations benefit from `low` or `medium` for multi-step reasoning. The
config should support different reasoning levels per context.

Sources:
- https://platform.openai.com/docs/guides/latest-model
- https://cookbook.openai.com/examples/gpt-5/gpt-5-2_prompting_guide
- https://www.turingcollege.com/blog/gpt-5-2-review


## 2. The Harness Problem: Edit Format Matters as Much as Model Quality

The most important finding from recent research: **edit tool design matters as
much as model quality**.

### The Three Main Edit Formats

**apply_patch (unified diff)**
- Used by: OpenAI Codex, GPT models post-trained on it
- Mechanism: Model emits structured diffs with context lines
- Strengths: Token-efficient, supports multi-file edits
- Weaknesses: Catastrophic failure rates on non-GPT models (50.7% failure on
  Grok 4, 46.2% on GLM-4.7)
- Robustness: Falls back from exact match to trimmed whitespace to fully
  trimmed matching

**str_replace (exact text match)**
- Used by: Claude Code, Gemini CLI
- Mechanism: Find exact old text, swap in new text
- Strengths: Intuitive, works across model families
- Weaknesses: Model must reproduce every character including whitespace;
  breaks when target string appears multiple times

**codeblock / whole (full region replacement)**
- Used by: Aider (whole format), many harnesses
- Mechanism: Model returns complete replacement in a fenced code block
- Strengths: Most universal and reliable; simple extraction
- Weaknesses: Token-expensive for large edits; model may elide code with
  placeholder comments

### Quantitative Evidence

From "The Harness Problem" (Feb 2026), testing 16 models on 180 tasks:

- Switching from apply_patch to hashline improved Grok Code Fast from
  6.7% to 68.3% success rate
- apply_patch is the worst format for nearly every non-GPT model
- The author's novel "hashline" format (referencing lines by content hash)
  matched or beat str_replace for most models

Aider's research found that plain text formats work best, with fenced code
blocks being the most reliable across GPT-3.5 and GPT-4 families.

All successful formats share two principles:
1. Avoid line numbers (models get them wrong)
2. Clearly delimit original vs replacement code

### Recommendation for Ovim

Ovim's `codeblock` extraction is the right default for selection edits. The
selected region is usually small enough that token cost doesn't matter, and
the format is universal across model families.

For future chat-driven file edits, consider:
- `apply_patch` for GPT models (they're trained on it)
- `str_replace` for Claude/Gemini
- Model-adaptive format selection based on provider

Sources:
- https://blog.can.ac/2026/02/12/the-harness-problem/
- https://fabianhertwig.com/blog/coding-assistants-file-edits/
- https://aider.chat/docs/more/edit-formats.html
- https://aider.chat/docs/leaderboards/edit.html


## 3. Inference-Time Scaling: More Thinking Has Diminishing Returns

Research on inference-time compute scaling shows that additional reasoning
tokens help for complex tasks but have diminishing — sometimes zero — returns
on simple ones.

### Key Papers

**"Inference-Time Scaling for Complex Tasks" (Microsoft, arXiv 2504.00294)**
- Advantages of more reasoning tokens diminish as problem simplicity increases
- For straightforward edits, extra reasoning is wasted compute

**"S*: Test Time Scaling for Code Generation" (arXiv 2502.14382)**
- Test-time scaling can make non-reasoning models surpass reasoning models
- Harness and selection strategy matter as much as raw model capability

**"Are More Tokens Rational?" (arXiv 2602.10329)**
- Models don't always benefit from generating more tokens
- Adaptive resource allocation is key — spend reasoning tokens where they help

**"Faster and Better LLMs via Latency-Aware Test-Time Scaling" (EMNLP 2025)**
- Latency-optimal scaling is achieved by jointly optimizing parallelism
  strategies, not just adding sequential reasoning

### Implication

For selection edits (rename, refactor, fix lint error), `reasoning_effort=none`
is not just acceptable — it's optimal. The task is simple enough that reasoning
tokens add latency without improving quality. For chat (explain this code,
find the bug, suggest architecture), `low` or `medium` reasoning produces
measurably better results.

Sources:
- https://arxiv.org/abs/2504.00294
- https://arxiv.org/html/2502.14382v1
- https://arxiv.org/html/2602.10329
- https://aclanthology.org/2025.findings-emnlp.928.pdf


## 4. GPT-5.2 API Specifics

### Parameter Changes from GPT-4.1

| Parameter | GPT-4.1 | GPT-5.2 |
|-----------|---------|---------|
| Max tokens | `max_tokens` | `max_completion_tokens` |
| Reasoning | N/A | `reasoning.effort` (none/low/medium/high/xhigh) |
| Verbosity | N/A | `text.verbosity` (low/medium/high) |
| Temperature | Always supported | Only at `reasoning_effort=none` |

### Verbosity Control

Orthogonal to reasoning effort. Controls output length:
- `low`: Concise code, minimal commentary — good for selection edits
- `medium`: Default, balanced
- `high`: Thorough explanations, verbose code — good for chat

### Preambles

GPT-5.2 can generate brief explanations before tool calls ("why I'm calling
this tool"). Enabled via system prompt instruction. Improves tool-calling
accuracy without bloating reasoning overhead.

### Custom Tools

GPT-5.2 supports `type: "custom"` tools that accept freeform text input
(not just JSON). The model can send code, SQL, shell commands, etc. directly.
Context-free grammars (CFGs) can constrain outputs to specific syntaxes.

### Responses API vs Chat Completions

The Responses API (`/v1/responses`) passes chain-of-thought between turns,
leading to fewer reasoning tokens, higher cache hit rates, and lower latency.
Chat Completions (`/v1/chat/completions`) still works but doesn't get CoT
passthrough benefits. Ovim currently uses Chat Completions.

Sources:
- https://platform.openai.com/docs/guides/latest-model
- https://platform.openai.com/docs/models/gpt-5.2


## 5. Recommendations for Ovim

### Short-Term (current architecture)

1. **Per-context reasoning**: Selection edits at `none`, chat at `low` or
   `medium`. This requires adding a `reasoning_effort` field to profile config.

2. **Verbosity parameter**: Add `verbosity` to profile config. Use `low` for
   selection edits, `medium` for chat.

3. **Keep codeblock extraction for selection edits**: It's universal, reliable,
   and the selected region is small enough that token cost is negligible.

4. **`response_format: json_object`** only with `reasoning_effort=none`:
   Higher reasoning levels may conflict with forced JSON output.

### Medium-Term

5. **Model-adaptive edit formats**: When applying edits from chat, detect the
   provider and use the optimal format (apply_patch for OpenAI, str_replace
   for Anthropic).

6. **Responses API migration**: For multi-turn chat with OpenAI models,
   migrating from Chat Completions to the Responses API would improve
   intelligence and reduce latency via CoT passthrough.

7. **Preamble support**: Enable GPT-5.2 preambles in chat mode for
   transparency into tool-calling reasoning.

### Long-Term

8. **Adaptive reasoning**: Automatically scale reasoning effort based on
   task complexity signals (selection size, number of diagnostics, user
   prompt complexity).

9. **Custom tool with CFG constraints**: Define Ovim's edit format as a
   custom tool with a grammar constraint, letting GPT-5.2 enforce correct
   syntax at the API level.
