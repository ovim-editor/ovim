# AI Chat UX and Harness Implementation Plan

Date: 2026-02-17  
Scope: AI chat UX, review flow, tool event presentation, harness tool primitives, and supporting architecture.

## Executive Summary

This plan combines:
- AI chat UX changes (docked chat while agent runs, cleaner review flow, compact tool activity, improved styling).
- Interaction upgrades (chat link hover/click, chat text selection/copy, hide empty tool-only assistant messages).
- Harness/tooling improvements from the external agent review (file creation/writing, path-addressed edits, project diagnostics, checkpoint/undo).
- Security/scope alignment from prior reviews (`01-security-scope-and-egress.md`, `02-ai-harness-flow-shortcomings.md`).

Primary default behavior:
- Keep chat visible on the right while the agent is working.
- Do not auto-close chat after a turn by default.
- Let users explicitly switch to an edits-focused view.

## Product Decisions

1. Chat visibility while agent works:
- Keep right-side chat docked during streaming and tool execution.
- Do not force `review_mode` from tool handlers (`open_file`, `select_text`) by default.

2. Review flow:
- Keep an explicit edits-focused mode for deep review.
- Enter/exit via explicit user action (`Ctrl-r`) or optional post-turn policy.

3. Post-turn auto behavior:
- Default: `stay` (chat remains open).
- Optional setting later: `focus_edits_if_changed`.

4. Tool activity presentation:
- Replace verbose tool result rows with compact structured summaries.
- Use disambiguated repo-relative paths and delta summaries (`+N -M`) for mutation tools.

5. Message styling:
- Replace border-heavy bubble style with compact cards/rows.
- Distinct background/text styles for user, assistant, and thinking content.

6. Empty responses:
- Hide assistant messages with empty text when they only carry tool calls.
- Preserve protocol data internally for provider retries/continuation.

## Architecture Direction

1. View state model:
- Replace bool-like review behavior with an explicit view mode enum:
  - `DockedChat`
  - `ReviewFocused`
- Keep chat state and review state orthogonal to tool dispatch.

2. Tool timeline model:
- Introduce structured tool events separate from raw chat message text.
- Suggested struct:
  - `tool_name`
  - `tool_kind` (Read, Navigation, Mutation, Diagnostics, Search, External, Error)
  - `target_path` / `query`
  - `summary`
  - `stats` (`added`, `removed`, `count`, `duration_ms`, `status`)
  - `timestamp`

3. UI rendering split:
- Keep provider transcript model intact.
- Render chat timeline from:
  - assistant/user/thinking messages
  - compact tool event rows
- Avoid parsing human-form text to derive UI metadata.

4. Path display utility:
- Add shared path display formatter:
  - repo-relative by default
  - minimal disambiguating parent segments
  - middle ellipsis when needed

5. Input/mouse model for chat:
- Add chat hit-test cache in `RenderCache` for links and selectable text ranges.
- Add explicit mouse handling branch for chat interaction (not only scroll).

## Workstreams

## WS1. Docked Chat While Agent Works

Goals:
- Keep chat visible during streaming/tool execution.
- Preserve ability to inspect edits in buffer simultaneously.

Changes:
- Stop auto-toggling `review_mode` in tool handlers (`open_file`, `select_text`) by default.
- Keep layout split active unless user explicitly enters `ReviewFocused`.
- Maintain current buffer scrolling while chat is open.

Acceptance:
- During streaming/tool loops, chat remains on right.
- Main editor can still be navigated/scrolled.
- No involuntary mode flips caused by tool calls.

## WS2. Compact Tool Activity Rows

Goals:
- Make tool activity readable, compact, and non-intrusive.

Changes:
- Add a `ToolEventSummary` pipeline in `ai_chat_tools`.
- Render one-line summaries with color by kind:
  - Mutation: `src/path/file.ts +10 -6`
  - Navigation: `src/path/file.ts:248`
  - Read: `read_file_at_path src/path/file.ts:120-180`
  - Search/list: `search \"query\" 23 matches`, `list_files src/ 120 files`
  - Diagnostics: `read_diagnostics E2 W5`
  - Errors: explicit error text in red
- Use disambiguated partial path logic to avoid ambiguous leaf names.

Acceptance:
- Tool rows remain single-line in normal width.
- Mutation rows include reliable `+/-` deltas.
- Path labels are unambiguous within the current turn.

## WS3. Message Styling Revamp

Goals:
- Improve density and readability; reduce chrome noise.

Changes:
- Move from border-heavy bubbles to compact row/card styling.
- Define a small style system:
  - user row style
  - assistant row style
  - thinking row style
  - tool event row styles by kind
- Keep branch/tree affordances and selection highlight legible.

Acceptance:
- Message area is visibly denser and easier to scan.
- Thinking content is visually distinct but not overpowering.

## WS4. Hide Empty Tool-Only Assistant Messages

Goals:
- Remove noisy blank assistant messages.

Changes:
- Render rule: if assistant message `content.trim().is_empty()` and only tool calls are present, suppress message row in UI.
- Keep underlying message data in conversation for protocol correctness.

Acceptance:
- No blank assistant bubble/row appears before/after tool activity.

## WS5. Link Hover/Click and Chat Text Selection

Goals:
- Let users interact with references in chat output.

Changes:
- Extend markdown parser/renderer to capture links and raw URLs.
- Cache rendered link hitboxes per frame.
- Mouse hover highlights link.
- Click behavior:
  - default `copy_link` + status toast
  - optional `open_with_confirm` in config
- Add chat text selection model for copy operations.

Acceptance:
- Hovering a link changes style.
- Clicking a link copies or opens based on configured behavior.
- Users can select and copy chat text without breaking existing editor selection behavior.

## WS6. Review UX and Shortcut Clarity

Goals:
- Keep review controls discoverable and consistent.

Changes:
- Keep compact floating shortcuts card in review-focused mode.
- Ensure status line includes:
  - review mode
  - edit count/file count
  - active target file hint
- Maintain arrow-left/right navigation and Enter accept behavior.

Acceptance:
- Users can always discover how to navigate/accept/exit review mode.

## WS7. Harness Primitive Upgrades (External Review Mapping)

Goals:
- Make multi-file refactors first-class and safer.

Changes:
- Add/create path-addressable primitives:
  - `create_file(path, content)`
  - `write_file_at_path(path, content)` (create or overwrite)
  - `apply_patch_at_path(path, diff)` or multi-file unified patch tool
  - `rename_file(from, to)` (optional initial phase)
  - `delete_file(path)` (optional initial phase)
- Add path-addressed edit tools:
  - `edit_range(path, ...)`
  - `insert_lines(path, ...)`
  - `delete_lines(path, ...)`
- Add diagnostics expansion:
  - `read_diagnostics(path)`
  - `read_project_diagnostics()`
- Add recovery:
  - `snapshot_file(path)` / `restore_file(path, snapshot_id)` or `undo_last_edit(path)`
- Optional ergonomic extension:
  - `open_file(path, create=true)`

Acceptance:
- Agent can perform clean multi-file extraction without wrong-buffer writes.
- Multi-file diagnostics are inspectable during refactors.
- Recovery exists for bad edit batches.

## WS8. Security and Scope Alignment

Goals:
- Keep harness improvements safe by default.

Changes:
- Reuse shared boundary/path policy from security plan:
  - repo-root boundary by default
  - explicit no-repo session approval root
  - sensitive-path guardrails
- Ensure all new path-addressed tools call the same validator.

Acceptance:
- New tools do not bypass existing project boundary/sensitivity policies.

## Phased Delivery

## Phase 1 (Foundational UX + State)

Includes:
- WS1 (docked chat during work)
- View-mode enum refactor (core architecture)
- Removal of implicit review mode toggles in tool handlers

Commit strategy:
- Commit A: state model refactor (enum + wiring)
- Commit B: layout/input behavior updates

## Phase 2 (Tool Timeline + Empty Message Suppression)

Includes:
- WS2 (structured compact tool rows)
- WS4 (hide empty tool-only assistant rows)

Commit strategy:
- Commit C: tool summary model + formatter utilities
- Commit D: renderer integration + suppression logic

## Phase 3 (Styling + Review UX Polish)

Includes:
- WS3
- WS6

Commit strategy:
- Commit E: row/card style system + renderer migration
- Commit F: review overlays/status polish

## Phase 4 (Links + Chat Selection)

Includes:
- WS5

Commit strategy:
- Commit G: markdown/link parsing + hit-test cache
- Commit H: mouse interaction + copy/open behavior

## Phase 5 (Harness Primitive Expansion)

Includes:
- WS7
- WS8

Commit strategy:
- Commit I: create/write/path-addressed edit primitives
- Commit J: diagnostics/snapshot tooling + policy integration

## Testing and Validation Plan

Automated:
- Unit tests:
  - tool summary/path disambiguation
  - empty-message suppression
  - view mode transitions
  - path-policy enforcement on new tools
- Integration tests:
  - chat stays docked through streaming + tool loops
  - path-addressed multi-file edit flow
  - review navigation behavior

Manual QA:
- Stream while scrolling chat and main buffer.
- Tool-heavy turn with mixed read/nav/mutation calls.
- Multi-repo folder (no git at cwd) with first-open approval prompt.
- Link hover/click and chat selection copy behavior.
- Regression checks for existing AI review shortcuts and edit markers.

## Risks and Mitigations

1. Risk: UI complexity regresses render performance.
- Mitigation: cache hit-test metadata and render summaries only from structured events.

2. Risk: transcript/protocol divergence.
- Mitigation: keep provider message model unchanged; treat timeline summary as presentation-only.

3. Risk: path-addressed tools bypass guardrails.
- Mitigation: single shared validator for all path-taking tools.

4. Risk: mouse interaction conflicts with existing buffer selection behavior.
- Mitigation: strict hit-testing boundaries (chat area vs buffer area) and mode-aware routing.

## Definition of Done

- Chat remains visible while agent is active by default.
- Edits-focused mode is explicit and clearly discoverable.
- Tool activity is compact and high-signal.
- Empty tool-only assistant responses are not shown as standalone chat rows.
- Link hover/click and chat text selection work reliably.
- Harness supports safe multi-file refactors via path-addressed create/write/edit tools.
- Security boundary policy is consistently enforced across all new tool entry points.
