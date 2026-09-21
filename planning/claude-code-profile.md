# Claude Code profile

Use the official Claude Agent SDK with the user's unmodified installed Claude Code.
Ovim retains its chat presentation in GUI and TUI; Claude owns inference, tools,
permissions, settings, skills, compaction, credentials, and native sessions.
No subscription credential reading, OAuth implementation, HTTP impersonation,
or Ovim tool-loop execution is part of this integration.

## Boundaries

- A profile declares whether it is inference for Ovim or an external agent.
- External agent events distinguish observations from requests to execute tools.
- Permission callbacks pause Claude and receive explicit decisions through the
  shared editor UI. User questions use the composer. No automatic approval.
- Native checkpoints are reusable only for the same branch, configuration and
  visible history. Otherwise start a fresh native session with the visible
  conversation as context. Interrupted turns never publish a reusable checkpoint.
- Cancellation and editor shutdown stop the helper and its process tree.
- Read-only chats restrict available Claude tools. Ovim-specific approval and
  comprehension controls cannot claim to govern Claude-owned tools.
- Codex defaults remain unchanged. Runtime profile selection updates the chat
  default for this editor session; Lua configuration controls startup defaults.

## Packaging

A small Node helper uses the official SDK. The pinned SDK module is stored,
compressed and unmodified, in the binary so GUI and TUI have identical behavior
without a runtime npm download. Node and Claude Code must be installed by the
user. The vendoring script and npm lockfile make updates reproducible.

## Acceptance

Verify profile parsing/defaults, no inference or delegated-agent routing, stream
ordering without duplicate text, observed tools never executing in Ovim,
approval allow/deny and user answers, native resume isolation, read-only mode,
malformed output/early exit, cancellation, and process cleanup. Exercise the
real Claude runtime in an isolated project and inspect both frontend projections.

Reference: T3 Code commit 371b52d9dad76876f84609a4cd0f80eaa757b69c,
apps/server/src/provider/Layers/ClaudeAdapter.ts. Consulted Anthropic's Agent SDK
permissions, subscription usage, and legal/authentication documentation.

## Editor MCP integration

The core tool registry owns editor-bridge eligibility alongside runtime-service
requirements. The three eligible builtins retain their existing schemas and core
handlers; the MCP projection additionally restricts open_file to existing files.
The shared MCP wire contracts were extracted from the existing external API.

The GUI does not run the terminal API loop, and the existing external MCP server
supports broad controls and cross-session discovery. The Claude helper therefore
hosts a private, stateless Streamable HTTP endpoint on loopback with a random
per-turn bearer token. Requests travel over the owning job's existing framed
channel and are answered on the shared editor thread. No session discovery or
public listener is involved. Tool schemas and dispatch eligibility come from the
core registry; arbitrary commands and writes are not exported.

Read-only queries use strictMcpConfig with only the editor server. Walkthroughs
have a provider-response continuation, correlated cancellation, and snapshot
replay binding. A walkthrough question is returned in the MCP result without
also queueing another turn. The editor server's tool timeout allows up to 24
hours for user interaction; cancellation and editor shutdown still close it.

Anthropic documents MCP and prompt appending as extension mechanisms. These are
not a product-specific approval. Authentication remains in the user's unmodified
Claude executable. Public product distribution must respect Anthropic's separate
authentication, commercial and branding requirements; the picker uses Claude Agent.
