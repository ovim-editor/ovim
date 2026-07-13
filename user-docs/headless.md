# Headless & Automation

Headless mode is designed for tests, CI, and automation. It runs ovim without the TUI and exposes a local API.

## Start a Session

```bash
ovim path/to/file.rs --headless --session dev
```

## Inspect and Control Sessions

### Subcommands

```bash
ovim session list
ovim session health -s dev
ovim snapshot -s dev --format pretty
ovim send -s dev "iHello<Esc>"
ovim exec -s dev "w"
ovim session kill -s dev
```

AI chat uses the same background poller and input dispatcher in headless mode
as in the TUI. Open editable chat with `Space Space`, type a request, and submit
with Enter:

```bash
ovim send -s dev "  "
ovim send -s dev "inspect the project<Enter>"
```

If auto mode pauses a dynamic tool for approval, the Codex response remains
blocked until the decision arrives. Inspect it with `ovim snapshot -s dev`, then
send `<C-y>` (or `<Enter>`) to allow once, or `<C-n>` (or `<Esc>`) to deny. The
50 ms headless background tick also polls Luna classifier completions; no
renderer or attached terminal is required.

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

Use `ovim snapshot -s <name>` instead of calling the API directly unless you need custom tooling.
