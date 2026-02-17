# AI Harness Security, Scope, and Data Egress Review

Date: 2026-02-17
Scope: AI chat/tool execution in `ovim-core`

## Executive Summary

Current behavior is permissive enough that an AI tool call can read sensitive files (including `.env`-style files) and send their contents to remote providers if configured. The biggest root cause is boundary resolution: "project root" is currently `cwd`, not guaranteed repo root.

This document maps shortcomings and concrete fixes, including stricter behavior when not in a git repo and safer defaults for batteries-included usage.

## S1. Project Boundary Is `cwd`, Not Repo Root

Evidence:
- `ToolExecutionContext` sets `project_root` from `std::env::current_dir()` (`ovim-core/src/editor/ai_chat_tools.rs:98`).
- `open_file` navigation also uses `current_dir` as root (`ovim-core/src/editor/ai_chat_mutations.rs:101`).
- Project tools return an error that implies git-root detection, but runtime check only validates `project_root` presence (`ovim-core/src/ai/tools/builtins.rs:205`, `ovim-core/src/ai/tools/builtins.rs:438`, `ovim-core/src/ai/tools/builtins.rs:505`).

Risk:
- Starting ovim from a broad directory (for example home dir) expands tool read/open scope beyond intended project.
- "Not in git repo" does not currently enforce a tighter boundary in practice.

Fix:
1. Add a single resolver for execution root:
   - Start from current file parent when available, else `cwd`.
   - Prefer nearest ancestor containing `.git`.
   - Return both `repo_root: Option<PathBuf>` and `fallback_root: PathBuf`.
2. Add policy modes:
   - `repo_only` (default): project-level tools disabled when no git root.
   - `directory_fallback` (opt-in): use fallback root as project root.
3. Populate `ScopeContext.project_root` from this resolver, not raw `cwd`.
4. Update tool descriptions/errors to match real behavior.

Acceptance criteria:
- Outside git repo with default policy, `list_files`, `search_project`, `read_file_at_path`, and `open_file` are unavailable.
- Inside git repo, project tools are strictly bounded to repo root.

## S2. `.env` and Secret Material Are Readable By Default Project Tools

Evidence:
- `read_file_at_path` reads any file under root with no secret-file denylist (`ovim-core/src/ai/tools/builtins.rs:190`).
- File walker includes hidden files (`ovim-core/src/editor/grep.rs:46`).
- `list_files` and `search_project` can discover sensitive paths/content (`ovim-core/src/ai/tools/builtins.rs:422`, `ovim-core/src/ai/tools/builtins.rs:504`).

Risk:
- Accidental disclosure of credentials (`.env`, keys, tokens, certs) into chat history and provider requests.

Fix:
1. Introduce centralized path policy for "sensitive by default":
   - Deny examples: `.env`, `.env.*`, `*.pem`, `*.key`, `id_rsa`, `.aws/*`, `.ssh/*`, keychain exports.
2. Apply this policy in all path-taking tools (`read_file_at_path`, `list_files`, `search_project`, `open_file`).
3. Add explicit allow override:
   - one-shot approval for a specific file
   - or explicit config allowlist pattern.
4. Emit blocked-access tool result with clear UX reason.

Acceptance criteria:
- `.env` read attempts are blocked by default, with an explicit override mechanism.

## S3. Tool Calls Auto-Execute Without User Approval

Evidence:
- Tool calls are executed immediately in loop (`ovim-core/src/editor/ai_chat_tools.rs:229`).

Risk:
- A single model response can trigger broad project reads and chain additional reads without user gating.

Fix:
1. Add tool approval policy:
   - `auto` (only for low-risk local profiles).
   - `sensitive_prompt` (default): prompt for sensitive reads, project-wide scans, and all mutations.
   - `always_prompt`.
2. Classify sensitivity by:
   - provider type (remote vs local),
   - file sensitivity policy,
   - scope size (file vs project).
3. Show approval prompt with `tool`, `path/query`, and estimated output size.

Acceptance criteria:
- In default mode, project-wide read tool calls require approval.

## S4. Tool Outputs and Context Are Sent To Providers With Minimal Redaction

Evidence:
- Tool results appended into conversation (`ovim-core/src/editor/ai_chat_tools.rs:237`).
- Tool messages are serialized verbatim for OpenAI/Ollama and Anthropic formats (`ovim-core/src/ai/provider.rs:507`, `ovim-core/src/ai/provider.rs:552`).
- System prompt always appends project context and editor state (`ovim-core/src/editor/ai_chat_tools.rs:622`, `ovim-core/src/editor/ai_chat_tools.rs:629`).

Risk:
- Sensitive local data can egress to external APIs once read by tools.

Fix:
1. Provider-aware default protections:
   - Local provider: existing behavior allowed.
   - Remote provider: reduced context defaults (no automatic project context; smaller editor state budget).
2. Add redaction pass for high-risk token patterns before provider serialization.
3. Add max-byte caps on each tool result inserted into conversation/provider payload.

Acceptance criteria:
- Remote profiles send strictly reduced context by default.

## S5. Observation Masking Is Too Narrow

Evidence:
- Only old `Tool` role content is masked (`ovim-core/src/ai/chat_types.rs:399`).
- Default observation window is 10 turns (`ovim-core/src/ai/config.rs:77`).

Risk:
- Recent sensitive tool outputs are still sent verbatim; user/assistant messages remain fully unmasked.

Fix:
1. Add role-aware masking policy for remote profiles:
   - aggressive masking/summarization of tool outputs older than 1-2 turns.
2. Add optional masking for assistant echoes of tool output.
3. Reduce default remote observation window.

Acceptance criteria:
- Remote profile payload excludes old raw tool dumps by default.

## S6. Default Scope/Tool Config Is Broad and Hard To Restrict From TOML

Evidence:
- `ProfileScope::default()` is `FileScope::Project` (`ovim-core/src/ai/types.rs:48`).
- Empty tool list means "all allowed by scope" (`ovim-core/src/ai/tools/mod.rs:98`).
- Default profile uses `tools: []` and default scope (`ovim-core/src/ai/config.rs:161`, `ovim-core/src/ai/config.rs:162`).
- TOML profile struct does not expose `tools`/`scope` fields (`ovim-core/src/ai/config.rs:109`).

Risk:
- Batteries-included defaults are more permissive than necessary.
- Users cannot easily harden via `ai.toml`.

Fix:
1. Add TOML fields:
   - `tools = ["..."]`
   - `scope = "file|project|selection|any"`
   - `scope_shell`, `scope_network`
2. Change default profile file scope to `file` for chat contexts.
3. Provide curated default toolset for each context (`chat`, `query`, `selection`) instead of implicit "all".

Acceptance criteria:
- A user can lock down scope/tools from config without Lua.

## S7. Scope Validation Logic Exists But Is Not Reused By Tool Handlers

Evidence:
- `Capabilities::validate_path` exists (`ovim-core/src/ai/scope.rs:19`).
- Tool handlers perform ad hoc checks instead of shared validator (`ovim-core/src/ai/tools/builtins.rs:200`, `ovim-core/src/editor/ai_chat_mutations.rs:97`).

Risk:
- Boundary logic can drift across tools and create inconsistencies.

Fix:
1. Route all path validation through one shared helper (wrapping `validate_path` plus sensitive-path policy).
2. Make every path-taking tool call the same function.
3. Add regression tests that run each tool against same boundary cases.

## Recommended Implementation Order

1. Boundary enforcement: repo-root resolution + `repo_only` default.
2. Sensitive file policy + unified path validator.
3. Tool approval modes and provider-aware defaults.
4. TOML scope/tool controls and safer default toolset.
5. Redaction + masking refinements and payload caps.

