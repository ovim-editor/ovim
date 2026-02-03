# Getting Started

## Build and Run

From the repo root:

```bash
cargo build --release
./target/release/ovim path/to/file.txt
```

## Headless Mode (for automation)

Headless mode runs ovim without the TUI and exposes a small REST API for driving the editor.

```bash
./target/release/ovim path/to/file.rs --headless --session dev
```

Then, in another terminal:

```bash
./ovim-ctl list
./ovim-ctl send dev "iHello<Esc>"
./ovim-ctl snapshot dev | jq '.buffer.content'
./ovim-ctl kill dev
```

## Common CLI Commands

ovim has built-in subcommands (alternative to `ovim-ctl`) that don’t require `jq`:

```bash
./target/release/ovim session list
./target/release/ovim send -s dev "iHello<Esc>"
./target/release/ovim snapshot -s dev --format pretty
./target/release/ovim session kill -s dev
```

## Configuration (Quick)

- Lua config: `~/.config/ovim/init.lua`
- Language config override: `~/.config/ovim/languages.toml`

See `configuration.md` for details.

