# ovim

A Neovim clone in Rust with LSP support and seamless headless mode.

## Quick Reference

```bash
# Build
cargo build --release

# Run
./target/release/ovim file.txt

# Headless with session
./target/release/ovim file.rs --headless --session dev

# Session control (no port needed!)
./ovim-ctl list                    # Show all sessions with LSP status
./ovim-ctl send dev "ggK"          # Send commands
./ovim-ctl health dev              # Health check
./ovim-ctl lsp dev                 # LSP server status
./ovim-ctl wait dev 30             # Wait for LSP ready
./ovim-ctl kill dev                # Kill & cleanup
```

## Architecture

```
src/
├── api/           # REST API (Axum) - /health, /lsp/status, /snapshot, etc.
├── buffer/        # Rope-based text buffer (ropey)
├── editor/        # Core logic, operators, motions, LSP actions
│   ├── input.rs   # Key event handling
│   ├── operators.rs  # d, c, y operators
│   ├── motions.rs    # Cursor movement
│   └── mod.rs        # Main editor state + LSP integration
├── lsp/           # Language Server Protocol client
│   ├── mod.rs     # LspManager (coordinator)
│   ├── server.rs  # LanguageServer (individual server)
│   └── logger.rs  # Request/response logging
├── session/       # Session persistence & discovery
├── ui/            # Terminal UI (ratatui + crossterm)
└── main.rs        # Event loops (TUI & headless)
```

## Development Workflow

### Headless Mode (Recommended)

1. **Start session**: `ovim --headless --session myproject src/main.rs`
2. **Control it**: `./ovim-ctl send myproject "ggK"` (no port needed!)
3. **Check LSP**: `./ovim-ctl lsp myproject`
4. **Kill it**: `./ovim-ctl kill myproject` (auto-cleanup)

Sessions stored in:
- macOS: `~/Library/Caches/ovim/sessions/`
- Linux: `~/.cache/ovim/sessions/`

### LSP Features

**Zero-config Java**:
```bash
ovim MyClass.java  # Auto-downloads jdtls, detects Java version, full IDE
```

**Rust, Python, JavaScript** also supported (rust-analyzer, pyright, typescript-language-server).

**LSP Logging** (all requests/responses logged to stderr):
```
[LSP-REQUEST] Method: textDocument/hover | Request ID: Number(2) | Server: rust
[LSP-RESPONSE] Success: textDocument/hover | Took: 751.5µs
```

**LSP Introspection**:
```bash
curl http://127.0.0.1:PORT/lsp/status  # Server states, pending requests
curl http://127.0.0.1:PORT/health      # LSP readiness check
```

### REST API Endpoints

| Endpoint | Method | Use Case |
|----------|--------|----------|
| `/health` | GET | Health + LSP readiness |
| `/lsp/status` | GET | Server states & pending requests |
| `/snapshot` | GET | Complete editor state (buffer, cursor, mode, registers, marks) |
| `/buffer` | GET/PUT | Buffer content |
| `/cursor` | GET | Cursor position |
| `/mode` | GET | Current mode |
| `/keys` | POST | Send keystrokes (e.g., `{"keys": "ggK"}`) |
| `/command` | POST | Execute ex command (e.g., `{"command": "w"}`) |
| `/render` | GET | ANSI rendering |

### Testing

```bash
# Unit tests
cargo test

# Headless integration tests
./target/release/ovim test.txt --headless --session test &
./ovim-ctl send test "iHello World<Esc>"
./ovim-ctl snapshot test | jq '.buffer.content'
./ovim-ctl kill test
```

### Lua Configuration

Create `~/.config/ovim/init.lua`:
```lua
vim.opt.number = true
vim.opt.relativenumber = true
vim.opt.tabstop = 4
vim.opt.shiftwidth = 4

-- Reload with :ConfigReload or :reload
```

## Key Implementation Details

### Session Management
- `SessionInfo` struct in `session.rs` with PID, port, file, LSP status
- Written on startup, auto-deleted on exit (signal handlers)
- `ovim-ctl` auto-discovers ports from session files

### LSP Integration
- `LspManager` coordinates multiple language servers
- `LanguageServer` handles individual server lifecycle
- Full request/response logging with timing
- Non-blocking with `try_lock()` to avoid blocking background tasks
- Debounced `didChange` notifications (150ms) to reduce traffic

### API Architecture
- Axum server on random port (port 0 → OS assigns)
- Tokio channels communicate with main event loop
- Thread-safe state mutations on main thread
- Port sent back via oneshot channel for session file

### Headless Event Loop
- Processes API requests, LSP notifications, LSP actions
- 50ms timeout for API requests (non-blocking)
- 10ms sleep to avoid busy loop
- Graceful shutdown with session cleanup

## Code Style

```bash
cargo fmt      # Format code
cargo clippy   # Lints
cargo test     # Tests
```

## Debugging

```bash
# LSP logs are in stderr (when --headless)
./target/release/ovim file.rs --headless 2>&1 | grep LSP-

# Check session files
cat ~/Library/Caches/ovim/sessions/test.json  # macOS
cat ~/.cache/ovim/sessions/test.json          # Linux

# Test LSP endpoints
curl http://127.0.0.1:PORT/lsp/status | jq '.'
curl http://127.0.0.1:PORT/health | jq '.'
```

## Common Tasks

**Add new REST API endpoint:**
1. Add variant to `ApiRequest` in `api/state.rs`
2. Add variant to `ApiResponse` in `api/state.rs`
3. Add handler in `api/handlers.rs`
4. Add route in `api/routes.rs`
5. Handle in `handle_api_request()` in `main.rs`

**Add new LSP feature:**
1. Add method to `LspManager` in `lsp/mod.rs`
2. Call from `Editor` LSP action methods
3. Add to `pending_lsp_action` enum if async
4. Process in `process_pending_lsp_actions()`

**Add new operator:**
1. Add function in `editor/operators.rs`
2. Call from operator dispatch in `editor/input.rs`
3. Add tests in `tests/`

## Environment

```bash
# Working directory
/Users/adrian.helvik/Personal/ovim

# Platform
Darwin 24.4.0 (macOS)

# Build target
aarch64-apple-darwin

# Dependencies
ropey, ratatui, crossterm, axum, tokio, lsp-types, mlua (optional)
```
