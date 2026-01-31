# ovim

A Neovim clone in Rust with LSP support and seamless headless mode.

## Quick Reference

```bash
# Build
cargo build --release

# Run editor
./target/release/ovim file.txt
./target/release/ovim file.rs --headless --session dev

# Control sessions (auto-discovers by default!)
./target/release/ovim sessions                # List all sessions
./target/release/ovim send "ggK"              # Auto-discover session, send commands (with \e, \c, \n escapes)
./target/release/ovim context                 # Auto-discover & get 21-line context around cursor (AI-optimized!)
./target/release/ovim buffer                  # Auto-discover & get full buffer content
./target/release/ovim mcp tools/list          # Auto-discover & send MCP requests
./target/release/ovim health                  # Auto-discover & check health
./target/release/ovim lsp-status              # Auto-discover & get LSP server status
./target/release/ovim kill                    # Auto-discover & kill session
./target/release/ovim cleanup                 # Clean up stale/expired/corrupted session files
./target/release/ovim cleanup --dry-run       # Show what would be cleaned up (no changes)
./target/release/ovim cleanup --max-age 7     # Remove sessions older than 7 days

# Explicit session override (when multiple sessions running)
./target/release/ovim send "ggK" --session dev
./target/release/ovim context --session dev
./target/release/ovim buffer --session dev
```

**New in v0.1**: Built-in session control (editor + client in one binary). **Session auto-discovery** (single session = no session flag needed). See [CLI_SUBCOMMANDS.md](code-docs/CLI_SUBCOMMANDS.md) for details.

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
2. **Control it**: `./ovim send "ggK"` (auto-discovers session!)
3. **Check LSP**: `./ovim lsp-status` (auto-discovers)
4. **Kill it**: `./ovim kill` (auto-discovers and cleans up)

Sessions stored in:
- macOS: `~/Library/Caches/ovim/sessions/`
- Linux: `~/.cache/ovim/sessions/`

### Session Auto-Discovery

All CLI commands support automatic session discovery:

**Single session** (most common case):
```bash
ovim send "ggK"                    # Just works!
ovim context                       # Gets context window
ovim buffer                        # Gets buffer content
ovim exec "set number"             # Executes ex command
ovim snapshot --format json        # Gets editor state
```

**Multiple sessions** (use --session flag to specify):
```bash
ovim send "ggK" --session projectA
ovim context --session projectB
ovim kill --session projectA
```

**Discovery priority** (when multiple sessions exist):
1. Named sessions first (e.g., `--session myproject`)
2. Most recent auto-generated session (by timestamp)
3. Falls back to `default` if no named sessions

This makes the common single-session workflow frictionless while still supporting complex multi-session setups.

### Language Support

**Introspection Commands**:
```bash
ovim list-languages              # Show all configured languages
ovim list-languages --verbose    # Show detailed LSP configuration
ovim check-lsp file.rs           # Check language detection & LSP status for file
ovim check-lsp file.ts --verbose # Show full config + installation hints
```

**Supported Languages** (out-of-the-box):
- **Rust** - rust-analyzer (install via `rustup component add rust-analyzer`)
- **TypeScript/JavaScript** - typescript-language-server (auto-installs via npm)
- **Python** - pyright-langserver (install via `pip install pyright`)
- **Java** - hyperion-lsp (auto-downloads, zero config!)
- **Markdown, JSON, YAML, HTML, CSS, Go, C/C++, Ruby, Bash** - syntax highlighting only

**Adding/Customizing Languages**:
Create `~/.config/ovim/languages.toml`:
```toml
[[language]]
id = "go"
name = "Go"
extensions = ["go"]

[language.lsp]
command = "gopls"
root_markers = ["go.mod"]
install_hint = "go install golang.org/x/tools/gopls@latest"
```

See [user-docs/LANGUAGE_SUPPORT.md](user-docs/LANGUAGE_SUPPORT.md) for complete guide.

### LSP Features

**Zero-config TypeScript** (auto-install):
```bash
ovim app.tsx  # Detects TypeScript, offers to install LSP via npm
```

**Zero-config Java** (auto-download):
```bash
ovim MyClass.java  # Auto-downloads jdtls, detects Java version, full IDE
```

**LSP Logging** (all requests/responses logged to stderr):
```
[LSP-REQUEST] Method: textDocument/hover | Request ID: Number(2) | Server: rust
[LSP-RESPONSE] Success: textDocument/hover | Took: 751.5µs
```

**LSP Introspection**:
```bash
curl http://127.0.0.1:PORT/v1/lsp/status  # Server states, pending requests
curl http://127.0.0.1:PORT/v1/health      # LSP readiness check
```

### REST API Endpoints

**HTTP Server**: Always runs on both headless and UI modes on `http://127.0.0.1:PORT`

**API Version**: All endpoints are available under `/v1/` prefix (recommended) and without prefix (legacy, deprecated).

| Endpoint | Method | Use Case |
|----------|--------|----------|
| `/v1/health` | GET | Health + LSP readiness |
| `/v1/lsp/status` | GET | Server states & pending requests |
| `/v1/snapshot` | GET | Complete editor state (buffer, cursor, mode, registers, marks) |
| `/v1/buffer` | GET/PUT | Buffer content |
| `/v1/cursor` | GET | Cursor position |
| `/v1/mode` | GET | Current mode |
| `/v1/keys` | POST | Send keystrokes (e.g., `{"keys": "ggK"}`) |
| `/v1/command` | POST | Execute ex command (e.g., `{"command": "w"}`) |
| `/v1/render` | GET | ANSI rendering |
| `/v1/mcp` | POST | Model Context Protocol (MCP) JSON-RPC 2.0 endpoint |

**Note**: Legacy unversioned endpoints (e.g., `/health`, `/buffer`) still work but are deprecated and will be removed in ovim v1.0. They return `X-API-Deprecation` and `Sunset` headers. Update your clients to use `/v1/` prefix.

### MCP (Model Context Protocol) Support

**ovim** is MCP-compliant, exposing its capabilities as an MCP server via JSON-RPC 2.0.

**Supported MCP Methods**:
- `initialize` - Capability negotiation
- `tools/list` - List available tools
- `tools/call` - Execute tools

**Available Tools**:
- `send_keys` - Send Vim key sequences to editor
- `get_buffer` - Get current buffer content
- `set_buffer` - Replace buffer content
- `get_cursor` - Get cursor position
- `set_mode` - Change editor mode (NORMAL, INSERT, VISUAL, etc.)
- `execute_command` - Execute ex commands
- `lsp_hover` - Get LSP hover information
- `lsp_goto_definition` - Jump to definition
- `get_snapshot` - Get complete editor state
- `get_health` - Get session health and LSP readiness
- `get_lsp_status` - Get language server status
- `get_context_window` - Get 21-line context around cursor (AI-optimized)
- `list_sessions` - List all active sessions

**Escape Sequences** (for `send_keys`):
When sending key sequences, use these escape sequences for special keys:
```
\e      Escape key
\c      Ctrl+C (cancel/interrupt)
\n      Enter/newline
\\      Literal backslash
```

Examples:
```
send_keys("/pattern\n")         # Search for pattern and confirm
send_keys("i\e")                # Insert mode, then escape
send_keys("d3w\\e")             # Delete 3 words (literal backslash at end)
```

**Context Window** (AI-First Feature):
The `get_context_window` tool returns a 21-line view (10 above, current, 10 below) with:
- Header showing filename, mode, and cursor position: `[ovim: file.rs | NORMAL | L42:C15]`
- Line numbers with cursor marker (`>>`) and position indicator (`^`)
- Automatic line truncation at 80 characters with `...`
- `FILE END` marker when end of file is visible

Example:
```json
{
  "context": "[ovim: main.rs | NORMAL | L17:C7]\n   15 | // Start line\n   16 | \n>> 17 | let x = calculate(data);\n                   ^\n   18 | print(x);\nFILE END\n",
  "file": "main.rs",
  "mode": "NORMAL",
  "line": 16,
  "column": 7
}
```

This tool is optimized for AI workflows - use it to get contextual information without fetching the entire buffer.

**Session Parameter**:
All tools (except `list_sessions`) support an optional `session` parameter to explicitly specify which session to use. This is useful when multiple ovim sessions are running:

```
send_keys(keys="ggdd", session="tui_12345_1234567890")
set_mode(mode="INSERT", session="my_session")
```

If `session` is omitted:
- Single session running: Auto-discovers and uses it ✓
- Multiple sessions running: Returns error with list of available sessions
- Current session (from previous calls): Uses it if available

This ensures predictable behavior when working with multiple sessions simultaneously.

**Resources**:
- `resources/list` - List available resources (buffer, snapshot, lsp/status, current file)
- `resources/read` - Read resource content
- `prompts/list` - List available prompts (empty for now)

**Example MCP Usage**:
```bash
# Initialize
curl -X POST http://127.0.0.1:PORT/v1/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"client","version":"1.0"}}}'

# List tools
curl -X POST http://127.0.0.1:PORT/v1/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'

# Call tool (send keys)
curl -X POST http://127.0.0.1:PORT/v1/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"send_keys","arguments":{"keys":"gg"}}}'

# Set editor mode (ensures correct state before operations)
curl -X POST http://127.0.0.1:PORT/v1/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"set_mode","arguments":{"mode":"NORMAL"}}}'

# Read resource
curl -X POST http://127.0.0.1:PORT/v1/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":6,"method":"resources/read","params":{"uri":"ovim://buffer"}}'
```

**Available Resources**:
- `ovim://context-window` - 21-line context around cursor (text/plain, AI-optimized!)
- `ovim://buffer` - Current buffer content (text/plain)
- `ovim://snapshot` - Complete editor state (application/json)
- `ovim://lsp/status` - LSP server status (application/json)
- `file://PATH` - Current file being edited (text/plain)

### Auto-Injection for Claude Code

For seamless AI editing with Claude Code CLI, set up auto-context injection:

**Option 1: Using `ovim context` (Recommended)**
Every time you message Claude Code, automatically include the current editor context:

```bash
# In .claude/hooks/before_response.sh (create if needed)
#!/bin/bash
SESSION=$(./target/release/ovim sessions | grep -o 'tui[^ ]*' | head -1)
if [ -n "$SESSION" ]; then
  ./target/release/ovim context "$SESSION" | head -30
fi
```

Then Claude Code will whisper the context into every response automatically.

**Option 2: Manual context fetching**
Just ask for context when you need it:

```bash
./target/release/ovim context my-session
```

**Why this matters:**
- **Before**: Blind, asking for full buffers (116+ lines of JSON)
- **Now**: Always see context (21 lines, formatted for reading)
- **Impact**: AI can work with surgical precision, not bulk replacements

The context window includes the header `[ovim: file.rs | NORMAL | L42:C15]` so you (and Claude) always know where you are.

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

More details in ./notes/TESTING.md

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
- Auto-cleanup of stale sessions (dead processes) during discovery
- PID verification with process start time to prevent PID reuse issues
- Session health checks verify API endpoint accessibility
- Session expiry support (optional max-age) for long-running cleanup
- Atomic writes with temp files to prevent corruption
- Detailed cleanup reporting with dry-run support

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
curl http://127.0.0.1:PORT/v1/lsp/status | jq '.'
curl http://127.0.0.1:PORT/v1/health | jq '.'
```

## AI-First IDE

ovim is designed as an **AI-first IDE** with native MCP support and integrated CLI:

**Multi-Session Workflows:**
```bash
# Spawn sessions for multiple files
ovim --headless --session main src/main.rs &
ovim --headless --session lib src/lib.rs &

# Query and edit via CLI
ovim mcp main tools/call '{"name":"get_buffer"}'
ovim send lib "ggdd"

# Coordinate changes
ovim sessions  # See all active sessions
ovim kill main lib  # Cleanup
```

### MCP for Any LLM Client

The **HTTP `/mcp` endpoint** is the primary MCP interface. Any tool can use ovim's MCP by:

1. **Discovering sessions**: Read `~/.cache/ovim/sessions/*.json` for port info
2. **Sending MCP requests**: POST JSON-RPC 2.0 to `http://127.0.0.1:PORT/mcp`
3. **Using the CLI**: `ovim mcp SESSION_NAME METHOD PARAMS`

**Supported MCP clients:**
- Claude Desktop (via `ovim install claude`)
- Cursor IDE (via `ovim install cursor`)
- Custom tools (POST to `/mcp` endpoint)
- Any MCP-compatible client

**Example workflows:**
```bash
# Install for Claude/Cursor
ovim install claude
ovim install cursor

# Or just use HTTP directly with any tool
curl -X POST http://127.0.0.1:PORT/v1/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'
```

See:
- [CLI_SUBCOMMANDS.md](code-docs/CLI_SUBCOMMANDS.md) - Complete CLI reference
- [AI_WORKFLOWS.md](code-docs/AI_WORKFLOWS.md) - AI workflow examples
- [MCP_INTEGRATION.md](code-docs/MCP_INTEGRATION.md) - MCP specification

## Common Tasks

**Add new CLI subcommand:**
1. Add variant to `Command` enum in `cli.rs`
2. Implement handler in `subcommands.rs`
3. Use `OvimClient` for HTTP/MCP requests

**Add new REST API endpoint:**
1. Add variant to `ApiRequest` in `api/state.rs`
2. Add variant to `ApiResponse` in `api/state.rs`
3. Add handler in `api/handlers.rs`
4. Add route in `api/routes.rs`
5. Handle in `handle_api_request()` in `main.rs`

**Add new MCP tool:**
1. Add tool definition in `api/mcp.rs::get_tools()`
2. Handle in `mcp_handler.rs::handle_tool_call()`
3. Map to existing `ApiRequest` or add new one

**Add new LSP feature:**
1. Add method to `LspManager` in `lsp/mod.rs`
2. Call from `Editor` LSP action methods
3. Add to `pending_lsp_action` enum if async
4. Process in `process_pending_lsp_actions()`

**Add new operator:**
1. Add function in `editor/operators.rs`
2. Call from operator dispatch in `editor/input.rs`
3. Add tests in `tests/`

## Performance

### Large File Optimizations
**Thresholds**: 50K lines or 5MB triggers optimizations

**Implemented**:
- `Buffer::is_large_file()` - detects large files (buffer/mod.rs:591-607)
- Auto-disable syntax highlighting for files >50K lines (buffer/mod.rs:554-580)
- Render dirty flag - only redraws on state changes (editor/mod.rs:210, event_loop.rs:196-205)
- `/metrics` endpoint - buffer size, render count, timing (event_loop.rs:516-537)

**Impact**: 10-20x faster large file loading, 10-50x less idle CPU

**Generate benchmarks**: `./generate-benchmarks.sh` (not committed, regenerate locally)

### LSP Hover Fix
**Issue**: Debounced `didChange` (150ms) causes stale hover data when pressing `K` immediately after typing

**Fix**: Flush pending changes before hover/goto_definition requests (editor/mod.rs:2367-2376, 3367-3378)
- Pattern: acquire lock → flush → release → 10ms delay → re-acquire → request
- Prevents lock contention while ensuring LSP has latest content

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

## Best practices

Files should be no longer than 3k lines of code. When it starts creeping up towards 2k lines, refactor.
If a file is too large to read, immediately start refactoring it by splitting it up into chunks that are readable and making fields public to just *get smaller files*. First dirty refactor, then clean it up, then proceed with what you were working on before.
Remember to remove debug logging after debugging sessions, as one stray eprintln!() may break the user facing TUI.
Commit as you go.

## Note taking

- Keep notes within the notes folder at the root of the repo
- Keep notes structured and up to date
- Notes include IMPLEMENTATION_STATUS.md, DESIGN.md, etc. Keep these up to date.

## Documentation

Document the structure of the codebase in the code-docs/ folder. Keep it neat and organized.

Document usage in the user-docs/ folder. It should be well organized and easy to understand for users.

## User instructions

User instructions are located in PRIORITIES.md and should be prioritized before other tasks. Only check off the tasks (when done and verified), don't edit the text.

## "Pre-existing test failures"? Also avoid running the full test suite

This likely means that another agent is working on unrelated functionality. It is critical that you don't `git stash` to check if it is pre-existing. The user will handle consolidation and running the full test suite in such cases. If you stash another agent's progress, they'll get confused and mess up.

## Committing

Commit early and often when the code is in a good state. Commit your changes. If your changes overlap with changes from another agent, don't commit.
