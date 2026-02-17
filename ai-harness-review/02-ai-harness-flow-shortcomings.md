# AI Harness Flow and State Shortcomings

Date: 2026-02-17
Scope: Behavior-level issues in AI chat/tool orchestration

## Executive Summary

Beyond security boundaries, there are several correctness issues in buffer targeting and chat state tracking that can make the agent appear "confused" (especially when switching buffers, opening files via tools, or running in review mode).

This document captures shortcomings and remediation paths.

## F1. Conversations And Agent State Use Unstable Buffer Indices

Evidence:
- Persistent conversations keyed by `(buffer_id, conversation_name)` where `buffer_id` is index (`ovim-core/src/editor/ai_state.rs:87`).
- Chat opens with `buffer_id = current_buffer_index` (`ovim-core/src/editor/ai_chat.rs:17`).
- Conversation lookup relies on `origin_buffer_id` (`ovim-core/src/editor/ai_chat.rs:574`).
- Buffer delete/add operations shift indices (`ovim-core/src/editor/buffer_manager.rs:182`, `ovim-core/src/editor/buffer_manager.rs:195`).

Impact:
- After buffer topology changes, conversations and edit-tracking can point at the wrong buffer.

Fix:
1. Introduce stable `BufferId` (u64 counter or UUID) on `Buffer`.
2. Key conversation and agent-edit maps by `BufferId` instead of index.
3. Keep a separate runtime mapping `index -> BufferId` and update on add/remove.
4. Migrate existing in-memory maps on startup/version bump.

Validation:
- Add/remove/reorder buffers does not reattach chat history to another file.

## F2. Read Context And Mutation Target Can Diverge

Evidence:
- Read tool context snapshots from `current_buffer_index` (`ovim-core/src/editor/ai_chat_tools.rs:79`).
- Mutation tools switch to `active_buffer_id` (`ovim-core/src/editor/ai_chat_mutations.rs:297`, `ovim-core/src/editor/ai_chat_mutations.rs:313`).
- `open_file` updates `active_buffer_id` and toggles review mode (`ovim-core/src/editor/ai_chat_mutations.rs:151`).

Impact:
- Agent can read one buffer but mutate another, especially after navigation tools or manual user buffer changes.

Fix:
1. Define one authoritative target for the whole tool loop turn.
2. Build read context from `active_buffer_id` when chat is active.
3. Include active target file in each tool result header for traceability.
4. Hard-fail if active target becomes invalid mid-turn.

Validation:
- In multi-buffer scenarios, read/write operations stay on the same intended file.

## F3. Review Mode Delegates Most Keys To Normal Mode

Evidence:
- In review mode, unhandled keys are passed to normal mode handler (`ovim-core/src/editor/input/ai_chat_mode.rs:33`).

Impact:
- Unintended normal-mode actions can interfere with in-flight chat context and active buffer assumptions.

Fix:
1. Replace broad delegation with explicit review-mode keymap allowlist.
2. Gate high-impact actions while an AI job is pending.
3. Add a visible review-mode state indicator with target file and pending status.

Validation:
- Review mode cannot accidentally trigger unrelated normal-mode edits while waiting on AI.

## F4. No-File-Open Behavior Is Inconsistent And Can Feel Confusing

Evidence:
- Editor state prompt path returns early with "No file open." (`ovim-core/src/editor/ai_chat_tools.rs:289`).
- LSP tools error when file path is missing (`ovim-core/src/editor/ai_chat_tools.rs:465`).
- `read_file` returns empty-buffer guidance (`ovim-core/src/ai/tools/builtins.rs:109`).
- Project-level tools may still be available via `cwd` root depending on config/path resolution (`ovim-core/src/editor/ai_chat_tools.rs:98`).

Impact:
- Agent appears inconsistent: "no file open" for some tools but can still roam project-level paths.

Fix:
1. When no file is open, default to file-scope tools only (or no project tools in `repo_only` mode).
2. Return a consistent "open/select a file first" guidance block.
3. Optionally auto-bind chat to the first real file buffer when available.

Validation:
- With unnamed/empty buffer, tool availability and messages are deterministic and consistent.

## F5. Auto-Save Policy Is Correct But Opaque

Evidence:
- Agent edits auto-save only when buffer was clean at chat start (`ovim-core/src/editor/ai_chat_mutations.rs:345`).

Impact:
- Users can interpret resulting behavior as inconsistent save semantics.

Fix:
1. Surface auto-save policy in chat UI status.
2. Add config options:
   - `always`
   - `only_if_clean_at_start` (current)
   - `never`
3. Include save action outcome in tool result text when mutation completes.

Validation:
- Users can predict whether agent edits persist to disk immediately.

## Cross-Cutting Implementation Plan

1. Add stable `BufferId` and migrate AI maps.
2. Unify active-target semantics for read/write tools.
3. Harden review mode key handling.
4. Align no-file-open behavior with boundary policy.
5. Add explicit UI/status messaging for save semantics and target file.

