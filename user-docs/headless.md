# Headless & Automation

Headless mode is designed for tests, CI, and automation. It runs ovim without
the TUI and exposes an authenticated loopback API. Ordinary interactive TUI
sessions do not open an API listener.

## Start a Session

```bash
ovim path/to/file.rs --headless --session dev --dimension 100x30
```

The logical viewport is initialized before the API starts accepting input.
Motions, wrapping, scrolling, snapshots, and renders therefore use the same
dimensions. Change it later with `ovim resize -s dev 120x40`.

At startup, Ovim creates a cryptographically random bearer capability in the
owner-private session descriptor. Built-in `ovim` commands and the MCP stdio
broker read and use it automatically. Treat the descriptor like a credential:
do not copy it into logs, bug reports, shell history, or a repository.

## Inspect and Control Sessions

### Subcommands

```bash
ovim session list
ovim session health -s dev
ovim snapshot -s dev --format pretty
ovim send -s dev "iHello<Esc>"
ovim paste -s dev 'literal text\nincluding newlines'
ovim resize -s dev 120x40
ovim exec -s dev "w"
ovim session kill -s dev
```

JSON snapshots carry a `schema_version` and a `view` object containing viewport,
scroll, tab, split, file-tree, command/search, and status state. They include an
`ai_chat` object whenever a chat is active. It reports focus, streaming/review
state, current composer text and cursor, pending approval, scheduled inputs,
and message history. The `activity` field is the authoritative lifecycle state:
`idle`, `inference`, `classifying_tool`, `running_shell`, `running_web`,
`waiting_tool_approval`, or `waiting_folder_approval`. Prefer it over inferring
ownership from compatibility booleans such as `waiting` and `streaming`. The
`attention_generation` value increases for each new blocking agent approval,
so a headless client can raise its own notification once per prompt. Completed
tool messages expose a compact summary; their arguments appear when that tool
row is expanded in the UI. Automation can therefore monitor turns without
parsing the rendered terminal grid.

`ovim send` accepts Unicode and Vim-style key names/modifiers. Use `ovim paste`
for literal or multiline input so it is delivered as one bracketed-paste event.

### Editing a live session

The file-operation commands normally read or write the file directly. Add
`--session` to operate on the live editor buffer instead:

```bash
ovim edit src/main.rs --old before --new after --session dev
ovim insert src/main.rs --after 10 --text 'new line' --session dev
ovim delete-lines src/main.rs --from 20 --to 22 --session dev
ovim read-lines src/main.rs --from 1 --to 30 --session dev
ovim exec -s dev w
```

The file argument must match the session's active buffer. Session-aware edits
remain unsaved until `:w`, preserving undo, LSP synchronization, diagnostics,
and render invalidation. A clean headless buffer automatically reloads external
disk changes. If the buffer has local changes, ovim keeps them and refuses a
plain `:w` rather than overwriting the external version; use `:e!` or `:w!` to
make that choice explicitly.

AI chat uses the same background poller and input dispatcher in headless mode
as in the TUI. Open editable chat with `Space Space`, type a request, and submit
with Enter:

```bash
ovim send -s dev "  "
ovim send -s dev "inspect the project<Enter>"
```

For unattended use, complete Ovim's contextual Codex sign-in once before
starting the session. The credential is stored in the same platform config
directory and refreshes automatically. If credentials need renewal during a
headless session, the auth dialog blocks inference without consuming the
draft. Device-code sign-in can be completed entirely through the headless API:

```bash
ovim send -s dev D
ovim snapshot -s dev --format json
```

Wait for `ai_chat.codex_auth.phase` to become `waiting_for_device_code`, then
open its `verification_url` in any browser and enter its `user_code`. Ovim
continues automatically after approval. Device-code login must be enabled in
your ChatGPT security settings or by your workspace administrator. You can
send `B` instead to use the localhost browser callback, or `<Esc>` to cancel
without losing the draft.

If auto mode pauses an Ovim tool for approval, the agent round remains blocked
until the decision arrives. Inspect it with `ovim snapshot -s dev`, then
send `<C-y>` (or `<Enter>`) to allow once, or `<C-n>` (or `<Esc>`) to deny. The
50 ms headless background tick also polls Terra classifier completions; no
renderer or attached terminal is required.

For a trusted session, enter `/yolo on` in the chat composer to bypass Terra and
interactive approvals for that chat; `/yolo off` restores normal policy. The
snapshot's `ai_chat.yolo_mode` field reports the current setting.

Comprehension policy is also scriptable: submit `/comprehension publish`,
`/comprehension commit`, or `/comprehension off`. Snapshots report the selected
mode in `ai_chat.comprehension_policy` and include
`ai_chat.comprehension_checkpoint` only while a recorded checkpoint still
covers the current repository content. Comprehension gates remain active in
YOLO mode.

LSP helpers:

```bash
ovim lsp wait -s dev --timeout 30000
ovim lsp status -s dev
ovim lsp hover -s dev
```

## Session Files

Session files are JSON and live in:

- macOS: `~/Library/Caches/ovim/sessions`
- Linux: `~/.cache/ovim/sessions`

The directory is owner-only on Unix and each descriptor is created with mode
`0600`. A descriptor contains the session's port, process metadata, active file
path, and bearer capability.

Override with:

```bash
export OVIM_SESSION_DIR=/path/to/ovim-sessions
```

## Cleanup

Remove stale/expired/corrupted sessions:

```bash
ovim session cleanup --dry-run
ovim session cleanup
ovim session cleanup --max-age 7
```

## Output & logs

- Headless mode may print basic status/errors to stderr (safe without the TUI).
- For debugging, check `ovim.log` and `lsp.log` in the ovim cache dir (see `troubleshooting.md`).

## REST API (reference)

When headless, ovim exposes endpoints like:

- `GET /v1/health`
- `GET /v1/snapshot`
- `POST /v1/keys`
- `POST /v1/paste`
- `POST /v1/resize`
- `POST /v1/command`
- `POST /v1/edit`
- `POST /v1/insert`
- `POST /v1/delete-lines`
- `GET /v1/lines`
- `POST /v1/mcp`

The same routes are still served without the `/v1` prefix for backward
compatibility. Those unversioned paths are deprecated; new surface is published
only under `/v1`.

Use `ovim snapshot -s <name>` instead of calling the API directly unless you need custom tooling.

For custom tooling, read both `port` and `capability` from the named session
descriptor and send `Authorization: Bearer <capability>` on every request.
Requests with a missing/wrong capability, an unexpected Host, or any browser
Origin are rejected. The API intentionally has no browser CORS mode or remote
bind mode.

## GUI protocol (remote editing)

The same session API also carries the conversation the native GUI has with the
editor, which is what lets a GUI on one machine drive a headless session on
another. See [Remote editing](remote.md) for the user-facing feature; this is
the wire surface it rides on.

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/v1/gui/command` | POST | One GUI command in, its reply out |
| `/v1/gui/stream` | GET | Server-sent stream of projected frames |

Both are published only under `/v1`, and both sit behind the same bearer
capability and Host guard as everything else.

A command is a tagged JSON object — `command` names the variant and `payload`
carries its fields:

```bash
PORT=... CAP=...   # both from the session descriptor

curl -s -X POST "http://127.0.0.1:$PORT/v1/gui/command" \
  -H "Authorization: Bearer $CAP" -H 'Content-Type: application/json' \
  -d '{"command":"snapshot","payload":{"columns":80,"rows":24}}'

curl -s -X POST "http://127.0.0.1:$PORT/v1/gui/command" \
  -H "Authorization: Bearer $CAP" -H 'Content-Type: application/json' \
  -d '{"command":"key","payload":{"input":{"key":"G"}}}'
```

The reply is `{"reply":…,"result":{"Ok":…}}`. A command with no answer to give
(`shutdown`) returns `204 No Content` instead. A command that failed *inside the
editor* still answers `200`, with `{"Err":"…"}` in place of `Ok`, so transport
failures and editor failures stay distinguishable.

`GET /v1/gui/stream` is Server-Sent Events with `event: snapshot` and a 15 s
keep-alive:

```bash
curl -N "http://127.0.0.1:$PORT/v1/gui/stream" -H "Authorization: Bearer $CAP"
```

A frame is a complete, viewport-sized projection of the editor — resolved
highlight segments, layout tree, panels, theme — published on change rather
than on a timer. Frames are only projected while somebody is subscribed, so an
automation session that never opens the stream pays nothing for it.

Two caveats for anyone writing a client:

- **Set the `Host` header explicitly** when you reach the session through a
  forwarded port. The guard compares `Host` against the *session's own* port,
  which under `ssh -L` is not the port you dialled. An HTTP client that derives
  `Host` from the URL will get `403` on every request.
- **`{"command":"shutdown"}` stops the session.** Do not wire it to a window
  closing; that is exactly what remote reattach depends on not happening.
