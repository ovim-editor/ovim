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
