# Remote editing

Ovim's native GUI can drive an editor that runs on another machine over SSH:

```bash
ovim gui --remote user@host /path/to/project
```

The window is on your laptop. Everything else — the buffers, the language
servers, git, the test runner, the AI agent, the file tree — runs on the remote
host, against the remote toolchain and the real filesystem. Nothing is mirrored
or synchronised, because there is only one copy.

The trade is latency for fidelity: every keystroke crosses the network, and in
return there are no path translations, no partially working language servers,
and the session keeps running when you close the lid. Read
[What to expect from the link](#what-to-expect-from-the-link) before deciding
whether the trade is right for your connection.

## Getting started

```bash
# Open a project directory on another host
ovim gui --remote user@host /srv/checkout/api

# Or a single file
ovim gui --remote build-box /srv/checkout/api/src/main.rs

# Replace a session that has got stuck, instead of reattaching to it
ovim gui --remote user@host /srv/checkout/api --fresh
```

`--remote` takes `[user@]host`, or a `Host` alias from your `~/.ssh/config`. A
non-standard port, an identity file, a jump host or a proxy command belong in
that config file rather than on Ovim's command line — Ovim shells out to your
`ssh`, so whatever works for `ssh user@host` works here.

The path is resolved **on the remote host**, so give an absolute path, or one
starting with `~`. Quote the tilde so your local shell does not expand it
first:

```bash
ovim gui --remote user@host '~/projects/thing'
```

Both `ovim gui --remote …` and the standalone `ovim-gui --remote …` binary
accept the same flags.

## What runs where

| On the remote host | On your machine |
|---|---|
| Buffers, undo history, registers | The window and the rendering |
| Language servers, diagnostics, completion | Keyboard, mouse, IME |
| Git, blame, diff review | The system clipboard |
| Test runner, debugger | `strok` vector preview |
| AI chat, its tools and its credentials | "Open externally" and window actions |
| The file tree and every path you see | |

The GUI is a projection of the editor, not a second copy of it: the frontend
receives a viewport-sized frame of already-resolved text, highlight segments,
panels and layout, and sends back commands. Remote editing swaps the transport
under that conversation and changes nothing else, which is why every panel that
works locally works remotely.

That is also why the editor lives on the far side rather than the near one. A
local buffer cannot survive a disconnect; a remote session can, and does.

## Requirements on the remote host

- **Ovim itself.** Ovim does not upload a copy of itself. Install it there and
  check that a non-interactive shell can find it:

  ```bash
  ssh user@host command -v ovim
  ```

  If `ovim` is not on that `PATH`, the bootstrap also looks in
  `~/.cargo/bin`, `~/.local/bin`, `/usr/local/bin` and `/opt/homebrew/bin`.

- **A matching Ovim version.** The GUI protocol carries no version of its own,
  so a mismatched pair fails in ways that do not name the cause. A patch
  difference (`1.2.6` against `1.2.7`) warns and continues; a feature
  difference (`1.2.x` against `1.3.x`) refuses. `--allow-version-mismatch`
  overrides the refusal when you know the two builds agree.

- **Its own toolchain and language servers.** rust-analyzer, pyright, gopls and
  the rest have to be installed on the host with the code, not on the laptop.
  Ovim's auto-install prompt runs there too, so in practice they arrive the
  first time you open a file.

- **Its own AI credentials.** The agent runs on the remote host, so it signs in
  there. Ovim's device-code flow is designed for exactly this: press `D` in the
  sign-in dialog and open the URL on your laptop. See
  [Troubleshooting](troubleshooting.md#codex-sign-in-or-repeated-401-errors).

- **Nothing inbound.** The session binds loopback only, and Ovim forwards that
  port over the SSH connection it already has. No port is exposed on the remote
  host, and no firewall rule is needed.

## Sessions persist

The reason for putting the editor on the far side is that it stays there.
Closing the window leaves the session running with its warm language servers,
its undo history, its in-flight test run and its half-finished AI conversation
intact. Reopening reattaches to the same session:

```bash
ovim gui --remote user@host /srv/checkout/api   # starts a session
# close the window, close the laptop, get on a train
ovim gui --remote user@host /srv/checkout/api   # picks up where it left off
```

Reattaching is the default, and the session is found by the project path, so
the second command has to be the same as the first. Ovim derives the session
name from the resolved path as `gui-<basename>-<checksum>`, which is why it is
a normal session you can inspect on the remote host:

```bash
ssh user@host
ovim session list
ovim session health -s gui-api-1106417393
ovim session kill -s gui-api-1106417393
```

A session that is alive but wedged is never replaced automatically — that would
throw away exactly the state this design exists to keep. `--fresh` is the
deliberate way out: it stops the old session and starts a new one in its place.
Everything the old one held is gone with it.

## The connection indicator

While the link is healthy there is no indicator at all. When it drops, Ovim
reconnects on its own and says what it is doing.

- **Reconnecting.** Attempts start 500 ms apart and double up to a 15 s
  ceiling, with jitter, for up to ten minutes. The banner names the attempt,
  the wait until the next one, and what went wrong last time. "Retry now"
  skips the wait.
- **The remote session has ended.** Reconnection reattaches; it never quietly
  starts a replacement, because an empty session looks identical to a recovered
  one. Starting a new one is offered as a separate, labelled action, and only
  when Ovim owns the SSH link.
- **Refused.** Authentication or host-key failures stop immediately. Repeating
  them only repeats the refusal.
- **Still unreachable.** The ten-minute budget ran out. The session on the far
  side may well still be running; "Try again" is always available, because a
  dead end you can only leave by relaunching throws away the session it was
  protecting.

**Keys typed during an outage are dropped, not queued.** This looks harsh and
is the safe choice: in a modal editor a key means whatever the mode, pending
operator, count and cursor make it mean *when it arrives*, and the session
keeps being edited while your window cannot see it. A replayed `dd` would
delete the wrong line with nothing to show that it had. Ovim reports the
dropped input rather than storing it up.

## Predictive echo

Above roughly 40 ms of round-trip time, waiting for the network to draw each
character stops being a delay and starts being the interface. Ovim borrows
mosh's answer: over a remote link it draws the character locally and marks it
as unconfirmed with a **dotted underline** until the editor's own frame catches
up — usually within one round trip. If the editor disagrees, its frame wins and
the prediction disappears.

It is deliberately narrow, because showing text that is not there costs far
more than showing text late. Predictions are only made when:

- the buffer is in `INSERT` mode with no prompt, picker, completion menu or
  overlay open, and the buffer is not read-only;
- the key is a single Latin-range printable character with no modifier but
  Shift (CJK, emoji and anything composed of several code points round-trip);
- the cursor line is rendered as one unwrapped row with no horizontal scroll;
- the editor itself certifies that the next plain character will be inserted
  literally.

**Enter, Tab and Backspace are never predicted**, nor is anything in Normal,
Visual or Command mode, nor `}`/`)`/`]` on an otherwise blank line where
auto-indent would reposition them. Those keys cost a full round trip, and a run
of predicted characters pauses until the frame that accounts for them comes
back. In practice a clean line of prose or code predicts nearly all of its
keystrokes; a realistic edit full of `<CR>`, `<BS>` and `<Esc>` predicts
somewhat fewer.

There is nothing to configure. Predictive echo is on for remote links and off
for local ones, where it would trade a chance of being wrong for no gain.

## Clipboard

Ovim bridges the two machines' clipboards in both directions, following the
`clipboard` option rather than inventing a policy of its own (see
[Options](options.md#clipboard)).

- **Remote editor to your machine.** Whenever the remote editor writes its own
  system clipboard — a yank or delete under the default
  `clipboard=unnamedplus`, or an explicit `"+y` whatever the option says — the
  text arrives on your laptop's clipboard, ready to paste into a browser. On a
  headless remote there is usually no clipboard at all, so without the bridge
  the yank would go nowhere.
- **Your machine to the remote editor.** Gated on `clipboard` asking for system
  integration. Your clipboard is handed over when the Ovim window is
  activated — the one moment between "copied something in a browser" and
  "pressed `p`" that Ovim can see. `set clipboard=` turns this direction off
  entirely; the window's own paste gesture (`Cmd-V` on macOS, `Ctrl-Shift-V`
  elsewhere) is explicit and keeps working either way.

Both directions cap at 1 MiB. An oversized yank is refused loudly rather than
spending a minute of your uplink on text nobody is going to paste; it is still
sitting in the remote register, with `:w` and shell pipes as better ways to
move it.

Because the hand-over happens on window activation, a copy made on your laptop
*after* the Ovim window is already focused is not seen until you focus
something else and come back.

## What to expect from the link

Every keystroke round-trips. Be honest with yourself about the connection:

| Link | Round trip | Feel |
|---|---|---|
| LAN or a local VM | 1-10 ms | Indistinguishable from local |
| Same city, or a datacentre VPN | 10-25 ms | Fine |
| Cross-country | 40-70 ms | Mushy; holding `j` visibly lags |
| Transatlantic | 100-150 ms | Unpleasant |
| Cafe wifi or tethering | 100-300 ms with jitter | Not worth it |

Predictive echo makes insert-mode typing feel instant regardless, but it only
covers the keys listed above, and there is a second, separate ceiling it cannot
hide:

**Commands that change editor state are serialised — one per round trip.** They
have to be, because `d` arriving after its motion means something different
from `d` arriving before it. So sustained typing faster than 1/RTT builds a
backlog that drains in the pauses: roughly 25 keys per second at 40 ms, and
under 7 per second at 150 ms. Predictive echo hides this while it is
speculating, but the first unmodelled key — an Enter, a Backspace, an `Esc` —
ends the run and the catch-up becomes visible.

Bandwidth is not the constraint. Frames are viewport-sized and only sent on
change, which is a few kilobytes gzipped; splits multiply that, since each pane
carries its own lines.

Read-only queries are exempt from the serialisation. A diff review that walks
Git objects for several seconds does not block the keys you type while it runs.

## Current limits

Remote editing is complete enough for daily use, but these limits are real:

- **No interactive remote terminal.** `:terminal`, `:term` and `:shell` need a
  real terminal to attach to, and the GUI has none — see
  [Terminal sessions](terminal.md). Over a remote link they are refused on the
  status line with *"Interactive terminal sessions require the TUI frontend"*.
  Use a separate `ssh` window, or the AI chat's shell tool.

  `:!command` is **not** affected: it runs on the remote host, where the code
  is, and puts its output on the status line. `:!uname -n` reports the remote
  machine's name, not your laptop's.
- **`strok` vector preview runs locally.** The Vector tab shells out to `strok`
  on the machine running the window, so it needs `strok` on your laptop rather
  than on the remote host. See [AI setup](ai.md).
- **Dragging an image into a remote window does not work.** The drop hands over
  a path on your laptop, which the remote editor cannot read. Pasting an image
  from the clipboard does work, because that carries the bytes.
- **`:openwin` is not host-aware.** The path it produces is not tagged with
  which machine it belongs to, so over a remote link it is refused on the
  status line with *"Opening project windows requires the GUI frontend"* —
  the remote editor is headless and has no window to open. Launch the second
  window with its own `ovim gui --remote` instead.
- **One host per window.** There is no mixed local/remote or multi-root
  workspace; a window talks to exactly one editor.
- **The GUI protocol is unversioned**, which is why the version check above
  exists.

## Driving a session behind a tunnel you already have

When the tunnel is somebody else's — an existing `ssh -L`, a VPN, a container,
or a headless session running on this machine — the low-level pair takes over
from `--remote`:

```bash
# Copy the remote session's descriptor to this machine, then:
ovim gui --remote-session ./api.json --remote-endpoint 127.0.0.1:8443
```

`--remote-session` takes the JSON descriptor a headless run writes on its own
host (`<cache>/ovim/sessions/<name>.json`; see
[Headless & Automation](headless.md#session-files)). It carries both the bearer
capability and the port the session listens on, so neither is ever typed on a
command line where the process table and your shell history would keep it.
Treat that file as a credential.

`--remote-endpoint` says where to dial: `HOST:PORT`, or a bare port on
loopback. It defaults to the port in the descriptor, which is right when the
session is on this machine. Under `ssh -L` it is the local end of the tunnel,
while the descriptor still fixes the `Host` header the session insists on.

The two forms are mutually exclusive: `--remote` conflicts with
`--remote-session`, and `--fresh` and `--allow-version-mismatch` only mean
something with `--remote`, since there is no bootstrap to apply them to.

There is no equivalent to reattach-or-start here, and no way for Ovim to tell a
dead tunnel from a dead session, so a client on this path keeps retrying — which
is exactly what rides out a forward being restarted underneath it.

## Under the hood

`--remote` runs two SSH invocations and authenticates once. The first sends a
small POSIX shell script on stdin, which finds `ovim` on the remote host,
resolves the path, reattaches to or starts a headless session for it, and
prints back the session descriptor. The second forwards that session's loopback
port to a free port here. Both share an SSH control socket, so a host with 2FA
does not prompt twice.

The capability that authenticates the GUI to the session arrives inside the SSH
channel and goes straight into memory. **It is never written to disk on your
machine**, so there is no local credential file to protect or clean up.

Closing the window tears down the forward and lets the control master expire.
It does not touch the session — only an editor Ovim started in-process is
Ovim's to stop on exit.

The conversation itself runs over the session's ordinary authenticated API:
`POST /v1/gui/command` and `GET /v1/gui/stream`. See
[Headless & Automation](headless.md#gui-protocol-remote-editing) if you want to
drive it yourself.

## Troubleshooting

See [Troubleshooting](troubleshooting.md#remote-editing).
