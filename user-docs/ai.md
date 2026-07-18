# AI Setup

This guide covers practical AI configuration in ovim:

- Recommended setup: Codex with a ChatGPT/Codex subscription
- Preferred setup: Lua API in `init.lua`
- Secure API key handling without `~/.zshrc`
- Legacy `ai.toml` compatibility

## Recommendation

Install the Codex CLI and sign in with ChatGPT once. Ovim imports that login on
first use, stores its own refreshable credentials as `ovim/codex-auth.json` in
the platform config directory (`~/.config` on Linux,
`~/Library/Application Support` on macOS), and calls the Codex Responses
transport directly:

```bash
npm install -g @openai/codex
codex login
codex login status
```

The built-in defaults use `gpt-5.6-sol` at medium effort for chat and
`gpt-5.6-terra` at low effort for selection edits and read-only queries. With
the default `codex` provider, Ovim—not Codex app-server—is the agent harness.
Ovim sends its own tool schemas, records tool intent, applies auto-mode policy,
executes approved effects in the repository, and returns results for the next
inference round. Codex's read-only workspace sandbox is therefore not involved
in repository reads or writes.

On the first direct-Codex chat, Ovim offers to enable live web search through
[Exa](https://dashboard.exa.ai/api-keys). Paste an Exa API key into the dialog,
or click the link to sign in and create one. The dialog is dismissible and can
always be reopened with `/exa`. Ovim stores the key beside `codex-auth.json` as
`ovim/exa-auth.json` in the platform config directory, using an atomic,
owner-only file on Unix. Set `EXA_API_KEY` to supply a key without storing it;
the environment value takes precedence.

The Ovim harness exposes `web_search` and `web_fetch` only while a usable key is
configured. A rejected or revoked stored key reopens the setup dialog. Credit
or budget exhaustion links to Exa's dashboard without discarding the key, and
temporary rate limits and server failures receive one bounded retry. Web tools
are read-only Ovim operations: they do not invoke a shell and do not require a
Codex sandbox or Terra approval.

Auto mode is the default. Read-only local inspection and tests run immediately;
context-dependent commands are reviewed by subscription-backed Terra at low
effort. Terra treats routine project-local formatting, building, linting, and
testing as authorized when they are reasonable steps toward your requested
implementation objective. Elevated privileges, credential access,
outside-project effects, remote-code pipelines, ambiguous authorization, and
classifier failures pause for you. Press Enter or Ctrl-Y to allow once, Ctrl-A
to allow the requested folder for the chat, or Esc/Ctrl-N to deny. To opt out
of auto mode, set
`tool_approval_mode = "sensitive_prompt"` or `"always_prompt"` in legacy
`ai.toml`.

For trusted work where approval interruptions are more costly than the safety
gate, click `YOLO OFF` at the top right of the chat to switch it to `YOLO ON`.
YOLO is opt-in per chat and defaults off. It bypasses Terra and interactive tool
approval prompts, and immediately releases a request already waiting for
approval. It does not disable malformed-input checks, `..` traversal rejection,
project-context requirements, or durable-run ownership checks. Click again or
run `/yolo off` to restore normal policy.

The docked chat width is adjustable: drag its left separator toward the editor
to make the chat wider, or toward the right to give the buffer more room. Ovim
keeps the chosen proportion for that chat as the terminal is resized.

When an active agent pauses for one of these approval decisions, Ovim emits the
terminal bell once. Whether that is audible, visual, or suppressed is controlled
by the terminal's bell settings. The notification is tied to the new prompt,
not to rendering, so an unattended prompt does not repeatedly ring.

Chat conversations are owned by Ovim. Each inference request replays the active
conversation branch, tool calls and results, and provider-encrypted reasoning
state. Forking or clearing a conversation therefore changes Ovim's branch
without depending on a hidden provider thread.

Agent turns have no tool-call ceiling by default; the lightning indicator in
the status line is a count, not a countdown. A profile may opt into a finite
guardrail with `max_tool_calls = 100`, in which case Ovim displays both the
current count and limit. Omitting the setting—or setting it to `0` in legacy
configuration—keeps long-running turns unlimited.

## Read-only delegated agents (preview)

Ovim can dispatch bounded explorer and reviewer children from an active AI
chat. The feature is disabled by default and is currently configured only in
legacy `ai.toml`. When enabled, the root model receives four controls:
`spawn_agent`, `list_agents`, `wait_agent`, and `interrupt_agent`. They appear
only while the editor owns an active durable root turn in a Git repository;
they are not ordinary profile tools and are never exposed to a child.

Every spawn must name both a catalog model and a reasoning effort. Ovim builds
those choices from configured profiles and rejects an unknown or unsupported
pair before allocating an agent, manifest, workspace, or lifecycle record.
The catalog ID is `profile/model`, for example `codex_sol/gpt-5.6-sol`.
`codex_app_server` profiles are not child routes because their nested provider
session cannot be safely reconstructed inside Ovim's child harness.

```toml
[subagents]
enabled = true
max_concurrent = 3
max_queued = 8
max_children_per_parent = 4
max_total_per_run = 8
max_depth = 1
default_timeout_seconds = 600
allow_writes = false
allow_network = false
allowed_models = ["codex_sol/gpt-5.6-sol", "codex_terra/gpt-5.6-terra"]
allowed_agent_kinds = ["explorer", "reviewer"]
allowed_reasoning_efforts = ["low", "medium"]

[subagents.budgets]
max_provider_events_per_agent = 256
max_tool_calls_per_agent = 48
max_total_provider_events = 1024
max_total_tool_calls = 160
max_estimated_cost = 5.0
```

An empty model or effort allowlist accepts every otherwise eligible catalog
choice; the live tool schema still advertises only exact supported pairs. The
preview rejects writes, network access, depth other than one, empty limits, and
duplicate allowlist entries. Changing subagent policy or provider profiles
while Ovim is running requires an editor restart instead of silently changing
authority beneath queued children.

Children see an immutable content-addressed snapshot captured at dispatch,
including authoritative unsaved editor buffers. Later edits in the root
worktree cannot change what an already-running child reads. A child receives
only bounded snapshot read, list, search, and unsaved-buffer tools—no shell,
network, navigation, mutation, approval, or further dispatch capability.

`spawn_agent` returns the durable task, agent, workspace, manifest, route, and
state immediately; the root should keep working while the child runs.
`list_agents` reports routing and lifecycle state without waiting. `wait_agent`
parks only that provider tool call, not the editor event loop, and completes on
a validated handoff, timeout, or new user steering. Delivered mailbox entries
are acknowledged durably after the wait result wins. `interrupt_agent`
interrupts the named child hierarchy while preserving partial run history.

The current preview does not attempt to resume an in-flight child provider
session after restarting Ovim. Existing child history remains intact, but the
resumed run fails closed for new delegated controls rather than guessing which
provider effects completed.

Use `vim.ai.setup(...)` in Lua to customize these defaults.

`ai.toml` still works, but it is legacy compatibility.

## Codex configuration

```lua
vim.ai.setup({
  default_profile = "codex_terra",
  contexts = {
    selection = "codex_terra",
    chat = "codex_sol",
    query = "codex_terra",
  },
  profiles = {
    codex_sol = {
      provider = "codex",
      model = "gpt-5.6-sol",
      reasoning_effort = "medium",
    },
    codex_terra = {
      provider = "codex",
      model = "gpt-5.6-terra",
      reasoning_effort = "low",
    },
  },
})
```

The `codex` provider does not accept an API key. To change accounts or repair
authentication, remove Ovim's `codex-auth.json`, then use `codex logout` and
`codex login` before opening Ovim again.

To retain the previous Codex-owned harness explicitly, configure
`provider = "codex_app_server"`. That strategy launches `codex app-server` and
keeps its native threads, sandbox, and orchestration. Ovim never falls back to
it silently when direct inference fails.

In AI chat, a single Escape hides the panel without interrupting the agent or
clearing the conversation—even while a review or approval is pending. You can
navigate and edit the project normally while the turn continues. Open the same
chat again to resume it with its input, queue, review, and view state intact.
While a hidden agent is running, a compact `AI working…` badge appears at the
top right of the editor; a paused approval uses an attention badge instead.
Press Ctrl-C with the chat open to stop the current generation without closing
or clearing the conversation; any partial response remains in history.

Persisted conversations are not restored automatically when starting a new
Ovim process. This avoids accidentally sending a large historical conversation
to a provider. Start Ovim with `--resume` only when you explicitly want to
restore the conversation associated with the file, repository, and chat name:

```sh
ovim --resume path/to/file.rs
```

Without `--resume`, opening AI chat creates a fresh durable conversation while
preserving the previous run on disk. Hiding and reopening chat within the same
Ovim process still keeps the live conversation as described above.

Chat slash commands are handled by ovim rather than sent to the provider:

Typing `/` or a partial command name opens an autocomplete popup. Use Up/Down
to choose, then Tab or Enter to insert the command; click selection is also
supported. Enter again executes a completed command.

- `/model` opens the profile picker.
- `/model codex_sol` switches directly to a named profile.
- `/clear` clears the current conversation and starts a fresh provider context.
- `/exa` opens web-search setup to add or replace an Exa API key.
- `/yolo`, `/yolo on`, and `/yolo off` toggle or set the per-chat approval
  bypass. This is also useful for headless sessions.

While an agent round is running, the composer remains editable:

- Shift-Enter inserts a newline without submitting. Ctrl-J provides the same
  behavior for terminals that encode modified Return as a legacy line feed.
- Enter queues a steer for the active round. Ovim delivers it after the next
  completed tool call. If the round finishes first, it becomes the next-round
  follow-up.
- Tab queues a message for the next round.
- Slash commands can also be queued; they run locally after the active round
  and are displayed as commands rather than user messages.

The composer wraps at word boundaries and keeps the cursor visible when input
grows beyond five rows. Click any visible composer row to place the cursor;
long words are split only when they cannot fit on a row by themselves.

Completed tool calls appear as compact summary rows in chat. Move focus into
message history, select a tool row, and press Enter to expand or collapse its
arguments and result.

Drag across text in message history to select it. Releasing the mouse copies
the selection to the system clipboard; `Ctrl-Y`, `y` while history is focused,
or `Cmd-C` also copies the active selection. Without a text selection,
`Ctrl-Y` keeps copying the complete conversation.

You can drag PNG, JPEG, GIF, or WebP files from the desktop into the chat
composer. Ovim displays attached filenames above the input; press Backspace on
an empty composer to remove the most recent image. Images submitted during an
active agent round are kept together and queued for the next round. Each image
may be up to 20 MiB, with a 40 MiB limit for the pending message.

When the terminal supports image rendering, pending attachments appear above
the composer. After submission, each thumbnail moves into the user message it
was sent with. Ovim emits terminal image data only while that message's complete
thumbnail is visible, so scrolling, new chat output, or hiding the chat cannot
leave a historical image pinned over the current screen. Click a visible
thumbnail to open its larger modal preview.

Terminal drag-and-drop is handled as a pasted image path, so the same behavior
is available headlessly with `ovim paste -s SESSION '/path/to/image.png'`.

## API-key providers

Codex is the default hosted path. The following sections apply only when you
deliberately configure a raw API provider.

### OpenAI Key Permissions (Restricted Key)

For ovim's OpenAI integration, a restricted key only needs:

- `Model capabilities`: write/request enabled
- `List models`: optional (`None` is fine)

Everything else can stay `None` for this use case.

### 1) Set API Key Securely (No `~/.zshrc`)

ovim reads API keys from environment variables at runtime. You can inject them only when launching ovim.

### Option A: macOS Keychain + launcher script

Store the key:

```bash
read -s OPENAI_TMP
echo
security add-generic-password -a "$USER" -s ovim-openai -U -w "$OPENAI_TMP"
unset OPENAI_TMP
```

Create `~/bin/ovim-openai`:

```zsh
#!/usr/bin/env zsh
export OPENAI_API_KEY="$(security find-generic-password -a "$USER" -s ovim-openai -w)"
exec ovim "$@"
```

Make it executable:

```bash
chmod +x ~/bin/ovim-openai
```

Use it:

```bash
ovim-openai
```

### Option B: one-shot session variable

```bash
read -s OPENAI_API_KEY
export OPENAI_API_KEY
ovim
```

This keeps the key out of shell startup files.

### 2) Configure an API provider with Lua

Put this in `~/.config/ovim/init.lua`:

```lua
vim.ai.setup({
  default_profile = "openai",
  contexts = {
    selection = "openai",
    chat = "openai",
    query = "openai",
  },
  profiles = {
    openai = {
      provider = "openai",
      model = "gpt-4.1-mini",
      api_key_env = "OPENAI_API_KEY",
      temperature = 0.2,
      max_tokens = 2048,
      edit_mode = "format",
      edit_format = "codeblock",
    },
  },
})
```

Built-in AI keybindings:

- Normal mode `Space Space`: chat
- Normal mode `Space ?`: read-only query
- Visual mode `Space Space`: single-shot selection edit

## Legacy `ai.toml` (Still Supported)

If you prefer TOML or need compatibility:

- macOS: `~/Library/Application Support/ovim/ai.toml`
- Linux: `~/.config/ovim/ai.toml`

```toml
default_profile = "openai"

[profiles.openai]
provider = "open_ai"
model = "gpt-4.1-mini"
api_key_env = "OPENAI_API_KEY"
temperature = 0.2
max_tokens = 2048
extraction = "json"
```

Important:

- Lua uses provider string `openai`
- `ai.toml` uses provider string `open_ai`

This naming difference is expected in the current parser.
