# ovim - System Documentation

Welcome to the ovim documentation. This folder contains comprehensive guides for understanding, developing, and extending ovim.

## 📚 Documentation Structure

### Getting Started
- **[ARCHITECTURE.md](./ARCHITECTURE.md)** - High-level system design and component overview
- **[MODULES.md](./MODULES.md)** - Detailed module structure and responsibilities

### Core Systems
- **[LSP_SYSTEM.md](./LSP_SYSTEM.md)** - Language Server Protocol implementation
- **[BUFFER_SYSTEM.md](./BUFFER_SYSTEM.md)** - Text buffer and rope management
- **[EDITOR_SYSTEM.md](./EDITOR_SYSTEM.md)** - Editor core (operations, motions, commands)
- **[UI_SYSTEM.md](./UI_SYSTEM.md)** - Terminal UI and rendering

### Advanced Topics
- **[SESSION_MANAGEMENT.md](./SESSION_MANAGEMENT.md)** - Session lifecycle and persistence
- **[API_ENDPOINTS.md](./API_ENDPOINTS.md)** - REST API specification
- **[PERFORMANCE.md](./PERFORMANCE.md)** - Optimization techniques and benchmarks
- **[TESTING.md](./TESTING.md)** - Testing strategies and guidelines

### Development
- **[DEVELOPMENT.md](./DEVELOPMENT.md)** - Development setup and workflow
- **[CODE_STYLE.md](./CODE_STYLE.md)** - Code organization and style guidelines
- **[EXTENDING.md](./EXTENDING.md)** - How to add new features

---

## 🚀 Quick Navigation

**New to ovim?**
→ Start with [ARCHITECTURE.md](./ARCHITECTURE.md)

**Want to understand a specific system?**
→ See the Core Systems section above

**Ready to develop?**
→ Read [DEVELOPMENT.md](./DEVELOPMENT.md) and [EXTENDING.md](./EXTENDING.md)

**Looking for API details?**
→ Check [API_ENDPOINTS.md](./API_ENDPOINTS.md)

---

## 📋 Project Overview

**ovim** is a Neovim clone written in Rust with:
- Full Vi/Vim keybindings and operators
- Language Server Protocol (LSP) support for intelligent code editing
- Headless mode with REST API for automation
- Session management for multi-session workflows
- Terminal UI with ratatui framework

**Key Features:**
- Zero-config Java support (auto-downloads jdtls)
- Multi-language support (Rust, Python, JavaScript, Java)
- Rope-based efficient text buffer
- Session persistence and discovery
- Full test coverage with integration tests

---

## 🏗️ Directory Structure

```
ovim/
├── src/
│   ├── main.rs              # Entry point, event loops
│   ├── lib.rs               # Library exports
│   ├── api/                 # REST API (Axum)
│   ├── buffer/              # Text buffer (ropey)
│   ├── editor/              # Core editor logic
│   │   ├── input.rs         # Key event handling
│   │   ├── operators.rs     # d, c, y, etc.
│   │   ├── motions.rs       # w, e, b, j, k, etc.
│   │   ├── textobjects.rs   # iw, aw, etc.
│   │   └── ...
│   ├── lsp/                 # Language Server Protocol
│   │   ├── mod.rs           # LspManager
│   │   ├── server.rs        # LanguageServer
│   │   ├── supervisor.rs    # Multi-server management
│   │   └── ...
│   ├── ui/                  # Terminal UI
│   │   ├── renderer/        # Ratatui rendering
│   │   └── ansi.rs          # ANSI export for headless
│   ├── session/             # Session management
│   ├── config/              # Configuration (Lua)
│   ├── commands.rs          # Ex commands (`:w`, `:set`, etc.)
│   └── ...
├── tests/                   # Integration tests
├── docs/                    # This documentation
├── notes/                   # Development notes
└── Cargo.toml              # Rust dependencies
```

---

## 🔄 Event Loop Architecture

ovim has two main event loops:

### TUI Mode
```
Terminal Events → Input Handler → Editor Operations → LSP Actions → Render
                     ↓
                State Changes → UI Update → Terminal Draw
```

### Headless Mode
```
API Requests → Handler → Editor Operations → LSP Actions → Response
                ↓
         Session Update → Response Sent
```

---

## 📡 Data Flow Examples

### Example 1: Hover Request
```
User presses K (hover keybinding)
    ↓
Input handler detects KeyCode::Char('K')
    ↓
Editor::hover() called with cursor position
    ↓
ensure_document_opened() → LSP didOpen
    ↓
LspManager::hover() sends textDocument/hover request
    ↓
rust-analyzer responds with hover info
    ↓
Hover window rendered on screen
```

### Example 2: LSP Diagnostics
```
File edited
    ↓
notify_did_change() (debounced 150ms)
    ↓
rust-analyzer re-analyzes
    ↓
publishDiagnostics notification received
    ↓
Diagnostics stored in Editor::diagnostics
    ↓
UI renders gutter signs and underlines
```

---

## 🎯 Key Design Decisions

1. **Rope-based Buffer**: Efficient for large files and frequent edits
2. **Async Tokio**: Non-blocking LSP and long-running operations
3. **Session Files**: JSON for persistence, atomic writes for safety
4. **Debounced Changes**: Reduces LSP traffic (150ms debounce)
5. **Modular LSP**: Separate supervisor for managing multiple servers
6. **ANSI Export**: Headless mode exports rendering as ANSI for scripting

---

## ✅ Testing Strategy

- **Unit Tests**: Isolated component testing (55+ tests)
- **Integration Tests**: Multi-component workflows
- **Headless Tests**: LSP and API testing without UI
- **Performance Tests**: Large file benchmarks

Run all tests:
```bash
cargo test --lib
cargo test --test '*'
```

---

## 🔗 External Resources

- [LSP Specification](https://microsoft.github.io/language-server-protocol/)
- [Vim Reference Manual](https://vim.org/docs.php)
- [Ropey Documentation](https://docs.rs/ropey/)
- [Ratatui Guide](https://docs.rs/ratatui/)

---

## 📞 Getting Help

- Check relevant documentation files in this folder
- Review [DEVELOPMENT.md](./DEVELOPMENT.md) for common questions
- Search codebase comments for implementation details
- Look at existing tests for usage examples

---

**Last Updated**: 2025-10-26
**Documentation Status**: Comprehensive
**Code Status**: 55/55 tests passing ✅
