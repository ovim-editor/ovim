-- ================================================================
-- builtin.lua — ovim's full harness policy
-- ================================================================
--
-- This file ships inside the binary via include_str!(). It runs
-- before the user's init.lua. Every line here can be overridden.
--
-- Design principle: Lua is the policy, Rust is the engine.
-- This file says WHAT to do. Rust code does the HOW.
--
-- The five building blocks:
--   1. API Keys          — where secrets live
--   2. Prompts           — what we tell the model
--   3. Formats           — how models express edits (extensible)
--   4. Context Policies  — how much context to gather
--   5. Profiles          — model + params + format + context


-- ================================================================
-- 1. API KEYS
-- ================================================================
--
-- Keys are registered by name. Profiles reference the name, never
-- the raw secret. Resolution at request time: env_var → file.
--
-- Users override in init.lua to change the source:
--   vim.api_keys.register("openai", { env_var = "OPENAI_API_KEY" })
--   vim.api_keys.register("openai", { file = "~/.secrets/openai.key" })
--
-- Programmatic access for plugins:
--   vim.api_keys.get("openai")                  → opaque key id
--   vim.api_keys.dangerously_get_raw("openai")  → actual secret string

vim.api_keys.register("openai", {
    env_var = "OVIM_OPENAI_API_KEY",
})

vim.api_keys.register("anthropic", {
    env_var = "OVIM_ANTHROPIC_API_KEY",
})


-- ================================================================
-- 2. PROMPTS
-- ================================================================
--
-- Named system prompt templates. Global defaults.
--
-- Resolution order for selection edits:
--   1. profile.edit_prompt          (per-profile override)
--   2. vim.ai.prompts["selection_{edit_format}"]  (this table)
--   3. format.prompt                (from vim.ai.formats.register)
--   4. Rust hardcoded fallback      (safety net)
--
-- Resolution order for chat:
--   1. profile.chat_prompt          (per-profile override)
--   2. vim.ai.prompts["chat"]       (this table)
--   3. Rust hardcoded fallback
--
-- Resolution order for chat edits:
--   1. profile.chat_edit_prompt     (per-profile override)
--   2. vim.ai.prompts["chat_edit_{chat_edit_format}"]
--   3. format.prompt                (from vim.ai.formats.register)
--   4. Rust hardcoded fallback
--
-- Users override in init.lua:
--   vim.ai.prompts.selection_codeblock = "Your custom prompt"

vim.ai.prompts = {

    -- ── Selection edit prompts ──────────────────────────────
    -- Used when the user selects code and asks for an edit.
    -- The model's response replaces the selection.

    selection_codeblock = [[
You are a code editing assistant.
Return ONLY the replacement code inside a single fenced code block.
Do not include any explanation outside the code block.
Do not use placeholder comments like "// rest of function" — include ALL code.]],

    selection_json = [[
You are a code editing assistant.
Return a JSON object with this schema:
{
  "replacement": "the code that replaces the selection",
  "new_import_statements": ["import lines to add at the top of the file"],
  "log": ["short descriptions of what you changed"]
}
Only output valid JSON, no explanation.]],

    selection_raw = [[
You are a code editing assistant.
Return ONLY the replacement code.
No markdown, no code fences, no explanation.
Do not use placeholder comments — include ALL code.]],

    -- ── Chat prompts ────────────────────────────────────────
    -- Used for the AI chat panel.

    chat = [[
You are an AI coding assistant integrated into ovim, a code editor.
Help the user with their code. Be concise and precise.
When showing code, use fenced code blocks with the language tag.
When suggesting edits, show only the changed portions with enough context to locate them.]],

    -- ── Chat edit prompts ───────────────────────────────────
    -- Used when chat applies edits to open files.
    -- Provider-adaptive: each model family gets the format it was
    -- trained on. Codeblock is the universal fallback.

    chat_edit_apply_patch = [[
Apply the requested change using the apply_patch format.

Format rules:
- Wrap the entire patch in *** Begin Patch / *** End Patch
- For each file: *** Update File: path
- Use @@ with optional function/class context to locate changes
- Prefix removed lines with -, added lines with +, context lines with space
- Include 3 lines of context around each change for unique matching

Example:
*** Begin Patch
*** Update File: src/main.rs
@@ fn handle_request()
 fn handle_request(input: &str) -> Result<Response> {
-    let response = process(input);
+    let response = process(input)?;
     Ok(response)
 }
*** End Patch]],

    chat_edit_str_replace = [[
Apply the requested change using SEARCH/REPLACE blocks.

For each change, provide the exact text to find and its replacement:

<<<<<<< SEARCH
exact text to find (include enough context for unique match)
=======
replacement text
>>>>>>> REPLACE

Rules:
- The SEARCH text must match the file exactly (including whitespace)
- Include enough surrounding lines to uniquely identify the location
- Use multiple SEARCH/REPLACE blocks for multiple changes
- Order blocks by their position in the file (top to bottom)]],

    chat_edit_codeblock = [[
Apply the requested change by returning the complete updated file
inside a single fenced code block. Include ALL code — do not elide
any sections with placeholder comments.]],
}


-- ================================================================
-- 3. FORMATS
-- ================================================================
--
-- Edit formats define how models express edits and how ovim
-- parses them. Built-in formats are implemented in Rust for
-- performance. Custom formats can be registered in Lua.
--
-- Built-in formats (Rust extraction engines):
--   "codeblock"     → extract first ``` block
--   "json"          → parse JSON {replacement, new_import_statements, log}
--   "raw"           → use entire response verbatim
--   "apply_patch"   → parse *** Begin/End Patch envelope
--   "str_replace"   → parse <<<<<<< SEARCH / >>>>>>> REPLACE blocks
--
-- Lua-registered formats get the same status as builtins.
-- A format is: tag (optional) + extract + prompt.
--   tag:     transforms context lines before they enter the prompt
--   extract: parses model output into structured edits
--   prompt:  system prompt fragment (fallback if no prompt in
--            vim.ai.prompts or profile override)
--
-- Users can register custom formats in init.lua for research
-- or specialized workflows.

vim.ai.formats.register("hashline", {
    -- Tag context lines with content hashes before sending to model.
    -- The hash provides content-based addressing — models reference
    -- lines by hash instead of reproducing exact text.
    --
    -- Research basis: "The Harness Problem" (Feb 2026) showed this
    -- format improved Grok Code Fast from 6.7% to 68.3% success rate.
    -- It works across model families because it separates "where to
    -- edit" from "what to write."
    tag = function(lines)
        local out = {}
        for _, l in ipairs(lines) do
            local h = vim.fn.sha256(l.content):sub(1, 4)
            out[#out + 1] = ("%d:%s|%s"):format(l.line, h, l.content)
        end
        return table.concat(out, "\n")
    end,

    -- Parse the model's response into edit operations.
    -- Returns a list of edits, or nil + error string.
    --
    -- For selection edits, return:
    --   { replacement = "..." }
    --
    -- For chat/multi-file edits, return:
    --   { { file = "...", old = "...", new = "..." }, ... }
    extract = function(response)
        local edits = {}
        -- Parse @@ near HASH blocks
        for block in response:gmatch("@@.-\n(.-)\n@@") do
            -- Parse +/- prefixed lines within each block
            -- ... (implementation)
        end
        if #edits == 0 then
            return nil, "no hashline edit blocks found"
        end
        return edits
    end,

    -- Default prompt for hashline format.
    prompt = [[
Each line of code is tagged as "LINE:HASH|content" where HASH is a
4-character content hash. To make edits, reference lines by their hash.

Format your edits as:
@@ near HASH
 N:HASH|unchanged context line
-N:HASH|line to remove
+new line to add
 N:HASH|unchanged context line

Rules:
- Use 2-3 context lines (by hash) around each change for unique matching
- Prefix removed lines with -, added lines with +, context lines with space
- New lines (+ prefix) do not have a hash — they are new content]],
})


-- ================================================================
-- 4. CONTEXT POLICIES
-- ================================================================
--
-- Pre-defined tables that control how much context the harness
-- gathers before constructing the prompt. Profiles reference
-- these directly or extend them with vim.tbl_extend().
--
-- These are plain Lua tables — no registry, no indirection.
-- Lua variables ARE the reuse mechanism:
--
--   local my_ctx = vim.tbl_extend("force",
--       vim.ai.context_policies.hybrid,
--       { budget = 16000, symbols = 20 })
--
-- Fields:
--   surrounding_lines  — ±N lines around the selection (default: 6)
--   symbols            — number of LSP symbols to include (default: 12)
--   diagnostics        — which diagnostics to include:
--                         "overlapping" = only those touching the selection
--                         "file" = all diagnostics in the current file
--   related_slices     — follow symbol references one hop (default: true)
--   budget             — hard token ceiling for the context pack

vim.ai.context_policies = {

    -- Minimal: just the selection + immediate surroundings.
    -- Good for: renames, simple fixes, small local models.
    fast = {
        surrounding_lines = 6,
        symbols = 0,
        diagnostics = "overlapping",
        related_slices = false,
        budget = 2500,
    },

    -- Balanced: selection + LSP signal.
    -- Good for: refactors, fixes that need type info, cloud models.
    hybrid = {
        surrounding_lines = 12,
        symbols = 12,
        diagnostics = "file",
        related_slices = true,
        budget = 8000,
    },

    -- Maximum: everything we can reasonably gather.
    -- Good for: complex multi-concern edits, frontier models.
    full = {
        surrounding_lines = 30,
        symbols = 24,
        diagnostics = "file",
        related_slices = true,
        budget = 24000,
    },
}


-- ================================================================
-- 5. PROFILES & CONTEXTS
-- ================================================================
--
-- A profile is: where to send the request + how to handle the response.
--
-- Fields:
--   provider          "openai" | "anthropic" | "ollama"
--   model             string (model name)
--   api_key           string? (name from vim.api_keys, not the secret)
--   base_url          string? (override provider endpoint)
--
--   temperature       number? (0.0-2.0)
--   max_tokens        number? (max response tokens)
--   reasoning_effort  string? ("none"|"low"|"medium"|"high" — OpenAI only)
--   verbosity         string? ("low"|"medium"|"high" — OpenAI 5.2+)
--
--   edit_format       string | function
--                     String: name of a built-in or registered format
--                       "codeblock" → extract first ``` block
--                       "json"      → parse JSON {replacement, ...}
--                       "raw"       → use entire response verbatim
--                       "hashline"  → Lua-registered format (above)
--                     Function: inline Lua extractor
--                       function(response) → code, error
--
--   chat_edit_format  string? (format for chat-driven file edits)
--                     If omitted, uses provider-adaptive default:
--                       OpenAI    → "apply_patch"
--                       Anthropic → "str_replace"
--                       Ollama    → "codeblock"
--
--   edit_prompt       string? (override selection edit system prompt)
--   chat_prompt       string? (override chat system prompt)
--   chat_edit_prompt  string? (override chat edit system prompt)
--
--   context           table? (inline context policy, or reference a
--                     builtin: vim.ai.context_policies.fast)
--                     If omitted, defaults to context_policies.hybrid.
--
--   syntax_check      bool? (run tree-sitter parse after applying edit;
--                     warn in hover panel if new syntax errors introduced.
--                     default: true for cloud providers, false for local)
--
--   retry             table? (what to do on extraction failure)
--                       max      — number of retries (default: 1)
--                       fallback — format to try after retries exhausted

vim.ai.setup({
    default_profile = "local",

    contexts = {
        selection = "local",
        chat = "local",
        query = "local",
    },

    profiles = {

        -- ── Local inference ─────────────────────────────────
        -- Works without API keys. Good default for first-run.
        ["local"] = {
            provider = "ollama",
            model = "qwen2.5-coder:7b",
            temperature = 0.2,
            max_tokens = 2048,
            edit_format = "codeblock",
            context = vim.ai.context_policies.fast,
            syntax_check = false,
            retry = { max = 1 },
            -- Terse prompt for small models — they follow short
            -- instructions better than verbose ones.
            edit_prompt = [[
Return ONLY the replacement code in a ```code block.
No explanation. No placeholders. Complete code only.]],
        },

        -- ── OpenAI ──────────────────────────────────────────

        openai_fast = {
            provider = "openai",
            model = "gpt-4.1-mini",
            api_key = "openai",
            temperature = 0.2,
            max_tokens = 2048,
            edit_format = "codeblock",
            chat_edit_format = "apply_patch",
            context = vim.ai.context_policies.fast,
            syntax_check = true,
            retry = { max = 1 },
        },

        openai = {
            provider = "openai",
            model = "gpt-4.1",
            api_key = "openai",
            temperature = 0.2,
            max_tokens = 4096,
            edit_format = "codeblock",
            chat_edit_format = "apply_patch",
            context = vim.ai.context_policies.hybrid,
            syntax_check = true,
            retry = { max = 1 },
        },

        openai_frontier = {
            provider = "openai",
            model = "gpt-5.2",
            api_key = "openai",
            temperature = 0.2,
            max_tokens = 4096,
            edit_format = "codeblock",
            chat_edit_format = "apply_patch",
            context = vim.ai.context_policies.hybrid,
            syntax_check = true,
            retry = { max = 1 },
            reasoning_effort = "none",
            verbosity = "low",
        },

        -- ── Anthropic ───────────────────────────────────────

        anthropic = {
            provider = "anthropic",
            model = "claude-sonnet-4-5-20250929",
            api_key = "anthropic",
            max_tokens = 4096,
            edit_format = "codeblock",
            chat_edit_format = "str_replace",
            context = vim.ai.context_policies.hybrid,
            syntax_check = true,
            retry = { max = 1 },
        },

        anthropic_frontier = {
            provider = "anthropic",
            model = "claude-opus-4-6",
            api_key = "anthropic",
            max_tokens = 4096,
            edit_format = "codeblock",
            chat_edit_format = "str_replace",
            context = vim.ai.context_policies.hybrid,
            syntax_check = true,
            retry = { max = 1 },
        },
    },
})


-- ================================================================
-- 6. CHAT CONTEXT MANAGEMENT
-- ================================================================
--
-- Controls how the chat conversation is serialized for the API.
-- Based on JetBrains research (Dec 2025): observation masking is
-- strictly better than LLM summarization for coding agents — 52%
-- cheaper, 2.6% better solve rates.
--
-- The full conversation is always stored in memory for display.
-- Only the serialized-for-API version gets masked. The user always
-- sees everything in the chat panel.

vim.ai.chat = {
    -- Full content for the N most recent turns.
    -- Older turns: tool results replaced with mask_template.
    observation_window = 10,

    -- What the model sees in place of old tool output.
    mask_template = "[output from turn {turn} — re-read if needed]",

    -- When total context exceeds this, drop oldest masked turns.
    max_context_tokens = 100000,
}


-- ================================================================
-- 7. AGENT LOOP
-- ================================================================
--
-- The agent loop is for chat/tool-using mode only. Selection edits
-- are single-shot and never enter the agent loop.
--
-- max_tool_calls is a safety rail, not a tuning knob. Set it high
-- enough that it rarely fires. The ideal is: the agent runs until
-- done, bounded by cost, not by arbitrary iteration counts.

vim.ai.agent = {
    max_tool_calls = 50,

    -- Future: cost-based limits (requires token counting)
    -- max_cost_per_request = 0.50,   -- USD
    -- max_cost_per_session = 5.00,   -- USD
}


-- ================================================================
-- 8. PROJECT CONTEXT FILES
-- ================================================================
--
-- Project-level instructions for AI agents. These files tell the
-- model about your codebase's conventions, architecture, constraints,
-- and preferences — things it can't infer from the code alone.
--
-- Research basis: AutoPrompter (Google, 2025) found that 27% of
-- failed edits succeed when the prompt is augmented with missing
-- codebase context. Project context files address this directly.
--
-- Supported files (checked in order, all that exist are loaded):
--   .ovim.md       — ovim-specific, takes priority
--   AGENTS.md      — generic agent instructions (provider-agnostic)
--   CLAUDE.md      — Claude Code's convention (widely adopted)
--
-- Content is injected into the system prompt for both selection
-- edits and chat, between the format instructions and the user prompt.
--
-- Hierarchical: ovim walks up from the current file's directory to
-- the repo root, loading context files at each level. Deeper files
-- take priority (more specific context wins).
--
-- Budget-aware: project context competes for the same attention
-- budget as code context. For selection edits with tight budgets
-- (2,500 tokens), a 3,000-token project file is counterproductive.
-- The budget cap prevents this. For chat with large windows, the
-- full file is included.

vim.ai.project_context = {
    -- Files to look for, in priority order.
    -- All matching files are loaded and concatenated.
    files = { ".ovim.md", "AGENTS.md", "CLAUDE.md" },

    -- Walk up directories from current file to repo root.
    hierarchical = true,

    -- Maximum tokens to include from project context.
    -- Applied per-request. When the context file exceeds this,
    -- it's truncated with a note: "[truncated — X tokens over budget]".
    budget = 2000,

    -- Set to false to disable project context loading entirely.
    -- enabled = false,
}


-- ================================================================
-- WHAT INIT.LUA LOOKS LIKE
-- ================================================================
-- (This section is commentary, not executed code.)
--
-- ── Minimal (3 lines): ──────────────────────────────────────
--
--   vim.ai.contexts.selection = "openai_fast"
--   vim.ai.contexts.chat = "openai_frontier"
--   vim.ai.contexts.query = "openai"
--
-- ── Custom API key source: ──────────────────────────────────
--
--   vim.api_keys.register("openai", {
--       file = "~/.secrets/openai.key",
--   })
--
-- ── Custom context policy: ──────────────────────────────────
--
--   local rust_deep = vim.tbl_extend("force",
--       vim.ai.context_policies.hybrid,
--       { budget = 16000, symbols = 20, surrounding_lines = 20 })
--
--   vim.ai.profiles.register("claude_rust", {
--       provider = "anthropic",
--       model = "claude-sonnet-4-5-20250929",
--       api_key = "anthropic",
--       max_tokens = 4096,
--       edit_format = "codeblock",
--       chat_edit_format = "str_replace",
--       context = rust_deep,
--       syntax_check = true,
--       retry = { max = 1, fallback = "codeblock" },
--       -- Per-profile prompt for Rust work:
--       edit_prompt = [[
-- You are editing Rust code. Return ONLY the replacement inside a
-- ```rust block. Preserve all lifetime annotations and trait bounds.
-- Never use placeholder comments. Include ALL code.]],
--   })
--
--   vim.ai.contexts.selection = "openai_fast"
--   vim.ai.contexts.chat = "claude_rust"
--
-- ── Custom prompt (global): ─────────────────────────────────
--
--   vim.ai.prompts.selection_codeblock = [[
--   You are editing code. Return ONLY the replacement
--   inside a ```code block. Preserve all lifetimes.]]
--
-- ── Custom extraction function: ─────────────────────────────
--
--   local function extract_rust(response)
--       local code = response:match("```rust\n(.-)\n```")
--       if code then return code end
--       code = response:match("```%w*\n(.-)\n```")
--       if code then return code end
--       return nil, "no code block found"
--   end
--
--   vim.ai.profiles.register("rust_specialist", {
--       provider = "openai",
--       model = "gpt-4.1",
--       api_key = "openai",
--       max_tokens = 4096,
--       edit_format = extract_rust,   -- function, not string
--       edit_prompt = "Return code in a ```rust block.",
--   })
--
-- ── Custom format (research): ───────────────────────────────
--
--   vim.ai.formats.register("my_format", {
--       tag = function(lines) ... end,      -- optional
--       extract = function(response) ... end,
--       prompt = "Instructions for the model...",
--   })
--
--   vim.ai.profiles.register("experimental", {
--       provider = "openai",
--       model = "gpt-4.1",
--       chat_edit_format = "my_format",
--   })
--
-- ── Chat tuning: ────────────────────────────────────────────
--
--   vim.ai.chat.observation_window = 20   -- larger for 200k models
--   vim.ai.chat.max_context_tokens = 150000
--
-- ── Self-hosted model: ──────────────────────────────────────
--
--   vim.ai.profiles.register("deepseek", {
--       provider = "openai",
--       model = "deepseek-coder-v3",
--       base_url = "http://10.0.0.5:8080/v1",
--       temperature = 0.1,
--       max_tokens = 4096,
--       edit_format = "codeblock",
--       chat_edit_format = "hashline",
--       context = vim.ai.context_policies.hybrid,
--   })
--   vim.ai.contexts.selection = "deepseek"


-- ================================================================
-- APPENDIX: DERIVED RUST TYPES
-- ================================================================
--
-- From the above, these are the Rust primitives we need:
--
-- ── Lua API surface ─────────────────────────────────────────
--
--   vim.api_keys.register(name, { env_var?, file? })
--   vim.api_keys.get(name) → string (opaque)
--   vim.api_keys.dangerously_get_raw(name) → string (secret)
--
--   vim.ai.prompts                   (plain table, read during sync)
--   vim.ai.context_policies          (plain table of policy tables)
--   vim.ai.chat                      (plain table, read during sync)
--   vim.ai.agent                     (plain table, read during sync)
--   vim.ai.project_context           (plain table, read during sync)
--   vim.ai.formats.register(n, t)    (register a Lua-implemented format)
--   vim.ai.setup(opts)               (sugar: sets profiles + contexts)
--   vim.ai.profiles.register(n, t)   (register one profile)
--   vim.ai.contexts.{name} = "..."   (set via metatable)
--   vim.ai.default_profile = "..."   (set via metatable)
--
-- ── Rust types ──────────────────────────────────────────────
--
--   struct ApiKeyConfig {
--       env_var: Option<String>,
--       file: Option<PathBuf>,
--   }
--
--   struct ContextGatheringPolicy {
--       surrounding_lines: u16,         // default: 6
--       symbols: u16,                   // default: 12
--       diagnostics: DiagnosticScope,   // Overlapping | File
--       related_slices: bool,           // default: true
--       budget: usize,                  // token ceiling
--   }
--
--   struct RetryPolicy {
--       max: u8,                        // default: 1
--       fallback: Option<String>,       // format name after exhaustion
--   }
--
--   struct AiProfileConfig {
--       name: String,
--       provider: AiProviderKind,
--       model: String,
--       base_url: Option<String>,
--       api_key: Option<String>,            // named key reference
--       temperature: Option<f32>,
--       max_tokens: Option<u32>,
--       reasoning_effort: Option<String>,   // OpenAI only
--       verbosity: Option<String>,          // OpenAI 5.2+
--       edit_format: EditFormat,            // for selection edits
--       chat_edit_format: Option<EditFormat>,// for chat edits (None = provider default)
--       edit_prompt: Option<String>,        // per-profile override
--       chat_prompt: Option<String>,        // per-profile override
--       chat_edit_prompt: Option<String>,   // per-profile override
--       context: ContextGatheringPolicy,    // inline, always resolved
--       syntax_check: Option<bool>,         // None = provider-adaptive
--       retry: RetryPolicy,
--       tools: Vec<String>,
--       scope: ProfileScope,
--   }
--
--   enum EditFormat {
--       Codeblock,
--       Json,
--       Raw,
--       ApplyPatch,
--       StrReplace,
--       Lua(String),       // name in vim.ai.formats registry
--   }
--
--   // When a profile's edit_format is a Lua function (not a name),
--   // the Lua-side vim.ai.profiles.register() auto-registers it
--   // under a generated name and stores that name as Lua("__anon_N").
--   // One code path on the Rust side.
--
--   struct ChatContextConfig {
--       observation_window: usize,      // default: 10
--       mask_template: String,
--       max_context_tokens: usize,      // default: 100_000
--   }
--
--   struct AgentLoopConfig {
--       max_tool_calls: u16,            // default: 50
--   }
--
--   struct ProjectContextConfig {
--       files: Vec<String>,             // default: [".ovim.md", "AGENTS.md", "CLAUDE.md"]
--       hierarchical: bool,             // default: true
--       budget: usize,                  // default: 2000
--       enabled: bool,                  // default: true
--   }
--
-- ── Types removed from current codebase ─────────────────────
--
--   CapabilityTier       → replaced by context policy tables
--   AgentMode            → replaced by context policy tables
--   ContextPolicy        → split into ContextGatheringPolicy + AgentLoopConfig
--   ExtractionStrategy   → replaced by EditFormat enum
--   EditMode             → implicit: selection uses edit_format, chat uses tools
--
-- ── Chat edit format engines (Rust) ─────────────────────────
--
--   "apply_patch":
--     Parser: find *** Begin/End Patch markers, parse file ops + hunks
--     Matcher: exact → trimmed whitespace → fully trimmed (Codex spec)
--     On failure: error feedback with closest match, retry
--     Fallback: codeblock (after retries exhausted)
--
--   "str_replace":
--     Parser: find <<<<<<< SEARCH / ======= / >>>>>>> REPLACE blocks
--     Matcher: exact → whitespace-normalized → fuzzy (Levenshtein > 0.8)
--     On failure: error feedback with closest match, retry
--     Fallback: codeblock (after retries exhausted)
--
--   "codeblock":
--     Parser: find first ``` block, strip language tag
--     No matching needed — output IS the edit
--     Universal fallback for all providers
--
--   "hashline" (Lua-implemented, registered above):
--     Parser: Lua extract function
--     Matcher: hash-based line addressing (handled in Lua)
--     Application: Rust applies the structured edits to the rope
--
-- ── Indentation handling ─────────────────────────────────────
--
--   Models generally produce code without leading indentation.
--   For inline edits (selection replacement), ovim computes the
--   correct indentation using the same rules as `o` and `O`
--   commands (based on the surrounding context in the buffer).
--   The model's internal indentation (relative indent between
--   lines) is preserved; only the leading indent is adjusted.
--
-- ── Snapshot flow ───────────────────────────────────────────
--
--   Lua VM                    EditorBridge (Mutex)           AiState
--   ────────                  ────────────────────           ───────
--   vim.ai.setup()         → bridge.ai_profiles
--   vim.api_keys.register()→ bridge.api_key_registry
--   vim.ai.prompts = {}    → (read during sync)
--   vim.ai.chat = {}       → (read during sync)
--   vim.ai.agent = {}      → (read during sync)
--   vim.ai.project_context → (read during sync)
--   vim.ai.formats.reg()   → bridge.format_registry
--                                                    ┐
--   sync_ai_config_from_bridge() runs each tick:     │
--     profiles    → ai_state.config.profiles          │
--     contexts    → ai_state.config.contexts          │
--     api_keys    → ai_state.api_key_registry         │
--     prompts     → ai_state.prompt_templates         ├─ main thread
--     chat cfg    → ai_state.chat_context_config      │
--     agent cfg   → ai_state.agent_loop_config        │
--     proj ctx    → ai_state.project_context_config   │
--     formats     → ai_state.format_registry          │
--                                                    ┘
--   Async request path reads from ai_state (snapshot).
--   Never touches the Lua VM.
--
-- ── Prompt resolution ───────────────────────────────────────
--
--   For selection edits:
--     profile.edit_prompt
--       → ai_state.prompt_templates["selection_{edit_format}"]
--       → format.prompt (from format registry, if Lua format)
--       → system_prompt_for_extraction(format)  // Rust fallback
--
--   For chat:
--     profile.chat_prompt
--       → ai_state.prompt_templates["chat"]
--       → hardcoded chat prompt  // Rust fallback
--
--   For chat edits:
--     profile.chat_edit_prompt
--       → ai_state.prompt_templates["chat_edit_{chat_edit_format}"]
--       → format.prompt (from format registry, if Lua format)
--       → Rust fallback per format
--
-- ── Project context injection ─────────────────────────────────
--
--   After resolving the system prompt, before building user prompt:
--   1. Walk from current file's directory up to repo root
--   2. At each level, check for files in project_context.files order
--   3. Load all matching files, deeper files first (more specific wins)
--   4. Concatenate, truncate to project_context.budget tokens
--   5. Inject between system prompt and user prompt as:
--      "## Project Context\n{content}"
--
--   Caching: project context files are read once per session and
--   cached. Cache invalidated on file change (if watching) or on
--   :AI reload command.
--
--   For selection edits: project context competes with code context
--   for the profile's context.budget. If project context is 1,500
--   tokens and budget is 2,500, only 1,000 tokens remain for code.
--
--   For chat: project context is injected once in the system prompt.
--   It doesn't count against per-turn context budgets.
--
-- ── API key resolution ──────────────────────────────────────
--
--   profile.api_key = "openai"
--     → ai_state.api_key_registry["openai"]
--     → try config.env_var (std::env::var)
--     → try config.file (read + trim)
--     → error with clear message
--
-- ── Edit format resolution ──────────────────────────────────
--
--   EditFormat::Codeblock
--     → extract_codeblock(response) in Rust
--
--   EditFormat::Json
--     → extract_json(response) in Rust
--
--   EditFormat::Raw
--     → use response verbatim
--
--   EditFormat::ApplyPatch
--     → parse_apply_patch(response) in Rust
--     → layered matching against buffer content
--     → apply edits in reverse order
--
--   EditFormat::StrReplace
--     → parse_str_replace(response) in Rust
--     → layered matching against buffer content
--     → apply edits in reverse order
--
--   EditFormat::Lua("hashline")  (or any registered name)
--     → look up format in format_registry
--     → call format.extract(response) on main thread
--     → Rust applies the structured edits to the rope
--
--   When a profile's edit_format is an inline Lua function:
--     → auto-registered as Lua("__anon_N")
--     → same path as above
--
-- ── Retry protocol ──────────────────────────────────────────
--
--   Phase A: Extraction retry (async side)
--
--   1. Call API with primary format
--      ├── extract succeeds → apply edit
--      └── extract fails → construct error feedback message
--
--   2. Retry with error context appended:
--      "Your response could not be parsed. Error: {detail}.
--       Please respond with {format_instructions}."
--      ├── extract succeeds → apply edit
--      └── retries exhausted → try fallback format (if set)
--
--   3. Fallback format (if profile.retry.fallback is set):
--      Re-prompt with fallback format's system prompt
--      ├── extract succeeds → apply edit (with "fallback used" note)
--      └── extract fails → report to user
--
--   Phase B: Elision detection (after successful extraction)
--
--   After extraction succeeds, scan the replacement for elision:
--     /^\s*\/\/\s*\.\.\./
--     /^\s*\/\/\s*rest of/i
--     /^\s*\/\/\s*remaining/i
--     /^\s*\/\/\s*unchanged/i
--
--   If detected in a selection edit:
--     → re-prompt: "Do not use placeholder comments. Include ALL code."
--   If detected in a chat edit:
--     → set warning flag on the result for the hover panel
--
-- ── Syntax check protocol ───────────────────────────────────
--
--   After applying an edit (if profile.syntax_check == true):
--   1. Run tree-sitter incremental parse on the modified region
--   2. Compare syntax errors before vs after the edit
--   3. If new errors introduced:
--      → Mark the edit region as "applied with warnings"
--      → Show warning in hover panel: "Edit introduced N syntax error(s)"
--      → User can still accept or revert via hover actions
--
-- ── Provider parameter guards (Rust enforced) ───────────────
--
--   OpenAI + reasoning_effort != "none":
--     → strip temperature from request body
--     → use max_completion_tokens instead of max_tokens
--     → inject body["reasoning"] = { effort: "..." }
--
--   OpenAI + verbosity:
--     → inject body["text"] = { verbosity: "..." }
--
--   Anthropic:
--     → max_tokens is required (not optional)
--     → system prompt goes in body.system, not messages
--
--   Ollama:
--     → temperature goes in body.options.temperature
--     → no API key needed
--
-- ── Chat context management (Rust enforced) ─────────────────
--
--   When serializing chat messages for the API:
--   1. Turns within observation_window: full content
--   2. Older turns: tool results replaced with mask_template
--   3. If total tokens > max_context_tokens: drop oldest masked turns
--   The full conversation is always kept in memory for display.
