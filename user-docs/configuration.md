# Configuration

## Lua Config (`init.lua`)

Create `~/.config/ovim/init.lua`:

```lua
vim.opt.number = true
vim.opt.relativenumber = false
vim.opt.tabstop = 4
vim.opt.shiftwidth = 4
vim.opt.expandtab = true
```

Reload:

- `:ConfigReload` (ovim-specific)
- `:reload`

## Options (`:set`)

Options mirror Vim-style `:set` behavior.

Examples:

```vim
:set number
:set nonumber
:set scrolloff=10
:set wrap
:set nowrap
:set clipboard=
:set textwidth=80
```

See `options.md` for details.

## Language Configuration (`languages.toml`)

ovim ships with default language config, and you can override/extend it with:

`~/.config/ovim/languages.toml`

You can validate detection/LSP setup for a file without starting a session:

```bash
ovim lsp check path/to/file.rs
ovim lsp check path/to/file.rs --verbose
```

List configured languages:

```bash
ovim lsp languages
ovim lsp languages --verbose
```

See `LANGUAGE_SUPPORT.md` for examples.

## Session Directory Override (Advanced)

By default, session files are stored under your OS cache directory:

- macOS: `~/Library/Caches/ovim/sessions`
- Linux: `~/.cache/ovim/sessions`

You can override this location by setting:

`OVIM_SESSION_DIR=/path/to/dir`

This affects session file reads/writes and cleanup.

