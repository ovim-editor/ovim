# AI Setup

This guide covers practical AI configuration in ovim:

- Preferred setup: Lua API in `init.lua`
- Secure API key handling without `~/.zshrc`
- Legacy `ai.toml` compatibility

## Recommendation

Use `vim.ai.setup(...)` in Lua as your primary config.

`ai.toml` still works, but it is legacy compatibility.

## OpenAI Key Permissions (Restricted Key)

For ovim's OpenAI integration, a restricted key only needs:

- `Model capabilities`: write/request enabled
- `List models`: optional (`None` is fine)

Everything else can stay `None` for this use case.

## 1) Set API Key Securely (No `~/.zshrc`)

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

## 2) Configure AI with Lua (Preferred)

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
- Visual mode `Space`: single-shot selection edit

## 3) Legacy `ai.toml` (Still Supported)

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
