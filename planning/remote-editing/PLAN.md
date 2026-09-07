# Remote editing over SSH

## Goal

Edit a project on a remote host from the local Ovim GUI, with LSP, git, grep,
tests, debugging, and AI all running remotely against the real toolchain.

## Architecture: remote backend, thin local frontend

The local `ovim-gui` process runs only the Tauri shell and the webview. The
full editor runs on the remote host. The GUI already works this way locally:
`GuiSnapshot` (`ovim/src/gui/mod.rs`) is a fully projected, viewport-sized
frame — resolved highlight segments, layout tree, panels, theme — and
`GuiBridge` is only `send command -> receive snapshot`. Remote editing is
therefore a *transport swap*, not a new editor.

### Why not the VS Code split-services model

VS Code Remote-SSH runs the editor locally and only the services remotely, so
typing never touches the network. That model would require defining an RPC
surface for every subsystem (file I/O, file tree, grep, git blame/diff/status,
LSP with local<->remote URI rewriting, DAP, test runner, AI tool path policy,
terminal) — roughly 5-10x this plan's work. It also forfeits the property that
makes the remote-backend model worth having:

**Session persistence.** Because the whole editor lives remotely, closing the
laptop and reconnecting preserves warm language servers, undo history, running
test runs, and in-flight AI conversations. Ovim already has the session
primitives for this (`ovim session list/health/kill/cleanup`, session files
carrying PID + port + capability). VS Code cannot offer this; a local buffer
cannot survive a disconnect.

A second structural win: there is **no URI translation**. The LSP client, the
language server, and the files are all on one host with one path space. The
warning in `ovim-core/src/buffer/file_io.rs` about URI stability stays
satisfied by construction.

### Why not an SFTP/VFS layer

Putting a `FileSystem` trait behind `buffer/file_io.rs` looks smaller but
silently disables LSP, git, test runner, DAP, and every AI file tool, across
~750 `fs::` call sites. `sshfs` already provides that outcome for free.

### The cost: interaction latency

Every keystroke round-trips. Rough budget:

| Link | RTT | Feel |
|---|---|---|
| LAN / local VM | 1-10ms | indistinguishable from local |
| Same city / DC VPN | 10-25ms | fine |
| Cross-country | 40-70ms | mushy; holding `j` visibly lags |
| Transatlantic | 100-150ms | unpleasant |
| Cafe wifi / tethering | 100-300ms + jitter | unusable |

Above ~40ms RTT, predictive local echo (chunk R7) is **required**, not polish.
The reference design is mosh — speculative echo with visual confirmation state
— not VS Code. In a modal editor only a narrow set of keys is safely
predictable (insert-mode printable characters, plain `hjkl` with the line
content already in the snapshot); anything with a pending operator, count,
register, macro, or auto-indent must round-trip.

Bandwidth is not a concern: snapshots are viewport-sized plus overscan, and
are only published on change. A 60-line viewport is roughly 40-50KB of JSON
uncompressed, ~5-8KB gzipped. Splits multiply it, since each `GuiPane` carries
its own `lines`.

## Verified constraints

- `pub mod gui` is **not** feature-gated in `ovim/src/lib.rs`; only
  `gui::app`, `gui::browser`, `gui::menu` are (`#[cfg(feature = "gui")]`).
  `GuiSnapshot`, `GuiRequest`, and `snapshot()` therefore already compile
  without Tauri, so the headless server can produce snapshots directly.
- `GuiRequest` has 31 variants, each embedding a `oneshot::Sender` reply
  channel. It is not serializable as written and must be split into a
  serializable command payload plus a transport-owned reply channel.
- `GuiSnapshot` and its ~20 nested types derive `Serialize` only.
- The API already authenticates: bearer `SessionCapability` plus a Host-header
  guard (`ovim/src/api/security.rs`).
- The API asserts a loopback bind (`ovim/src/api/mod.rs`). SSH forwarding
  satisfies this rather than violating it — no port is ever exposed.
- `ApiSecurity::host_is_allowed` requires the Host header to equal
  `127.0.0.1:{port}` or `localhost:{port}` for the **server's own** port. Under
  `ssh -L` the local port generally differs, so the client must set the Host
  header explicitly to the remote port. This is a real trap; see R4.
- `gui/mod.rs` is 3648 lines, past the 3k refactor threshold in `CLAUDE.md`.
  Extracting the protocol types is required cleanup, not incidental churn.
- `FileArg::parse` (`ovim/src/cli.rs`) splits on `:` from the right, so any
  URI-style target is mangled. Use a separate `--remote` flag, not a scheme.
- The GUI currently rejects terminal and shell sessions ("External shell
  sessions require the TUI frontend"). Remote work is exactly when a shell is
  wanted; tracked as a follow-up, not in scope here.

## Chunks

Each chunk is independently reviewable and leaves the tree green.

### R1 — Serializable GUI protocol
Extract `GuiSnapshot`, its nested types, and a new serializable `GuiCommand`
(the payload of `GuiRequest` minus reply channels) into `ovim/src/gui/protocol.rs`.
Add `Deserialize` alongside `Serialize`. Add serde round-trip tests. No
behavior change. Also relieves the `gui/mod.rs` size problem.

### R2 — `GuiTransport` abstraction
Introduce a transport trait (`send(GuiCommand) -> GuiReply`, `subscribe() ->
watch::Receiver<GuiSnapshot>`). Reimplement `GuiBridge` over it, with the
existing mpsc channel becoming `LocalTransport`. All 31 Tauri commands must
keep working untouched. Riskiest refactor — isolated deliberately.

### R3 — Server: GUI protocol over the headless API
`POST /v1/gui/command` and `GET /v1/gui/stream` (SSE). Drive a snapshot
producer from the headless event loop. Reuses the existing bearer + Host
security layer.

### R4 — Client: remote transport
`RemoteTransport` implementing `GuiTransport` over reqwest + SSE, with the
explicit Host header workaround and bearer auth from the remote session file.

### R5 — SSH bootstrap and `--remote`
`ovim gui --remote user@host --path /project`. ControlMaster/ControlPath so a
2FA host is not prompted twice per window; two-phase bootstrap (query the
remote endpoint, then forward it); remote binary discovery and version match.

### R6 — Reconnect and session persistence
Resume an existing remote session after a dropped link. The payoff feature.

### R7 — Predictive echo
Required for usable editing above ~40ms RTT. Mosh-style speculation with
confirmation state, limited to safely predictable keys.

### R8 — Clipboard bridging
A yank on the remote fills a remote register. Bridge it to the local system
clipboard, and the reverse for paste. Daily papercut if skipped.

### R9 — Documentation
`user-docs/remote.md`, plus README and CLAUDE.md updates.

## Out of scope

Multi-root or mixed local/remote workspaces; remote terminal (blocked on the
GUI terminal gap); local file drag-drop into a remote window.

## Remote-incompatible surface (audited)

Of 33 Tauri commands in `ovim/src/gui/app.rs`, 28 are pure passthrough to the
bridge and work over any transport unchanged. The exceptions:

1. **`gui_diff_state` / `gui_diff_open_file`** — both take the `PathBuf`
   returned by `GuiCommand::DiffWorkspace` and run `native_diff::review()` /
   `file_patch()` on it locally. The root problem is `DiffWorkspace` returning
   a path at all: it hands a server-side path to a client that assumes it is
   local. Fix in R3 by replacing it with commands that return the computed
   `DiffReview` / patch, so the work happens where the repository is.

2. **`GuiCommand::AttachImages { paths }`** — drag-drop yields paths on the
   laptop, but the handler opens them on the editor's host. `AttachImageData`
   (raw bytes) already exists and is transport-independent; route drag-drop
   through it under a remote transport.

3. **`PendingWindowOpen` / `:openwin`** — broken in *both* directions. A
   remote `:openwin` produces a remote path that is then `fs::metadata`'d
   locally and handed to a locally spawned `current_exe()`; and the local
   directory picker produces a local path handed to a remote editor. The
   window-open request must carry which host the path belongs to.

4. **`render_vector_preview`** — spawns `strok` from the *local* PATH. It
   works over a transport because it operates on the source string rather than
   a path, but it puts a toolchain requirement on the laptop, contradicting the
   "real toolchain runs remotely" premise. Needs an explicit decision, not an
   accident.

5. **Intentionally local, do not "fix"**: `gui_open_external`
   (`open::that_in_background`) and `gui_window_action`. These belong on the
   laptop.

## Implementation notes gathered during R1/R2

**R3 is cheaper than first estimated.** `run_headless_loop`
(`ovim/src/event_loop.rs`) already accepts `initial_dimensions`, calls
`handle_viewport_resize`, and runs `process_editor_tick` +
`process_pending_rehighlight` on a 50ms interval — the same tick work the GUI's
`run_editor` performs. R3 is therefore a snapshot publisher plus two routes,
not a rewrite of the headless loop. The existing `/resize` endpoint already
lets a client set its viewport on connect, so no new server work is needed for
sizing.

**The Host-header trap, with its mechanism.** `OvimClient`
(`ovim/src/client.rs`) builds `base_url` as `http://127.0.0.1:{port}`, so
reqwest derives the `Host` header from the URL. Under `ssh -L` the local
forwarded port differs from the remote port, and
`ApiSecurity::host_is_allowed` compares against the *server's own* port — so
the auto-derived header fails and every request 403s. The remote transport
must set `Host` explicitly to the remote port.

**`OvimClient` cannot be reused as-is.** It is blocking reqwest
(`pub fn`, `.send()` without `.await`). `RemoteTransport` needs an async
client, so R4 writes its own rather than extending `OvimClient`.

**R3 landed; notes for R4.**

- The routes are `POST /v1/gui/command` (a `GuiCommand` in, a `GuiReply` out,
  `204 No Content` for a fire-and-forget command) and `GET /v1/gui/stream`
  (SSE, `event: snapshot`, 15s keep-alive). They live only under `/v1`, not on
  the deprecated unversioned paths. A command that failed *in the editor*
  still answers `200` with an `Err` reply, exactly as `LocalTransport` does,
  so `RemoteTransport` can be a thin byte-for-byte shim.
- Snapshot production is lazy on `watch::Sender::receiver_count()`. A session
  with no stream subscriber never projects a `GuiSnapshot` at all. Because
  `Sender::subscribe` marks the current value as already seen, the frame on
  record is cleared when the last subscriber leaves; a new subscriber
  therefore never reads a stale frame, and the publisher, seeing nothing on
  record, projects a fresh one on its next 50ms tick.
- `GuiCommand::Shutdown` stops the headless session. R4 must not wire it to
  the local window closing -- that would defeat the session persistence R6 is
  built on. Use it only for an explicit "quit the remote editor".
- `GuiCommand::DiffWorkspace` is gone. `DiffReview { spec }` and
  `DiffFilePatch { spec, path }` replace it and return the computed values, so
  `GuiReply::Path` is gone too and the command count is 32, not 31. The two
  Tauri diff commands are now pure passthroughs; the frontend was unchanged.

**R4 landed; notes for R5.**

- `RemoteTransport` (`ovim/src/gui/remote.rs`) owns its own two-worker Tokio
  runtime. Every request and the snapshot stream run on it, so a `send` future
  can be polled by whatever runtime Tauri hands it and the synchronous
  `send_oneway` has a reactor to reach even though it runs from a plain thread.
- `RemoteEndpoint` keeps the address dialled and the session's own port as two
  separate fields and always sets `Host` from the latter, pinned in the
  client's default headers. `RemoteEndpoint::from_session` takes both, plus the
  capability, from the session descriptor, so the only thing R5 has to supply
  is where to dial.
- The launch surface is `ovim gui --remote-session <descriptor>
  [--remote-endpoint HOST:PORT]`, also accepted by the `ovim-gui` binary. The
  capability is never an argument -- it travels in the descriptor. R5 replaces
  this pair with `--remote user@host`, which will fetch the descriptor over SSH
  and forward the port itself; the transport underneath needs no change.
- The stream is decoded by a small hand-written SSE decoder rather than a
  dependency: both ends of the stream are ours and the format in use is three
  field names and a blank line.
- A stream that dies annotates the last frame's status line with
  `remote::CONNECTION_LOST` and stops. That is deliberately a placeholder for
  R6: the watch channel can only carry snapshots, so there is nowhere else for
  a connection state to live.
- Verified live on one machine, including through a forwarded port: a GUI
  window driven by a headless session, and the session outliving the window.
  `ovim/tests/remote_session_test.rs` is the ignored end-to-end test that does
  it, driven by `OVIM_REMOTE_SESSION` and `OVIM_REMOTE_ENDPOINT`.

**Test harness for R4 already exists.** R2 added `RecordingTransport` and
`every_typed_helper_sends_the_command_variant_it_is_named_for` in
`gui/bridge.rs`, which pins every helper to its command variant. R4
should reuse that harness to prove the remote transport is wire-equivalent to
the local one.

**R5 landed; notes for R6.**

- The launch surface is `ovim gui --remote user@host /path/to/project`, with
  `--fresh` and `--allow-version-mismatch` beside it, on both the `ovim` and
  `ovim-gui` binaries. `--remote-session` / `--remote-endpoint` stay as the
  low-level pair: they are the only way in when the tunnel is somebody else's
  (an existing forward, a VPN, a container, a headless session on this
  machine), and they are what `remote_session_test.rs` drives. clap makes them
  conflict with `--remote`, since both answer "where is the editor".
- Everything lives in `ovim/src/gui/ssh.rs`. `RemoteOptions::resolve` is the
  single place the flag combinations mean anything, so the clap parser and
  `ovim-gui`'s hand-rolled one cannot drift apart.
- **The capability is never written to disk locally.** It arrives on the
  bootstrap's stdout, inside the SSH channel, and goes straight into
  `RemoteEndpoint`. There is no local descriptor file to secure or clean up.
- **Two invocations, one authentication.** The bootstrap runs with
  `ControlMaster=auto` + `ControlPersist=60` and leaves a master behind; the
  forward runs with `ControlMaster=no` over it. The control socket is per
  launch, under `$XDG_RUNTIME_DIR/ovim-ssh/<8 hex>` (0700), chosen from a list
  of bases and rejected unless `len + 9 <= 104` -- `ssh` binds `path.XXXXXXXX`
  before renaming, and overrunning `sun_path` fails without naming the cause.
  A shared per-host socket would allow zero authentications for a second
  window, but one window's teardown would then cut another's connection, and a
  socket left by a killed process makes `ssh` silently disable multiplexing --
  which looks exactly like the double prompt being avoided.
- **The bootstrap script travels on stdin** (`ssh <dest> /bin/sh -s`), not as
  an argument: `ssh host <script>` is interpreted by the *login* shell, which
  may be fish or csh.
- **Reattach beats start.** The session name is derived on the remote host from
  the resolved path: `gui-<basename>-<cksum>`. A descriptor whose PID is alive
  is reused; a stale one is deleted and replaced. A session that is alive but
  wedged is *not* killed automatically -- that would throw away exactly the
  state this architecture exists to keep -- so it fails at the transport
  handshake and `--fresh` is the documented way out.
- **Version policy: patch drift warns, feature drift refuses**, with
  `--allow-version-mismatch` as the override. The GUI protocol is unversioned,
  so a mismatch fails as a missing field rather than as anything that names the
  cause; but a source build on the laptop against a released host is the normal
  case, and refusing on every patch difference would make the feature unusable.
- **Teardown does not touch the session.** `SshTunnel::drop` kills the forward
  and asks the master to exit. The forward's remote command is `cat` rather
  than `-N`, so it holds this process's stdin pipe: a GUI killed outright still
  closes the pipe, `cat` sees end of file, and the tunnel takes itself down.
  `ControlPersist` then reaps the master, and `ssh` unlinks its own socket.
- The forwarded port is chosen by binding `127.0.0.1:0` and reading back what
  the kernel gave. `ExitOnForwardFailure=yes` turns the remaining race into a
  clean non-zero exit, and `SshTunnel::open` retries three times.
- Not verified against a real remote host: this machine's sshd accepts only
  keys that are not present, and forcing it would have meant editing
  `authorized_keys`. What *was* verified live: the bootstrap script against the
  real `ovim` binary (start, then reattach, then `--fresh` replacing the
  process), the resulting session driven end to end by `remote_session_test`,
  and the authentication-rejected path against the real sshd on this machine.

**R6 landed; notes for R7.**

- **Connection state has its own channel, not a field on `GuiSnapshot`.**
  `GuiTransport::connection() -> watch::Receiver<GuiConnection>` beside
  `subscribe()`, bridged to the webview by `gui_connection` beside
  `gui_subscribe`. A snapshot only arrives while the link works, so a field on
  it could not change at the one moment it has something to say. The default
  trait implementation hands back a receiver whose sender is already dropped,
  so `LocalTransport` reads `Connected` once and is never heard from again.
- **The frontend does assume monotonic revisions.** `shouldAcceptRevision`
  (`gui/src/stateProjection.ts`) drops any frame below the newest seen. Within
  one session that never fires, because `GuiServer`'s counter only rises. Two
  things could make it fire: R4's placeholder annotated the last frame at
  `revision + 1`, which could push the client's floor past the session's own
  counter and silently eat the first genuine frame after a reconnect (removed);
  and a session that was *replaced* starts counting from one again. `Link::publish`
  therefore keeps a floor and raises any frame that would go backwards. Nothing
  is rewritten while the far side is the monotonic one.
- **Input during an outage is dropped, never queued.** `RemoteTransport::send`
  refuses while disconnected instead of letting requests pile up behind a
  connect timeout. In a modal editor a key means whatever the mode, pending
  operator, count and cursor make it mean when it *arrives*, and the session
  keeps being edited while the client cannot see it -- so a replayed `dd` can
  delete the wrong line with nothing to show that it did. This is load-bearing
  for R7: predictive echo may only ever speculate *locally*, and must discard
  its speculation rather than send it when the link is down.
- **Backoff**: 500ms doubling to a 15s ceiling, ±25% jitter, 10-minute budget
  (~44 attempts). Running out is `Lost { GaveUp }`, which is a state the user
  can leave -- every terminal state offers a manual retry, because a dead end
  that can only be left by relaunching throws away the session it was
  protecting.
- **Four failure classes, told apart at two places.** `Failure::from_status`
  reads the session's own answer (`401`/`403` -> authentication, `503` -> the
  editor stopped, `404` -> a protocol this Ovim does not serve, 5xx ->
  retry); `bootstrap_fault` reads what `ssh` said, sharing its phrase list with
  the message the user reads so the two cannot drift. A client using
  `--remote-session` cannot tell a dead forward from a dead session -- there is
  no bootstrap to ask -- so it keeps retrying, which is what rides out a
  `socat` or `ssh -L` being restarted underneath it.
- **Reconnect reattaches, and the remote script enforces it.**
  `LaunchMode::ReattachOnly` exits `EXIT_SESSION_GONE` rather than starting a
  replacement. A replacement would come up empty and look identical, which is
  the worst outcome available. Starting one is a separate, labelled action in
  the banner, and only offered when this process owns the SSH link.
- `RemoteLaunch` now carries an `Arc<dyn RemoteLink>` instead of an
  `Option<SshTunnel>`, and the transport owns it -- so `SshLink` can drop the
  old tunnel and build a new forward (on a new local port, with a new
  `RemoteEndpoint` and a new pinned Host header) without reaching back into
  `app.rs`.
- Verified live on one machine: a headless session driven through a `socat`
  forward, the forward's process group killed mid-stream, the session edited by
  a second client while the first was blind, then the forward restarted. The
  client reported `Reconnecting { attempt: 1, retry_in_ms: 566, detail: "the
  stream failed: the connection dropped" }`, refused a keystroke during the
  outage, came back `Connected`, and rendered the edit it had never seen, at a
  higher revision. `remote_session_test::a_forward_that_dies_and_comes_back_leaves_the_session_untouched`
  is that drill, ignored by default.
- Not verified live: the SSH-owned reconnect paths (re-forwarding, and
  `EXIT_SESSION_GONE`), for the same reason R5 could not verify its own --
  no reachable sshd here. The bootstrap script itself is exercised against a
  real `/bin/sh` in `gui::ssh`'s tests.
