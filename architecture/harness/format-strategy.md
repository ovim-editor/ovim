# Format Strategy Architecture

Which edit format to use for which context × provider combination, how to
fall back when extraction fails, and how to recover from errors.

## The Decision Matrix

The research is clear: no single format dominates. The optimal choice
depends on (1) the provider/model family, (2) the edit context, and
(3) the size of the edit.

### Selection Edits

For selection edits, the region is small (1-50 lines typically). Token cost
is negligible. Universal reliability matters most.

**Decision: codeblock for all providers.**

Rationale:
- Works with every model, every provider, every size
- No matching logic needed — the output IS the edit
- Diff-XYZ shows search-replace has a slight edge on accuracy, but codeblock
  has zero format compliance failures
- The token overhead on a 20-line selection is ~100 extra tokens — meaningless

### Chat-Driven File Edits (Future)

When chat applies edits to open files, the edit may span hundreds of lines
across multiple locations. Token cost matters. Matching reliability matters.

**Decision: provider-adaptive with universal fallback.**

```
                    ┌─────────────┐
                    │ Chat edit   │
                    │ requested   │
                    └──────┬──────┘
                           │
                    ┌──────▼──────┐
                    │ Provider?   │
                    └──────┬──────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
        ┌─────▼────┐ ┌────▼─────┐ ┌────▼─────┐
        │ OpenAI   │ │ Anthropic│ │ Ollama/  │
        │          │ │          │ │ Other    │
        └─────┬────┘ └────┬─────┘ └────┬─────┘
              │            │            │
        apply_patch   str_replace   codeblock
              │            │            │
              └────────┬───┘            │
                       │                │
                ┌──────▼──────┐         │
                │ Match fail? │         │
                └──────┬──────┘         │
                       │ yes            │
                ┌──────▼──────┐         │
                │ Retry with  │         │
                │ error msg   │         │
                └──────┬──────┘         │
                       │ fail           │
                ┌──────▼──────┐         │
                │ Fall back   │←────────┘
                │ to codeblock│
                └─────────────┘
```


## Format Specifications

### codeblock (current default)

**System prompt:**
```
Return ONLY the replacement code inside a single fenced code block (```).
Do not include any explanation outside the code block.
```

**Extraction:** Find first ```` ``` ```` block, strip language tag, use content.

**Failure modes:**
- Model includes explanation before/after block → handled (we take first block)
- Model uses `// ... rest of function` elision → NOT handled (see §4)
- Model returns no code block → extraction fails

### apply_patch (OpenAI models)

**System prompt:**
```
Apply the requested change using the apply_patch format.
Use *** Begin Patch / *** End Patch delimiters.
For each changed file, use *** Update File: path.
Use @@ headers to locate changes. Use - for removed lines, + for added lines.
Include 3 lines of context around each change.
```

**Extraction:**
1. Find `*** Begin Patch` ... `*** End Patch` markers
2. Parse file operations (Update File, Add File, Delete File)
3. For each hunk: match context lines to locate position
4. Apply `-` removals and `+` additions

**Matching fallback (from Codex reference implementation):**
1. Exact character match
2. Trim trailing whitespace, match
3. Trim all whitespace, match

**Failure modes:**
- Wrong context lines → match fails → retry with error feedback
- Wrong function name in `@@` header → match fails
- Non-GPT models: 46-51% failure rate (do NOT use for non-GPT)

### str_replace (Anthropic/Claude models)

**System prompt:**
```
Apply the requested change using str_replace blocks.
For each change, provide the exact text to find and the replacement text.
Use SEARCH/REPLACE delimiters.
```

**Extraction:**
1. Find `<<<<<<< SEARCH` ... `=======` ... `>>>>>>> REPLACE` blocks
2. For each block: find `old_str` in file, replace with `new_str`

**Matching fallback (inspired by Aider):**
1. Exact match
2. Whitespace-insensitive match
3. Indentation-preserving match (strip common indent, match, reindent)
4. Fuzzy match (Levenshtein distance above threshold)

**Failure modes:**
- `old_str` appears multiple times → ambiguous → fail or use context to disambiguate
- Model gets whitespace wrong → handled by fallback layer 2
- Model gets indentation wrong → handled by fallback layer 3


## Fallback Matching Architecture

For both apply_patch and str_replace, implement layered matching:

```rust
enum MatchResult {
    Exact(usize),           // matched at byte offset
    WhitespaceNormalized(usize),
    IndentNormalized(usize),
    Fuzzy(usize, f64),     // offset + similarity score
    NotFound,
}

fn find_match(haystack: &str, needle: &str) -> MatchResult {
    // Layer 1: Exact
    if let Some(offset) = haystack.find(needle) {
        return MatchResult::Exact(offset);
    }

    // Layer 2: Whitespace-normalized
    let normalized_needle = normalize_whitespace(needle);
    for (offset, line_range) in line_ranges(haystack) {
        let normalized_line = normalize_whitespace(&haystack[line_range]);
        if normalized_line.contains(&normalized_needle) {
            return MatchResult::WhitespaceNormalized(offset);
        }
    }

    // Layer 3: Indentation-normalized
    let stripped_needle = strip_common_indent(needle);
    // ... similar search with indent normalization

    // Layer 4: Fuzzy (Levenshtein)
    let best = find_best_levenshtein_match(haystack, needle, threshold: 0.8);
    if let Some((offset, score)) = best {
        return MatchResult::Fuzzy(offset, score);
    }

    MatchResult::NotFound
}
```


## Error Recovery Architecture

When extraction or matching fails, the harness should retry before giving up.

### Retry Protocol

```
1. First attempt with primary format
   ├── Success → apply edit
   └── Failure → construct error feedback

2. Retry with error context appended to conversation
   ├── Success → apply edit
   └── Failure → fall back to codeblock format

3. Codeblock fallback (for apply_patch/str_replace failures)
   ├── Success → apply edit (with "fallback used" warning)
   └── Failure → report to user
```

### Error Feedback Message Template

```
Your previous response could not be applied.
Error: {specific_error}

{if match_failure}
The text you specified to replace was not found in the file.
The closest match is:
```
{closest_match_with_line_numbers}
```
{end if}

{if parse_failure}
Your response did not contain a valid {format_name} block.
Please respond with {format_instructions}.
{end if}

Please try again.
```


## Elision Detection

Models sometimes produce placeholder comments instead of real code:

```rust
fn complex_function() {
    // ... existing setup code ...
    let new_line = do_something();
    // ... rest of function ...
}
```

### Detection Patterns

```
/^\s*\/\/\s*\.\.\./       // ...
/^\s*\/\*\s*\.\.\./       /* ...
/^\s*#\s*\.\.\./          # ...
/^\s*\/\/\s*rest of/i     // rest of function
/^\s*\/\/\s*remaining/i   // remaining code
/^\s*\/\/\s*unchanged/i   // unchanged
/^\s*\/\/\s*same as/i     // same as before
```

### Response to Elision

1. If detected in selection edit: re-prompt with
   "Do not use placeholder comments. Include all code."
2. If detected in chat edit: warn user in hover panel
3. Track elision rate per model profile for diagnostics


## Configuration Schema

### Current

```lua
profiles = {
    gpt_4_1_mini = {
        edit_format = "codeblock",  -- "codeblock" | "json" | "raw"
    },
}
```

### Proposed Extension

```lua
profiles = {
    gpt_5_2 = {
        -- Selection edit format (used for visual selection edits)
        edit_format = "codeblock",

        -- Chat edit format (used when chat applies file edits)
        -- If omitted, uses provider default
        chat_edit_format = "apply_patch",

        -- Fallback format when primary fails
        fallback_format = "codeblock",

        -- Maximum retry count before fallback
        max_retries = 1,
    },

    claude_sonnet = {
        edit_format = "codeblock",
        chat_edit_format = "str_replace",
        fallback_format = "codeblock",
    },
}
```

### Provider Defaults

When `chat_edit_format` is not set, infer from provider:

```rust
fn default_chat_edit_format(provider: AiProviderKind) -> &'static str {
    match provider {
        AiProviderKind::OpenAi => "apply_patch",
        AiProviderKind::Anthropic => "str_replace",
        AiProviderKind::Ollama => "codeblock",
    }
}
```


## Implementation Phases

### Phase 1: Foundation (current work)

- Keep codeblock for all selection edits
- Add elision detection (warn, don't retry yet)
- Add retry-on-extraction-failure for codeblock format

### Phase 2: Provider-Adaptive Chat Edits

- Implement apply_patch parser + matching (with fallback layers)
- Implement str_replace parser + matching (with fallback layers)
- Add error feedback retry protocol
- Add `chat_edit_format` config field

### Phase 3: Optimization

- Track success/failure rates per model × format combination
- Auto-select format based on historical performance
- Add Levenshtein fuzzy matching as final fallback layer
- Consider hashline format for models that struggle with both
  apply_patch and str_replace
