# Claude Code profile validation

## Local checks (2026-09-21)

- `cargo test -p ovim-core --lib`: 1,749 passed, 1 existing ignored.
- `cargo test -p ovim --lib`: 305 passed, 2 existing ignored.
- `cargo clippy -p ovim --all-targets --locked -- -D warnings`: passed.
- `npm test --prefix ovim/claude-runtime`: 12 passed, including an independent official MCP client.
- In `ovim/gui`, `npm run check`: passed; `npm test -- --run`: 107 passed.
- In `ovim/gui`, `npx playwright test --project=chromium`: all 12 scenarios
  passed, including all three Claude scenarios. Inspected approval, question and walkthrough screenshots.
  The editor remains visible; permission text preserves line breaks; Allow once
  and Deny buttons fit the panel; a Norwegian answer with emoji remains intact;
  both Claude and Codex appear in the picker; Ovim-only policies are hidden.
- WebKit launch was blocked before tests ran: host lacks libicu74, libxml2 and
  libflite1 expected by Playwright. Run the same tests on a supported WebKit host.

The Rust tests exercise GUI projection and terminal rendering, provider/default
selection, active-turn guards, observed tools never reaching Ovim's executor,
permission answers and cancellation, question validation, native checkpoint
persistence and history isolation, attachment reconstruction, malformed protocol
frames, SDK asset integrity, and process-tree cleanup. Node tests use an injected
SDK query fixture and do not consume Claude usage. Browser scenarios use runtime
projection fixtures, not a live model.

## Editor bridge checks

Core tests cover the narrow generated tool list, replacing a definition without
inheriting its bridge authority, invalid inputs, file creation rejection, absolute
and symlink path escapes, code walkthroughs in folders without Git, unsaved-buffer preservation, live context, correlated
walkthrough cancellation, questions without duplicate queued turns, native tool
history and snapshot replay. Node checks cover authentication, cross-turn token
rejection, browser-origin rejection, request-size limits, notifications, cancellation,
and endpoint shutdown. The official MCP client completes initialize, tools/list
and tools/call against the actual HTTP transport without any model requests.

## Live validation limit

This machine cannot make successful Claude requests with its signed-in account.
Both the official SDK and the unmodified `claude -p` previously reported:
“Your organization has disabled Claude subscription access for Claude Code.”
The user confirmed that usable Claude access is on another machine. No further
live model requests were made after that clarification. Successful live edits,
native resume/compaction, and account-backed permission behavior remain unverified.

## Acceptance on a machine with Claude access

Use an isolated disposable project with a small source file and a runnable test.
Install Node and Claude Code; authenticate through Claude's normal terminal flow.
Exercise these scenarios in both GUI Ovim and terminal Ovim:

1. Start with the shipped configuration: Codex is selected. Choose `claude_code`
   in the picker or with `/model claude_code`; close/reopen chat and open a query.
   Claude remains selected. Set `vim.ai.default_profile = "claude_code"` and
   restart Ovim to verify the configured startup default.
2. Ask Claude to explain the open file and then make a small tested change.
   Check streaming text, observed tool rows, disk changes, buffer refresh, and
   test output. Check behavior with an unsaved buffer before accepting disk edits.
3. Request an action that Claude's own settings require approving. Deny it, then
   request it again and allow once. Verify denial prevents the action and approval
   applies only to that invocation. Ask Claude to use AskUserQuestion and answer
   through the composer, including a free-text answer with non-ASCII characters.
4. Send a follow-up, restart Ovim, and continue the same durable conversation.
   Verify context is retained. Run `/compact` after a successful native turn;
   continue chatting. Clear the conversation and verify old context is absent.
5. Cancel while a tool is running. Verify the process tree stops, unfinished
   outcomes are reported as unknown, and the next turn does not blindly resume
   an interrupted checkpoint. Close Ovim during another running turn and check
   for surviving helper/Claude/tool processes.
6. Ask Claude to call workspace_context after switching files, open a different
   existing file, and use explain_with_codebase. Verify the walkthrough, questions,
   completion/dismissal and replay in both frontends. While it is active, verify
   navigation cannot replace the walkthrough. Open another Ovim window and verify
   the agent cannot select it through this connection.
7. Open a read-only query and request an edit. Verify no write tools are exposed,
   while Ovim context and navigation remain available.
   Switch back to Codex after the turn and verify its existing controls return.

## Follow-up: model selection and macOS startup (2026-09-21)

Acceptance criteria: the GUI and terminal select a model within the Claude
profile; the configured model reaches the official SDK unchanged; changing
profiles preserves the choice and unsent draft; active turns reject changes
without partial mutation. Both interfaces display the same effective effort as
the runtime request. Fable defaults to Medium, Opus/Sonnet to High, and Haiku
omits effort even after a previous override. User/profile effort overrides take
precedence where supported. Narrow terminal pickers keep the selected row visible
and mouse targets match the scrolled rows.

The exact current model IDs were checked against Anthropic's model reference
and Claude Code configuration documentation. The provider owns a single curated
preset table; it does not claim account-specific availability. Arbitrary explicit
IDs/aliases remain selectable through `/model`. No authentication or entitlement
checks are replaced.

The screenshot's silent exit was reproduced without inference: Node canonicalizes
its entry module URL but preserves a symlink in argv. The helper's old comparison
silently skipped startup through aliased directories, including the path shape
used by macOS temporary directories. A real subprocess regression test copies the
helper beside an SDK fixture and launches through both direct and symlinked paths,
including spaces and Unicode. The fixture cannot make model requests. Both paths
now emit the requested model's text and a completed turn. This verifies the
startup defect locally; a successful real Claude response on macOS still requires
the user's other machine/account.

Validation commands and evidence:

- `cargo test --workspace --lib`: native/frontend and core behavioral suites.
- `cargo clippy -p ovim --all-targets --locked -- -D warnings`: native GUI included.
- `npm test --prefix ovim/claude-runtime`: 14 offline SDK/transport tests, including
  the direct/aliased subprocess regression and exact Fable model/effort forwarding.
- `npm run check --prefix ovim/gui` and GUI unit tests: reactive model selection,
  selected-row identity, exact model callback arguments, and composer focus.
- Chromium Playwright: 13 scenarios, including the actual model picker at
  1154×1054. Inspected `claude-model-picker.png`: all exact IDs visible, Fable
  selected, default Medium shown, and the popover leaves the composer accessible.
- Rebuilt tracked GUI assets so installed builds include the selector changes.

On an account with access, additionally select Fable/Opus/Sonnet before the first
message, verify the response and displayed effort, and switch models between
turns. The no-live-inference constraint and unavailable WebKit dependencies from
the earlier validation still apply.
