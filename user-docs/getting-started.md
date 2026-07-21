# Getting Started

## Build and Run

From the repo root:

```bash
cargo build --release
./target/release/ovim path/to/file.txt
```

Open a project directory to start in the file explorer:

```bash
ovim path/to/project
```

## File Explorer

Press `-` from a file to reveal it in the project tree. Press `Tab` to move
focus back to the buffer while leaving the tree open, or `q` to close it.

| Key | Action |
|---|---|
| `j` / `k`, arrows | Move down / up |
| `Enter`, `o`, `l` | Open a file or expand a directory |
| `h` | Collapse a directory or select its parent |
| `a` | Create a file; end the name with `/` to create a directory |
| `R` / `d` | Rename / delete (delete requires confirmation) |
| `y` / `X` / `p` | Copy / cut / paste |
| `f` or `/` / `F` | Filter loaded entries / clear the filter |
| `H` / `I` | Toggle hidden / git-ignored entries |
| `r` | Refresh the tree |
| `gg` / `G` | Select first / last entry |
| `?` | Toggle the explorer key reference |
| `Tab` / `q` | Focus the buffer / close the explorer |

Create, rename, and paste operations refuse to overwrite existing paths.
Rename and create prompts also reject paths that escape the selected directory,
and the explorer root cannot be renamed, moved, or deleted.

## Headless Mode (for automation)

Headless mode runs ovim without the TUI and exposes a local REST API for driving the editor.

```bash
ovim path/to/file.rs --headless --session dev
```

Then, in another terminal:

```bash
ovim session list
ovim send -s dev "iHello<Esc>"
ovim snapshot -s dev --format pretty
ovim session kill -s dev
```

## Configuration (Quick)

- Lua config: `~/.config/ovim/init.lua`
- Language config override: `~/.config/ovim/languages.toml`

See `configuration.md` for details.
