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

- **A matching Ovim version.** Two checks apply:
  - **GUI protocol version.** On every connect and every reconnect the window
    asks the session which GUI protocol it speaks, before anything else crosses
    the link. If the two numbers differ, or the session is too old to answer,
    the window refuses with both versions in the message. Nothing overrides
    this, because frames from another protocol would be misread.
  - **Release version (`--remote` only).** The bootstrap also compares the
    release of the `ovim` binary it finds on the remote host with yours. A
    patch difference (`1.2.6` against `1.2.7`) warns and continues; a feature
    difference (`1.2.x` against `1.3.x`) refuses unless you pass
    `--allow-version-mismatch`. This checks the binary on `PATH`, not the
    running session, so on reattach the protocol check above is what catches a
    session started by an older build.

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

`--fresh` stops the old session only after the remote `ovim` confirms that the
process ID in its descriptor still belongs to that session: the same process
start time and an Ovim executable (`ovim session kill --verified`). If the
process ID now belongs to some other process, nothing is signalled and a new
session starts. If it cannot be confirmed either way, `--fresh` refuses and
kills nothing, and the message says why.

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

Ovim bridges the two machines' clipboards. Both directions are decided by the
window on your machine, from what you do in that window. Nothing the remote
side sends can turn either direction on, and the remote `clipboard` option
plays no part in it.

- **Your machine to the remote editor: only when you paste.** Your clipboard
  is sent only as part of a paste you type in the Ovim window:
  - a put from the clipboard registers in Normal or Visual mode: `"+p`, `"*p`,
    `"+P`, `"+gp`, `"+]p` and the like, with or without a count;
  - `Ctrl-R +` or `Ctrl-R *` in Insert mode or on the command line;
  - `:put +` or `:put *` typed on the command line;
  - the window's paste key (`Cmd-V` on macOS, `Ctrl-Shift-V` elsewhere) and
    **Edit → Paste**, which carry the text themselves.

  It is sent immediately ahead of that key, in the same ordered stream, so the
  paste always reads what you copied. Nothing is sent when the window gains
  focus, and nothing is sent for any other key.

  A plain `p` does **not** send your clipboard, even under the default
  `clipboard=unnamedplus`. It pastes the remote session's own unnamed
  register, which holds the last thing yanked there or pasted from your
  machine. Use `"+p` or the paste key to paste what you copied locally.

  Which keys count is decided from the mode in the latest frame. If you type
  `Ctrl-R +` faster than one round trip after entering Insert mode, the window
  may not know yet that you are in Insert mode, and the paste uses the
  session's register instead.

- **Remote editor to your machine: only yanks made in this window.** When a
  command you sent from this window makes the remote editor write its `+`
  register (a yank or delete under `clipboard=unnamedplus`, or an explicit
  `"+y`), the text is copied to your laptop's clipboard. The session records
  which window's command made each write, and the window copies only its own,
  and only if it has sent a command since the last one it copied. Connecting
  and reconnecting never write your clipboard: whatever the session yanked
  before the window attached stays on the session.

A paste that carries your clipboard costs one extra command in the ordered
lane, so it takes one more round trip than a paste of the session's register:
PASTE_LATENCY.

Both directions cap at 1 MiB, checked on both sides. An oversized yank is not
copied and an oversized local clipboard is not sent; the refusal goes to the
log rather than to the status line. The key itself is still delivered, so an
oversized paste uses the session's register.

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

Latency is what you feel, but bandwidth is not free either. Frames are
viewport-sized and only sent on change — and each change sends a whole frame,
uncompressed: roughly 20 KB for an 80×24 view of highlighted code and 45 KB for
120×35. Splits multiply that, since each pane carries its own lines. Nothing on
the path compresses it, so on a link under a couple of Mbit/s the frames matter
as much as the round trips do.

A few read-only queries (the workspace directory, the vector source, the
clipboard text) are exempt from the serialisation, because where they land in
the sequence cannot change what any other command does. Opening or refreshing
a diff review is not one of them: it changes what the editor shows, so it takes
its place in line, and the editor computes it on the remote host.

## Current limits

Remote editing is complete enough for daily use, but these limits are real:

- **No interactive remote terminal.** `:terminal`, `:term` and `:shell` need a
  terminal to attach to, and the remote session is headless. Typed over a
  remote link they are refused on the status line ("`:terminal` needs a
  terminal; a headless session cannot run it"). Use a separate `ssh` window,
  or the AI chat's shell tool.

  `:!command` typed at the command line is refused the same way, because the
  interactive path hands it to a terminal-owning frontend. It does work through
  the session's API, where it runs on the remote host, in the project
  directory, and reports its output. From that host:

  ```bash
  ovim exec '!uname -n' -s gui-api-1106417393   # the remote machine's name
  ```
- **`strok` vector preview runs locally.** The Vector tab shells out to `strok`
  on the machine running the window, so it needs `strok` on your laptop rather
  than on the remote host. See [AI setup](ai.md).
- **The terminal panel is local-only.** It runs a shell on the machine with the
  window, so over a remote link it refuses to open rather than start a shell
  in a directory that only exists on the other host.
- **Dropped images are read on your laptop.** The window reads the file and
  sends its bytes (PNG, JPEG, GIF or WebP, up to 20 MiB), because the remote
  editor cannot read a path on this machine.
- **`:openwin` is not host-aware.** The path it produces is not tagged with
  which machine it belongs to, and the remote editor is headless and has no
  window to open, so over a remote link it is refused on the status line
  ("Opening project windows requires the GUI frontend"). **File → New Window**
  refuses over a remote link for the same reason. Launch the second window with
  its own `ovim gui --remote` instead.
- **One host per window.** There is no mixed local/remote or multi-root
  workspace; a window talks to exactly one editor.

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
