# ovim

A Vim clone in Rust with LSP support and seamless headless mode for testing and automation.

## Quick Start

```bash
cargo build --release

# Interactive editing
./target/release/ovim myfile.txt

# Headless mode with named session
./target/release/ovim myfile.rs --headless --session dev
```

# Development Workflow
~/.cache/ovim/sessions/` (Mac) or `~/.cache/ovim/sessions/` (Linux)
- Auto-cleanup on exit or manual `./ovim-ctl kill`
- Multiple concurrent sessions with different names
- LSP readiness tracking per session

### REST API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Health check with LSP readiness |
| `/lsp/status` | GET | LSP server states & pending requests |
| `/snapshot` | GET | Complete editor state |
| `/buffer` | GET/PUT | Buffer content |
| `/cursor` | GET | Cursor position |
| `/mode` | GET | Current mode |
| `/keys` | POST | Send keystrokes |
| `/command` | POST | Execute ex command |
| `/render` | GET | ANSI rendering |

### LSP Support

Zero-config Java support:
```bash
ovim MyClass.java  # Auto-downloads jdtls, detects Java version, full IDE features
```

Rust (rust-analyzer), Python (pyright), JavaScript (typescript-language-server) also supported.

#### LSP Logging

All LSP communication is logged:
```
[LSP-REQUEST] Method: textDocument/hover | Request ID: Number(2) | Server: rust
[LSP-REQUEST] Pending requests before: 0 | Adding: textDocument/hover
[LSP-REQUEST] Waiting for response (timeout: 10s)
[LSP-RESPONSE] Success: textDocument/hover | Took: 751.5µs | Request ID: Number(2)
```

### Testing

```bash
# Unit tests
cargo test

# Integration tests with headless mode
./target/release/ovim test.txt --headless --session test &
./ovim-ctl send test "iHello<Esc>"
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

print("Config loaded!")
```

Reload: `:ConfigReload` or `:reload`

## Architecture

- **buffer/**: Rope-based text buffer
- **editor/**: Core logic, operators, motions, LSP actions
- **lsp/**: Language server protocol client
- **api/**: REST API server (Axum)
- **session/**: Session persistence & management
- **ui/**: Terminal UI (ratatui + crossterm)

## Features

- Modal editing (Normal, Insert, Visual, Command)
- Operators + motions (d, c, y + w, $, gg, etc.)
- Visual selection, undo/redo, macros, marks
- Text objects (w, p, sentence, quotes, brackets)
- Search (/, ?, n, N)
- LSP (hover, go-to-def, completion, diagnostics)
- Lua configuration
- Git status in UI
- Session management for headless mode

## Contributing

Run before committing:
```bash
cargo fmt
cargo clippy
cargo test
```

## License

[Add license]
