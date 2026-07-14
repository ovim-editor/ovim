# AI Setup

This guide covers practical AI configuration in ovim:

- Recommended setup: Codex with a ChatGPT/Codex subscription
- Preferred setup: Lua API in `init.lua`
- Secure API key handling without `~/.zshrc`
- Legacy `ai.toml` compatibility

## Recommendation

Install the Codex CLI and sign in with ChatGPT. ovim delegates authentication,
token refresh, and subscription usage to the supported Codex app-server:

```bash
npm install -g @openai/codex
codex login
codex login status
```

The built-in defaults use `gpt-5.6-sol` at medium effort for chat and
`gpt-5.6-terra` at low effort for selection edits and read-only queries. Codex
runs with its app-server sandbox read-only and approval policy set to `never`.
For editable chats, ovim exposes its own durable mutation and raw-shell dynamic
tools. Ovim records tool intent first, applies auto-mode policy, executes the
approved effect in the repository, and returns the result to Codex.

Auto mode is the default. Read-only local inspection and tests run immediately;
context-dependent commands are reviewed by subscription-backed Luna at low
effort. Elevated privileges, credential access, outside-project effects,
remote-code pipelines, ambiguous authorization, and classifier failures pause
for you. Press Enter or Ctrl-Y to allow once, Ctrl-A to allow the requested
folder for the chat, or Esc/Ctrl-N to deny. To opt out of auto mode, set
`tool_approval_mode = "sensitive_prompt"` or `"always_prompt"` in legacy
`ai.toml`.

Chat conversations retain a native Codex thread and reuse a shared app-server
process. After the first message, ovim sends only the new user turn plus current
editor context; Codex retains its prior turns and tool observations natively.
Changing the model, project boundary, project instructions, or available tools
starts a fresh Codex thread for that ovim conversation.

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
authentication, use `codex logout` and `codex login` directly.

In AI chat, Escape hides the panel without interrupting the agent or clearing
the conversation. Open the same chat again to resume it with its input and view
state intact.

Chat slash commands are handled by ovim rather than sent to the provider:

- `/model` opens the profile picker.
- `/model codex_sol` switches directly to a named profile.
- `/clear` clears the current conversation and starts a fresh provider context.

While an agent round is running, the composer remains editable:

- Enter queues a steer for the active round. Ovim delivers it after the next
  completed tool call. If the round finishes first, it becomes the next-round
  follow-up.
- Tab queues a message for the next round.
- Slash commands can also be queued; they run locally after the active round
  and are displayed as commands rather than user messages.

Completed tool calls appear as compact summary rows in chat. Move focus into
message history, select a tool row, and press Enter to expand or collapse its
arguments and result.

You can drag PNG, JPEG, GIF, or WebP files from the desktop into the chat
composer. Ovim displays attached filenames above the input; press Backspace on
an empty composer to remove the most recent image. Images submitted during an
active agent round are kept together and queued for the next round. Each image
may be up to 20 MiB, with a 40 MiB limit for the pending message.

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
