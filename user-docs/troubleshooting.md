# Troubleshooting

## Sessions not found

Symptoms:

- `Session '<name>' not found`
- `ovim session list` shows no sessions

Checks:

- Ensure you started headless with `--headless --session <name>`.
- Verify the session dir:
  - macOS: `~/Library/Caches/ovim/sessions`
  - Linux: `~/.cache/ovim/sessions`
- If you set `OVIM_SESSION_DIR`, make sure your tooling uses the same location.

## LSP not working

First, check language detection/LSP configuration (no session required):

```bash
ovim lsp check path/to/file.ext
ovim lsp check path/to/file.ext --verbose
```

Then, for a running headless session:

```bash
ovim lsp status -s dev
ovim lsp wait -s dev --timeout 30000
```

Common causes:

- LSP server not installed / not on `PATH`
- Wrong project root (adjust `root_markers` in `languages.toml`)
- Large project indexing delay (wait for readiness)

## Codex sign-in or repeated 401 errors

Ovim signs in to ChatGPT independently from Codex CLI. Open AI chat with
`Space Space`, then press `Enter` in the Ovim sign-in dialog and complete the
browser flow. A visual selection attached with `Space Space` uses the same chat
and sign-in flow.

In an SSH session, `Enter` uses device-code sign-in instead: open the URL shown
by Ovim on your local computer and enter the one-time code. The remote machine
does not need a browser and you do not need to forward Ovim's localhost OAuth
port. Press `D` to select this flow explicitly, or `B` to use the localhost
browser callback when you have already arranged access to it. If an
organization disables device-code authorization, Ovim reports that policy and
keeps browser sign-in available.

Ovim refreshes expiring credentials automatically and retries one inference
request after an unexpected `401 Unauthorized`. If the refresh token is
rejected, the dialog asks you to sign in again while preserving the current
draft or unchanged selection.

To force an account change or replace damaged credentials, close Ovim, remove
`ovim/codex-auth.json` from the platform config directory, and reopen AI chat:

- macOS: `~/Library/Application Support/ovim/codex-auth.json`
- Linux: `~/.config/ovim/codex-auth.json`

Do not copy `~/.codex/auth.json` into Ovim. Sharing rotating refresh tokens with
Codex CLI can make either application lose authentication periodically.

## Remote editing

For what remote editing does and what it cannot do, see
[Remote editing](remote.md). Ovim's own error messages name the next step; this
is the same list with a little more room.

### `No 'ovim' was found on <host>`

Ovim does not upload itself. Install it on the remote host, then check that a
*non-interactive* SSH shell can see it — this is a different `PATH` from the one
your login shell uses:

```bash
ssh user@host command -v ovim
```

That must print a path. If it does not, put `ovim` in `~/.cargo/bin`,
`~/.local/bin`, `/usr/local/bin` or `/opt/homebrew/bin`, which the bootstrap
also searches, or export a `PATH` from `~/.ssh/environment` or your shell's
non-interactive rc file.

### `'<path>' does not exist on <host>`

The path after `--remote user@host` is resolved on the remote host, never on
yours. Give an absolute remote path; a leading `~` is expanded there, so quote
it (`'~/projects/thing'`) to stop your local shell expanding it first.

### `Ovim on <host> did not finish starting a session … in time`

The session did not write its descriptor within 30 seconds. Run the same thing
by hand to see what it is stuck on:

```bash
ssh user@host
ovim /path/to/project --headless --session probe
```

### SSH refuses the connection

Ovim shells out to your `ssh`, so anything that stops `ssh user@host` stops
Ovim too. Run it by hand first.

| Message | Remedy |
|---|---|
| `could not be resolved` | Check the spelling, or give the alias a `Host` entry in `~/.ssh/config`. |
| `did not pass host key verification` | Run `ssh user@host` once by hand, see the key, and decide. Ovim will not answer that for you. |
| `refused the SSH authentication` | `ssh-add` your key, or `ssh-copy-id user@host`. |
| `could not be reached over SSH` | Check the host is up. A non-standard port belongs in `~/.ssh/config`, not in `--remote`. |

### Version mismatch

The GUI protocol is not versioned, so a mismatched pair fails as a missing
field rather than as anything that names the cause. A patch difference warns and
continues; a feature difference refuses. Upgrade whichever side is older, or
pass `--allow-version-mismatch` when you know the two builds agree.

The same refusal appears as "did not report a version this Ovim could read"
when the binary found on the remote host is not Ovim at all.

### The session is wedged

Ovim never replaces a live session on its own — that would silently discard the
undo history and warm language servers the session exists to keep. Relaunch with
`--fresh` to stop it and start a replacement, accepting that loss:

```bash
ovim gui --remote user@host /path/to/project --fresh
```

To look before you leap, the session is an ordinary Ovim session on the remote
host, named `gui-<basename>-<checksum>`:

```bash
ssh user@host
ovim session list
ovim session health -s gui-project-1106417393
```

### "The remote session has ended"

Reconnecting reattaches; it never quietly starts a replacement, because an empty
session and a recovered one look identical. Whatever the old session held is
gone. The banner offers "Start a new session" as a separate, labelled action.

### Keystrokes go missing while the indicator is showing

Input during an outage is dropped rather than queued, on purpose: a replayed key
means whatever the mode and cursor make it mean when it *arrives*, against a
buffer that may have moved. Wait for the indicator to clear.

### Typing lags behind in bursts

Commands that change editor state are serialised, one per round trip, so
sustained typing faster than 1/RTT builds a backlog — about 25 keys per second
at 40 ms, under 7 at 150 ms. Predictive echo hides it while it can, but Enter,
Tab and Backspace end a predicted run and the catch-up becomes visible. Nothing
is lost; it drains in the pauses. See
[What to expect from the link](remote.md#what-to-expect-from-the-link).

## Logs & debug mode

If something “mysteriously” fails (or the UI gets corrupted), the first thing to grab is the log files.

Default locations:

- macOS: `~/Library/Caches/ovim/ovim.log` and `~/Library/Caches/ovim/lsp.log`
- Linux: `~/.cache/ovim/ovim.log` and `~/.cache/ovim/lsp.log`

Overrides:

- `XDG_CACHE_HOME` changes the base cache dir on most systems.

Useful env vars:

- `OVIM_DEBUG=1` enables extra app debug logging.
- `OVIM_LSP_DEBUG=1` enables verbose LSP debug logging (can be noisy).
