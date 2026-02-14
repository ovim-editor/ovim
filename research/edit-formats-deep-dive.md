# Edit Formats: A Deep Dive

How LLMs apply code edits, which formats work best, and why the harness
matters as much as the model.

Date: 2026-02-13

## 1. The Landscape

Every AI coding tool must solve the same problem: the model produces text,
but the editor needs a structured edit. The format that bridges the two is
the single biggest lever on edit success rate — bigger than model selection,
bigger than prompt engineering.

Six major approaches exist in the wild:

| Format | Used By | Mechanism | Token Cost |
|--------|---------|-----------|------------|
| apply_patch | OpenAI Codex | Structured diff with context lines | Low |
| str_replace | Claude Code, Gemini CLI | Exact old→new text match | Medium |
| codeblock / whole | Aider, Ovim | Full replacement in fenced block | High |
| hashline | blog.can.ac (novel) | Reference lines by content hash | Low |
| search/replace blocks | Aider, RooCode | Delimited SEARCH/REPLACE sections | Medium |
| neural merge | Cursor | Separate "sketch" + "apply" models | Variable |


## 2. Format Details

### apply_patch (OpenAI)

The format GPT models are trained on. Structure:

```
*** Begin Patch
*** Update File: src/main.rs
@@ fn handle_request()
-    let response = process(input);
+    let response = process(input)?;
*** End Patch
```

**Rules:**
- Three operations: Add File, Update File, Delete File
- Hunks introduced by `@@` with optional class/function context
- Context lines (space prefix), deletions (`-`), additions (`+`)
- 3 lines of context by default, more when needed for uniqueness
- Progressive fallback: exact → trimmed whitespace → fully trimmed

**Strengths:**
- Token-efficient (only changed lines + small context)
- Multi-file edits in a single response
- GPT models post-trained on this format → high compliance

**Failure modes:**
- Catastrophic on non-GPT models: 50.7% failure on Grok 4, 46.2% on GLM-4.7
- Context lines must match exactly — models hallucinate surrounding code
- Hunk headers are fragile — wrong function names break matching

**Quantitative data (Diff-XYZ benchmark, Dec 2025):**
- Diff generation: GPT-4.1 scores 0.95 EM vs 0.81 for udiff
- Apply task: GPT-4.1 scores 0.90 for udiff vs 0.96 for search-replace
- The unified diff format is harder for models than search-replace

Source: https://arxiv.org/html/2510.12487v2

### str_replace (Claude Code / Gemini CLI)

```json
{
  "tool": "str_replace_editor",
  "old_str": "let response = process(input);",
  "new_str": "let response = process(input)?;"
}
```

**Strengths:**
- Intuitive — models understand find-and-replace
- Works across model families (not provider-specific training)
- Aider's implementation adds layered fallback: exact → whitespace-insensitive
  → indentation-preserving → fuzzy

**Failure modes:**
- Model must reproduce every character including whitespace
- Breaks when target string appears multiple times in file
- Long `old_str` blocks are error-prone (the model is reciting from memory)

**Key insight from Code Surgery (Hertwig 2025):** RooCode extends this with
"middle-out fuzzy matching" — starts searching near the expected location,
expands outward, uses Levenshtein distance scoring. This handles minor
inaccuracies without requiring exact recall.

Source: https://fabianhertwig.com/blog/coding-assistants-file-edits/

### codeblock / whole (Aider, Ovim)

The model returns the complete replacement inside a fenced code block:

````
Here's the fixed code:

```rust
let response = process(input)?;
```
````

**Strengths:**
- Most universal and reliable format
- Simple extraction (find first ``` block)
- No matching required — the entire output IS the edit
- Works with every model family, every size

**Failure modes:**
- Token-expensive for large regions (entire replacement must be emitted)
- Models may elide code with `// ... rest of function` placeholder comments
- No multi-file support in a single response

**Why Ovim uses this for selection edits:** The selected region is small
(typically 1-50 lines). Token cost is negligible. The format is universal.
There's no matching to go wrong. This is the right default.

### hashline (blog.can.ac, Feb 2026)

A novel format that tags each line with a short content hash:

```
11:a3|function hello() {
22:f1|  return "world";
33:0e|}
```

Models reference the hash to anchor edits instead of reproducing content.

**Results (16 models, 180 tasks, 3 runs each):**
- Grok Code Fast: 6.7% → 68.3% success rate (10× improvement vs apply_patch)
- MiniMax: more than doubled
- Grok 4 Fast: 61% reduction in output tokens
- Matched or beat str_replace for most models

**Key insight:** The format separates "where to edit" from "what to write."
Models are good at the second part but bad at the first (line numbers,
exact text reproduction). Hashline solves addressing without requiring
recall.

Source: https://blog.can.ac/2026/02/12/the-harness-problem/

### search/replace blocks (Aider)

```
<<<<<<< SEARCH
let response = process(input);
=======
let response = process(input)?;
>>>>>>> REPLACE
```

Aider's layered matching strategy:
1. Exact match
2. Whitespace-insensitive
3. Indentation-preserving (strip common indent, match, reindent)
4. Fuzzy (similarity threshold)

Each layer is tried in order. Detailed error messages guide the model on
retry: "No exact match found. Did you mean line 42: `let response = ...`?"

### neural merge (Cursor)

Two-stage approach:
1. **Sketch phase**: Primary LLM generates intended changes focusing on logic
2. **Apply phase**: Separate model trained specifically for code integration

The apply model handles context, indentation, and structural nuances that
the primary model's sketch might gloss over.

**Trade-off:** Requires training a separate model. Adds latency from two
inference calls. But handles complex files better than any single-format
approach.


## 3. Cross-Format Analysis

### What works universally

From research across all formats, two principles hold:

1. **Avoid line numbers.** Models get them wrong. Every successful format uses
   content-based addressing: context lines (apply_patch), exact text match
   (str_replace), content hashes (hashline), or full replacement (codeblock).

2. **Clearly delimit original vs replacement.** Whether it's `+`/`-` prefixes,
   SEARCH/REPLACE markers, or a fenced code block — the boundary between old
   and new must be unambiguous.

### What the Diff-XYZ benchmark tells us (Dec 2025)

Tested unified diff, relaxed diff (no line numbers), verbose markers, and
search-replace across GPT-4o, GPT-4.1, Claude Sonnet, and Qwen models:

**Apply task (old code + diff → new code):**
| Model | search-replace | udiff |
|-------|---------------|-------|
| GPT-4o | 0.94 EM | 0.86 |
| GPT-4.1 | 0.96 | 0.90 |
| Claude Sonnet | 0.97 | 0.95 |
| Qwen 32B | 0.92 | 0.84 |

**Diff generation (produce the diff):**
| Model | search-replace | udiff |
|-------|---------------|-------|
| GPT-4o | 0.73 EM | 0.41 |
| GPT-4.1 | 0.95 | 0.81 |
| Claude Sonnet | 0.94 | 0.82 |

**Key findings:**
- search-replace is the best format overall for large models (7B+)
- Verbose markers (ADD/DEL/CON) *hurt* performance vs simpler formats
- Below 7B parameters, no format works reliably
- Diff generation is harder than diff application across all formats
- No open-source model matches proprietary models on diff generation

Source: https://arxiv.org/html/2510.12487v2

### The Harness Problem (Feb 2026)

The strongest finding: **switching edit format alone can swing success rate
from 6.7% to 68.3% on the same model.** That's a 10× improvement from
changing zero model weights.

Implications:
- Benchmarks that use a fixed format are measuring format fit, not model quality
- "Model X is bad at coding" often means "Model X is bad at apply_patch"
- The harness is a first-class engineering concern, not plumbing

Source: https://blog.can.ac/2026/02/12/the-harness-problem/


## 4. Error Recovery and Fallback

### Layered matching (industry standard)

Every production harness implements some form of progressive fallback:

**Codex (apply_patch):** exact → trimmed line endings → all whitespace trimmed

**Aider (search/replace):** exact → whitespace-insensitive → indent-preserving → fuzzy

**RooCode:** Middle-out fuzzy matching with Levenshtein distance scoring.
Starts near expected location, expands outward, picks highest-scoring match
above threshold.

### Error messages matter

Aider's approach: when matching fails, provide specific diagnostic feedback
to the model for retry. "No match found for your SEARCH block. The closest
match is on line 42..." This turns a hard failure into a recoverable retry.

### ROCODE: Backtracking during generation (ICSE 2025)

Rather than fixing errors after generation, ROCODE detects them *during*:
- Incremental syntax checking as tokens are generated
- When error detected: backtrack, apply penalty to error-causing tokens
- Result: 99.1% compilation pass rate, 23.8% higher test pass rate
- 19.3% token reduction vs post-generation repair

This is the most promising approach for harness-level error handling: don't
let the model finish a bad edit, catch it mid-stream.

Source: https://arxiv.org/abs/2411.07112


## 5. Implications for Ovim

### Selection edits (current)

**Codeblock is correct.** It's universal, reliable, simple to extract, and
the selected region is small. No change needed.

### Chat-driven file edits (future)

This is where format strategy becomes critical. Recommendations:

1. **Provider-adaptive format selection:**
   - OpenAI models: apply_patch (post-trained on it)
   - Anthropic models: str_replace (Claude Code's native format)
   - Ollama/local: codeblock (safest for smaller models)

2. **Layered fallback matching:**
   Implement Aider-style progressive matching for str_replace:
   exact → whitespace-insensitive → fuzzy (Levenshtein)

3. **Error feedback for retry:**
   When matching fails, construct a diagnostic message and re-prompt
   the model with the failure details.

4. **Consider hashline for multi-model support:**
   If Ovim needs a single format that works across all providers,
   hashline is the most promising candidate from recent research.

Sources:
- https://blog.can.ac/2026/02/12/the-harness-problem/
- https://fabianhertwig.com/blog/coding-assistants-file-edits/
- https://arxiv.org/html/2510.12487v2
- https://aider.chat/docs/more/edit-formats.html
