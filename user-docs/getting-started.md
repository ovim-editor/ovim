# Getting Started

## Build and Run

From the repo root:

```bash
cargo build --release
./target/release/ovim path/to/file.txt
```

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

