# ovim

Oxidized Vim — a snappy, batteries-included editor with Vim keybindings, LSP support, a native GUI, and seamless headless mode.

## Quick Reference

```bash
# Build
cargo build --release

# Run editor (supports FILE:LINE:COL)
./target/release/ovim file.txt
./target/release/ovim src/main.rs:42:10
./target/release/ovim file.rs --headless --session dev

# Native GUI (Tauri + SolidJS)
./target/release/ovim gui file.rs
./target/release/ovim gui --remote user@host /path/to/project   # edit over SSH

# File operations (stateless — no session needed)
ovim edit src/main.rs --old "foo" --new "bar"
ovim insert src/main.rs --after 42 --text "new line"
ovim delete-lines src/main.rs --from 42 --to 45
ovim read-lines src/main.rs --from 40 --to 60

# Session control (always requires -s SESSION)
ovim send "ggK" -s dev
ovim exec "set number" -s dev
ovim context -s dev
ovim buffer -s dev

# LSP commands (grouped under `lsp`)
ovim lsp status -s dev
ovim lsp hover -s dev
ovim lsp check file.rs              # No session needed
ovim lsp languages --verbose        # No session needed

# Session management
ovim session list
ovim session kill -s dev
ovim session health -s dev
ovim session cleanup --dry-run
ovim session cleanup --max-age 7
```

**Sessions are opt-in.** TUI mode doesn't register a session. Headless mode requires `--session NAME`. A session must be explicit at process startup — `:session start NAME` is deliberately kept as a migration error so an ordinary TUI instance never exposes a latent mutation surface. `:session stop` and `:session list` do work.

## Architecture

```
ovim-core/                 # Shared library crate — the editor itself
├── src/
│   ├── editor/            # Core logic, operators, motions, AI, LSP actions
│   │   ├── input/         # Key event handling (normal/, insert_mode.rs, commands/)
│   │   │   └── normal/operators.rs  # d, c, y operator dispatch
│   │   ├── operators.rs   # The `Operator` enum (d, c, y, >, <, gu, ...)
│   │   ├── motions/       # Cursor movement (word.rs, paragraph.rs, ...)
│   │   ├── lsp_integration.rs # Editor-side LSP actions + intent dispatch
│   │   └── mod.rs         # Main editor state
│   ├── syntax/            # Tree-sitter grammars & highlighting
│   │   ├── languages.rs   # Language enum & detection
│   │   └── queries/       # Custom .scm highlight queries
│   ├── buffer/            # Rope-based text buffer (ropey)
│   ├── lsp/               # LSP client implementation
│   ├── session.rs         # Session descriptors (PID, port, capability)
│   └── ...
└── languages.toml         # Language configurations (embedded at compile time)

ovim/                      # Binary crate — frontends, CLI, API
├── src/
│   ├── api/               # REST API (Axum) — /v1/health, /v1/snapshot, ...
│   │   └── gui.rs         # /v1/gui/command + /v1/gui/stream (remote editing)
│   ├── gui/               # Native GUI backend (Tauri host side)
│   │   ├── mod.rs         # Snapshot projection + request handling
│   │   ├── protocol.rs    # Serializable GuiCommand / GuiSnapshot
│   │   ├── bridge.rs      # GuiBridge — the frontend's typed API
│   │   ├── remote.rs      # RemoteTransport (HTTP + SSE)
│   │   ├── reconnect.rs   # Connection state, backoff, reattach
│   │   ├── ssh.rs         # `--remote` bootstrap, tunnel, version check
│   │   └── clipboard.rs   # Clipboard bridging across the link
│   ├── frontend/          # Frontend-agnostic runtime plumbing (TUI/headless/GUI)
│   ├── ui/                # Terminal UI (ratatui + crossterm)
│   ├── bin/ovim-gui/      # Standalone GUI binary
│   ├── cli.rs             # CLI argument parsing (clap)
│   ├── subcommands.rs     # CLI subcommand handlers
│   ├── client.rs          # Blocking HTTP client for session-addressed commands
│   ├── event_loop.rs      # Event loops (TUI & headless)
│   ├── api_dispatch.rs    # handle_api_request() — API request → editor
│   └── main.rs            # Startup, session registration, signal handling
└── gui/                   # GUI frontend (SolidJS + Vite)
    └── src/               # App.tsx, stateProjection.ts, predictiveEcho.ts, ...
```

## Gotchas

- **tree-sitter version conflicts**: We use `tree-sitter = "0.25"`. Some grammar crates require older versions (0.19, 0.20, 0.24). Check docs.rs for compatibility before adding.
- **Workspace structure**: `ovim-core` contains shared logic (syntax, buffer, LSP types), `ovim` is the binary. Language/syntax code lives in ovim-core.
- **Highlight queries**: Some grammars export `HIGHLIGHTS_QUERY`, others `HIGHLIGHT_QUERY` (singular). Some export neither and need custom `.scm` files.
- **eprintln!()**: Breaks TUI rendering. Use only for headless debugging, then remove before committing.
- **Large files**: `ovim-core/src/editor/mod.rs` (~2.8k) and `ovim/src/gui/mod.rs` (~2.9k) are both at the refactor threshold. Split before adding more code there.
- **Multi-agent work**: If tests fail unexpectedly, another agent may be working on the codebase. Don't `git stash` their changes.

## Common Tasks

**Add new CLI subcommand:**
1. Add variant to `Command` enum (or `LspCommand`/`SessionCommand` for grouped commands) in `cli.rs`
2. Implement handler in `subcommands.rs`
3. For session-addressed commands: use `OvimClient` for HTTP requests
4. For file-addressed commands: use direct file I/O (no session needed)

**Add new REST API endpoint:**
1. Add variant to `ApiRequest` in `api/state.rs`
2. Add variant to `ApiResponse` in `api/state.rs`
3. Add handler in `api/handlers.rs`
4. Add route in `api/routes.rs`
5. Handle in `handle_api_request()` in `api_dispatch.rs`

**Add new MCP tool:**
1. Add tool definition in `api/mcp.rs::get_tools()`
2. Handle in `mcp_handler.rs::handle_tool_call()`
3. Map to existing `ApiRequest` or add new one

**Add new LSP feature:**
1. Add method to `LspManager` in `ovim-core/src/lsp/mod.rs`
2. Call from the `Editor` LSP action methods in `editor/lsp_integration.rs`
3. If it must run asynchronously, add a flag to `LspIntents` (`editor/lsp_state.rs`)
   and set it from the synchronous input path
4. Fire it from `dispatch_pending_intents()`, which every frontend's tick calls

**Add new operator:**
1. Add a variant to the `Operator` enum in `ovim-core/src/editor/operators.rs`
2. Handle it in `ovim-core/src/editor/input/normal/operators.rs` (key dispatch
   and motion/range application) and `input/normal/text_objects.rs`
3. Arm it from `input/normal/pending_commands.rs` if it takes a pending key
4. Add tests in `ovim/tests/`

**Add new GUI command (Tauri ↔ editor):**
1. Add a variant to `GuiCommand` (and a `GuiReply` shape) in `gui/protocol.rs`
2. Map it to/from `GuiRequest` in `gui/bridge.rs`, and add the typed helper
3. Handle it in `handle_request()` in `gui/mod.rs`
4. Add the `#[tauri::command]` wrapper in `gui/app.rs` and register it
5. It works remotely for free — but decide `must_keep_its_place()`: ordering is
   the default, and only read-only queries may be overtaken

**Add new language support:**
1. Check tree-sitter grammar crate compatibility with `tree-sitter = "0.25"` on docs.rs
2. Add grammar crate to `ovim-core/Cargo.toml`
3. Add variant to `Language` enum in `ovim-core/src/syntax/languages.rs`
4. Update these functions in `languages.rs`:
   - `detect_from_extension()` - file extension mappings
   - `get_tree_sitter_language()` - grammar binding (e.g., `tree_sitter_foo::LANGUAGE.into()`)
   - `get_highlight_query()` - query source (official constant or custom `.scm` file)
   - `get_lsp_language_id()` - LSP language identifier string
   - `from_info_string()` - markdown code fence support
5. Add language config block to `ovim-core/languages.toml`
6. If grammar doesn't export highlights query, create `ovim-core/src/syntax/queries/<lang>.scm`
7. Update `user-docs/LANGUAGE_SUPPORT.md`
8. Test with `ovim lsp check test.<ext> --verbose`

## Testing

```bash
cargo fmt      # Format code
cargo clippy   # Lints
cargo test     # All tests

# Test specific areas
cargo test syntax --lib              # Syntax highlighting tests
cargo test buffer --lib              # Buffer tests
cargo test -p ovim-core              # Core library tests only
cargo test -p ovim gui:: --lib       # GUI bridge, projection, protocol

# GUI frontend (SolidJS)
npm test --prefix ovim/gui           # Solid DOM suite
npm run check --prefix ovim/gui      # Type check
npm run build --prefix ovim/gui      # Must pass before the Rust GUI build

# Verify new language support
ovim lsp check test.sql --verbose    # Check language detection
ovim lsp languages                   # List all languages
```

**Vim-semantics tests must be derived from actual vim behavior, not from the implementation** — verify in `nvim --clean` before writing the assertion, and cite the reference in a comment. See [notes/TESTING_VIM_SEMANTICS.md](notes/TESTING_VIM_SEMANTICS.md).

## Language Support

Run `ovim lsp languages` to see all supported languages.

**Languages with LSP**: Rust, TypeScript, TSX, JavaScript, Astro, Python, Java, Kotlin, Scala, Groovy, SQL, C#, Terraform, Go, C, C++, Ruby, Bash, JSON, YAML, HTML, XML, CSS, TOML, Zig, Lua, Elixir, Ghostty

**Syntax highlighting only**: Markdown, Dockerfile, Tree-sitter Query, HCL, Diff, WGSL

The list above drifts; `ovim lsp languages` is authoritative.

See [user-docs/LANGUAGE_SUPPORT.md](user-docs/LANGUAGE_SUPPORT.md) for installation instructions.

## REST API & MCP

**HTTP Server**: Runs on `http://127.0.0.1:PORT` (random port, stored in session file)

| Endpoint | Method | Use Case |
|----------|--------|----------|
| `/v1/health` | GET | Health + LSP readiness |
| `/v1/lsp/status` | GET | Server states & pending requests |
| `/v1/snapshot` | GET | Complete editor state |
| `/v1/buffer` | GET/PUT | Buffer content |
| `/v1/keys` | POST | Send keystrokes |
| `/v1/command` | POST | Execute ex command |
| `/v1/mcp` | POST | MCP JSON-RPC 2.0 endpoint |
| `/v1/gui/command` | POST | One GUI command in, its reply out |
| `/v1/gui/stream` | GET | SSE stream of projected `GuiSnapshot` frames |

The same routes are also served unprefixed for backward compatibility, except
the GUI pair, which exists only under `/v1`.

For MCP protocol details, see [user-docs/MCP.md](user-docs/MCP.md).

## Key Implementation Details

### Session Management
- **Sessions are opt-in**: TUI mode doesn't register. Headless requires `--session NAME`.
- `SessionInfo` struct in `session.rs` with PID, port, file, LSP status
- Session files: macOS `~/Library/Caches/ovim/sessions/`, Linux `~/.cache/ovim/sessions/`

### LSP Integration
- `LspManager` coordinates multiple language servers
- `LanguageServer` handles individual server lifecycle
- Debounced `didChange` notifications (150ms) to reduce traffic
- Flush pending changes before hover/goto_definition to avoid stale data

### Remote Editing (`ovim gui --remote user@host /path`)
- The whole editor runs on the remote host; the local process is only the Tauri
  shell. Remote editing is a **transport swap** under `GuiBridge`, not a second
  editor — see `gui/remote.rs` and `gui/ssh.rs`.
- The session is reattached, not restarted, so warm LSP, undo history and
  in-flight AI survive a disconnect. `--fresh` is the deliberate replacement.
- The capability arrives on the bootstrap's stdout inside the SSH channel and
  never touches local disk.
- **Host-header trap**: `ApiSecurity::host_is_allowed` compares against the
  *server's own* port, which under `ssh -L` is not the port dialled. Any client
  must pin `Host` explicitly or every request 403s.
- State-changing commands are serialised one-per-round-trip
  (`GuiCommand::must_keep_its_place`); only read-only queries may be overtaken.
- Input is **dropped** while disconnected, never queued: a replayed key means
  something different against a buffer that moved.
- Design record: `planning/remote-editing/PLAN.md`. User docs:
  `user-docs/remote.md`.

### API Architecture
- Axum server on random port (port 0 → OS assigns)
- Tokio channels communicate with main event loop
- Thread-safe state mutations on main thread

## Debugging

```bash
# LSP logs (headless mode)
./target/release/ovim file.rs --headless 2>&1 | grep LSP-

# Check session files
cat ~/Library/Caches/ovim/sessions/test.json  # macOS
cat ~/.cache/ovim/sessions/test.json          # Linux

# Test endpoints
curl http://127.0.0.1:PORT/v1/health | jq '.'
```

## Best Practices

- **File size**: Keep files under 2k lines, refactor at 3k. Split large files before adding more code.
- **Debug logging**: Remove `eprintln!()` before committing - breaks TUI.
- **Commits**: Commit early and often when code is in a good state.
- **Multi-agent**: If your changes overlap with another agent's uncommitted work, don't commit. Let the user consolidate.
- **Tests failing unexpectedly**: Another agent may be working. Don't `git stash` to check - you'll lose their progress.

## Documentation

- **notes/**: Internal design docs, implementation status, architecture decisions
- **code-docs/**: Codebase structure documentation
- **user-docs/**: User-facing documentation (language support, MCP, etc.)

## User Instructions

Check `ISSUE_TRACKER.md` for current priorities. Only check off tasks when done and verified.
