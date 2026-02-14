# Agent Patterns for Code Editing

Which agent design patterns apply to an editor, which are overkill, and
what the research actually says about ReAct, tool use, and test-time scaling.

Date: 2026-02-13

## 1. The Spectrum: Single-Shot to Autonomous Agent

Not every AI feature needs an agent. Anthropic's "Building Effective Agents"
guide (Dec 2024) identifies a spectrum:

```
Single LLM call → Prompt chain → Routed workflow → Agent loop
     ↑                                                    ↑
  Selection edits                                   Chat with tools
```

**Principle:** Start simple. Add agentic complexity only when simpler
solutions fall short. A selection edit ("rename this variable") is a
single LLM call. A chat conversation with file navigation is an agent loop.
The harness should support both without forcing one pattern.

Source: https://www.anthropic.com/research/building-effective-agents


## 2. Anthropic's Five Patterns

### Prompt Chaining

Decompose a task into fixed sequential steps. Each step processes the
previous step's output. Programmatic "gates" between steps verify progress.

**Ovim application:** Selection edits already use this implicitly:
gather context → construct prompt → call model → extract edit → apply.
Each stage is deterministic except the model call.

### Routing

Classify an input and direct it to a specialized handler. Simple inputs
go to cheap/fast models, complex ones to capable/expensive models.

**Ovim application:** This is exactly what `contexts` config does today.
Selection edits route to `gpt-4.1-mini`, chat routes to `gpt-5.2`. The
research validates this pattern — but suggests making it adaptive rather
than static (see §5).

### Parallelization

Run subtasks concurrently. Two forms:
- **Sectioning:** Independent subtasks in parallel
- **Voting:** Same task multiple times, pick best result

**Ovim application:** Test-time scaling research (§4) shows that parallel
sampling with Best-of-N selection is the single most effective scaling
strategy. For expensive operations (chat-driven refactors), running 2-4
parallel generations and picking the best could be worth the cost.

### Orchestrator-Workers

A central LLM dynamically breaks down a task and delegates to workers.
Subtasks aren't predetermined — the orchestrator decides what to do.

**Ovim application:** This is the pattern for tool-using chat. The chat
model decides when to read files, check diagnostics, navigate symbols.
Ovim already has this architecture via tool calls in `ai_chat_mode.rs`.

### Evaluator-Optimizer

One LLM generates; another evaluates and provides feedback. Iterative
refinement loops.

**Ovim application:** Not currently used, but could power "AI review" —
generate an edit, then have a second call evaluate whether it introduced
regressions, matches style, etc.


## 3. ReAct: What It Gets Right and Where It Falls Apart

### The Original Promise (Yao et al., 2022)

ReAct interleaves reasoning traces ("I need to find the function definition")
with actions ("search: function_name") in alternating steps. The reasoning
trace is supposed to help the model plan and recover from errors.

Source: https://arxiv.org/abs/2210.03629

### The Brittle Foundations (Verma et al., May 2024)

A systematic analysis found that ReAct's performance advantages are largely
illusory:

**What they tested:** Varied reasoning trace quality (strong guidance,
placebo "take a deep breath", reversed instructions, anonymized reasoning)
and exemplar similarity (same-task vs cross-task examples).

**Key findings:**

1. **Interleaving doesn't matter.** Consolidating all reasoning upfront
   (Chain-of-Thought) matched or outperformed interleaved ReAct. GPT-3.5
   improved from 27.6% to 46.6% with CoT vs base ReAct.

2. **Reasoning content doesn't matter.** Placebo guidance performed
   comparably to carefully crafted reasoning traces.

3. **Exemplar similarity is everything.** Performance collapsed when
   examples didn't match the query type:
   - Domain synonym substitution: 27.6% → 1.6%
   - Cross-task exemplars: near-zero (0-6.6%)
   - Single mismatched exemplar: 44.7% → 5.2%

**Implication for Ovim:** The "reasoning" in agent loops is mostly pattern
matching against examples in the prompt. What matters is that the system
prompt contains examples highly similar to the actual task. For selection
edits, this means task-specific prompts ("You are renaming a variable..."
vs "You are fixing a type error...") would outperform a generic "You are
a code editing assistant" — *if* the task type can be classified cheaply.

Source: https://arxiv.org/abs/2405.13966

### What modern agents actually do

Post-ReAct agent systems don't rely on interleaved reasoning. They use:
- Tool calls with structured outputs (not free-form "Thought: ... Action: ...")
- System prompts with diverse, task-relevant examples
- Constrained tool schemas that limit action space

Ovim's chat mode already follows this pattern with structured tool call
schemas, not ReAct-style text parsing.


## 4. Test-Time Scaling for Agents

### The Core Finding (arXiv 2506.12928, Jun 2025)

The first systematic study of test-time scaling for agents (not just
reasoning tasks). Tested on GAIA benchmark with GPT-4.1.

**Strategies compared:**

| Strategy | Score | Notes |
|----------|-------|-------|
| Baseline (single run) | 55.76% | |
| Best-of-N (BoN) | **63.03%** | +8 points, best overall |
| BoN-wise (per-step) | 58.79% | Better on hardest tasks |
| Beam Search | ~56% | Limited by verifier quality |
| DVTS (tree search) | ~56% | Same limitation |

**Key findings:**

1. **BoN is king.** Simple parallel sampling with list-wise verification
   outperforms sophisticated search strategies. Run N independent agents,
   pick the best result.

2. **Selective reflection beats continuous reflection.** Score-based
   reflection (only reflect when score < threshold) at 56.36% outperformed
   reflection-at-every-step at 55.15%. Over-reflecting degrades coherence.

3. **List-wise verification beats scoring.** Showing a verifier all
   candidates at once (list-wise: 63.03%) outperforms scoring each
   independently (scoring: 59.39%) or voting (56.8%).

4. **Multi-model mixing is powerful.** Using 4 different models for
   parallel rollouts: 74.55% Pass@4 vs 55.76% single-model baseline.

**Verification approaches:**

| Method | BoN Score |
|--------|-----------|
| Voting | 56.8% |
| Scoring (independent) | 59.39% |
| List-wise (compare all) | **63.03%** |

Source: https://arxiv.org/abs/2506.12928

### Implications for Ovim

**Selection edits:** Single-shot is fine. The task is simple enough that
parallel sampling adds cost without meaningful quality improvement (per
the inference-time scaling research showing diminishing returns on simple
tasks).

**Chat/agentic edits:** For complex operations (multi-file refactor, bug
diagnosis), consider:
- Run 2 parallel generations, pick the one that compiles/passes lint
- Use a different verification signal per context (syntax check for code
  edits, coherence check for explanations)
- If multi-model profiles are configured, mix models for diversity


## 5. Prompt Routing and Task Classification

### The Research Case (ICLR 2025)

RouteLLM (UC Berkeley/Anyscale/Canva) demonstrates that routing between
models based on task complexity achieves 85% cost reduction while
maintaining 95% of GPT-4 quality.

**Approaches:**
- **Pre-generation routing:** Classify task before calling any model
- **Cascade routing:** Start with cheap model, escalate if quality check fails

### Complexity Signals Available in Ovim

For selection edits, several signals indicate complexity:

| Signal | Simple Task | Complex Task |
|--------|------------|-------------|
| Selection size | 1-5 lines | 20+ lines |
| Diagnostic count | 0-1 | 3+ overlapping |
| Prompt length | Short instruction | Multi-sentence |
| Language complexity | Rename, format | Architecture, refactor |
| Symbol count in scope | Few | Many cross-references |

**Practical approach:** Rather than training a classifier, use heuristic
routing based on these signals. The Lua config already supports per-context
routing (`contexts.selection`, `contexts.chat`). Extend to support
complexity-based sub-routing within a context.


## 6. AutoPrompter: Inferring Missing Context (Google, 2025)

Research from "Prompting LLMs for Code Editing" found five categories of
information commonly missing from developer prompts:

1. **Specifics** — Variable names, types, API calls
2. **Operationalization plan** — How to implement (which approach)
3. **Localization/scope** — Where changes apply
4. **Codebase context** — Proprietary systems the model hasn't seen
5. **User intent** — Vague directives like "make it simpler"

**AutoPrompter** automatically infers missing information from surrounding
code context, achieving 27% improvement on previously-failing edits
(approaching the theoretical 37% ceiling).

**Ovim already does parts of this:** The context pack provides codebase
context (nearby symbols, diagnostics, surrounding code). What's missing:
- Intent classification (is this a rename? refactor? bug fix?)
- Operationalization hints (which approach for this language/framework)
- Scope clarification (apply to selection only, or propagate?)

Source: https://arxiv.org/html/2504.20196v2


## 7. Tool Design for Agents (Anthropic, 2025)

Anthropic's "Writing Tools for Agents" provides design principles that
apply directly to Ovim's tool schemas:

**Write descriptions like onboarding a new engineer.** Make implicit
context explicit. Specialized query formats, niche terminology, and
resource relationships should be spelled out.

**Target specific workflows, not generic APIs.** Instead of wrapping
all operations, design tools for high-impact patterns. `read_file` with
line ranges is better than raw file I/O.

**Return high-signal information only.** Filter, paginate, truncate.
Use response format options (concise vs detailed) so the agent controls
its own context consumption.

**Error messages must be actionable.** "No match found" is useless.
"No match found for `old_str`. Closest match is on line 42: `let x = ...`.
Did you mean this?" is recoverable.

Source: https://www.anthropic.com/engineering/writing-tools-for-agents


## 8. ROCODE: Mid-Generation Error Recovery (ICSE 2025)

Most harnesses fix errors after generation. ROCODE fixes them *during*
generation by integrating static analysis into the decoding loop:

1. **Incremental error detection:** Check syntax/types as tokens stream
2. **Backtracking:** When error detected, roll back to last good state
3. **Constraint regeneration:** Penalize tokens that caused the error

**Results (model-agnostic, tested on 9 LLMs):**
- Compilation pass rate: 99.1%
- Test pass rate: +23.8% over best baseline
- Token cost: -19.3% vs post-generation repair

**For Ovim:** This is the most promising approach for improving edit
reliability at the harness level, but requires control over the decoding
process (possible with local models, not with API providers). For API
providers, the closest analog is structured generation / constrained
decoding (see §9).

Source: https://arxiv.org/abs/2411.07112


## 9. Constrained Decoding and Structured Generation

### Grammar-Constrained Decoding (ICML 2025)

Force LLM outputs to conform to a context-free grammar (CFG). The latest
algorithm (Feb 2025) achieves 17.71× faster preprocessing while maintaining
state-of-the-art mask computation speed.

**GPT-5.2 specific:** Supports `type: "custom"` tools with CFG constraints.
Ovim could define its edit format as a grammar and let the API enforce
syntactic correctness.

**IterGen (ICLR 2025):** Forward/backward generation tied to grammar
symbols with KV-cache reuse. Enables interactive editing — generate, undo,
regenerate — efficiently.

### Practical application for Ovim

For JSON extraction: OpenAI's `response_format: {type: "json_object"}`
already enforces valid JSON. This is equivalent to a JSON CFG constraint.

For codeblock extraction: A CFG for "optional text, then ``` block, then
optional text" would prevent the model from emitting responses without a
code block. This is not yet available through standard APIs but could be
implemented with custom tool definitions on GPT-5.2.

Sources:
- https://arxiv.org/abs/2502.05111
- https://arxiv.org/abs/2305.13971
