# Headless & Automation

Headless mode is designed for tests, CI, and automation. It runs ovim without the TUI and exposes a local API.

## Start a Session

```bash
ovim path/to/file.rs --headless --session dev
```

## Inspect and Control Sessions

### Built-in `ovim` subcommands

```bash
ovim session list
ovim session health -s dev
ovim snapshot -s dev --format pretty
ovim send -s dev "iHello<Esc>"
ovim exec -s dev "w"
ovim session kill -s dev
```

LSP helpers:

```bash
ovim lsp wait -s dev --timeout 30000
ovim lsp status -s dev
ovim lsp hover -s dev
```

### `ovim-ctl` helper script

`ovim-ctl` is a small helper that shells out to `curl` and (optionally) `jq`.

```bash
./ovim-ctl list
./ovim-ctl wait dev 60
./ovim-ctl send dev "iHello<Esc>"
./ovim-ctl snapshot dev | jq '.cursor'
./ovim-ctl kill dev
```

## Session Files

Session files are JSON and live in:

- macOS: `~/Library/Caches/ovim/sessions`
- Linux: `~/.cache/ovim/sessions`

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

- `GET /health`
- `GET /snapshot`
- `POST /keys`
- `POST /command`

Use `ovim snapshot -s <name>` (or `ovim-ctl snapshot`) instead of calling the API directly unless you need custom tooling.
