# Module Structure

This document provides a detailed breakdown of each module in ovim.

## Directory Tree with Descriptions

```
src/
├── main.rs                          # Entry point, event loops (TUI & Headless)
├── lib.rs                           # Library exports, module declarations
├── api/                             # REST API for headless mode (Axum)
│   ├── mod.rs                       # API module coordination
│   ├── state.rs                     # ApiRequest/ApiResponse enums
│   ├── handlers.rs                  # Endpoint handlers (snapshot, keys, etc)
│   ├── routes.rs                    # Route definitions
│   └── server.rs                    # REST server setup
├── buffer/                          # Text buffer (ropey-based)
│   ├── mod.rs                       # Buffer struct, rope operations
│   └── syntax.rs                    # Syntax highlighting
├── editor/                          # Core editor logic (LARGE MODULE)
│   ├── mod.rs                       # Editor state, main dispatch
│   ├── input/                       # Key event handling (5824 lines - needs refactoring)
│   │   ├── mod.rs                   # Main input dispatcher
│   │   ├── operators.rs             # Delete, yank, change, etc (SEPARATE)
│   │   ├── motions.rs               # Word, line, paragraph movements
│   │   └── textobjects.rs           # iw, aw, i", a", etc
│   ├── operators.rs                 # Operator implementations
│   ├── motions.rs                   # Motion implementations
│   ├── textobjects.rs               # Text object implementations
│   ├── commands.rs                  # Ex commands (:w, :set, :e, etc)
│   ├── register.rs                  # Register management (a-z, 0-9, etc)
│   ├── change.rs                    # Undo/redo change types
│   ├── marks.rs                     # Mark management
│   ├── visual.rs                    # Visual mode operations
│   ├── yank_register.rs             # Yank buffer
│   ├── completion.rs                # Completion menu
│   ├── fold.rs                      # Code folding (zf, zo, zc, etc)
│   ├── quickfix.rs                  # Quickfix list
│   ├── tabpage.rs                   # Tab page management
│   ├── filetree.rs                  # File explorer
│   ├── window.rs                    # Window/split management
│   ├── lsp_integration.rs           # LSP core: init, polling, document sync, dispatcher
│   └── lsp_modules/                 # LSP feature implementations
│       ├── mod.rs                   # Module declarations
│       ├── hover.rs                 # Hover display (K command)
│       ├── goto.rs                  # Go-to-definition, implementation, type
│       ├── diagnostics.rs           # Error/warning diagnostics
│       ├── completion.rs            # Code completion
│       ├── actions.rs               # Formatting, code actions, rename, semantic tokens
│       ├── references.rs            # Find references, document/workspace symbols, call/type hierarchy
│       └── workspace_edits.rs       # Text edit application, workspace edit handling
├── lsp/                             # Language Server Protocol
│   ├── mod.rs                       # LspManager (main coordinator)
│   ├── server.rs                    # LanguageServer (per-language instance)
│   ├── supervisor.rs                # Multi-server supervision
│   ├── protocol.rs                  # JSON-RPC 2.0 message handling
│   ├── logger.rs                    # Request/response logging
│   ├── types.rs                     # Type conversions (LSP ↔ ovim)
│   └── lsp_init/                    # Language-specific initialization
│       ├── rust.rs                  # Rust (rust-analyzer)
│       ├── python.rs                # Python (pyright)
│       ├── javascript.rs            # JavaScript (typescript-language-server)
│       └── java.rs                  # Java (jdtls)
├── ui/                              # Terminal UI (ratatui)
│   ├── mod.rs                       # UI module coordination
│   ├── renderer/                    # Main rendering engine (refactored)
│   │   ├── mod.rs                   # Module exports
│   │   ├── core.rs                  # Main render loop
│   │   ├── buffer.rs                # Buffer + syntax highlighting
│   │   ├── widgets.rs               # Picker, hover, completion, etc
│   │   ├── helpers.rs               # Text utilities
│   │   └── styles.rs                # Color/style generation
│   ├── terminal.rs                  # Terminal wrapper
│   └── ansi.rs                      # ANSI export for headless
├── config/                          # Configuration
│   └── mod.rs                       # Lua config support (mlua)
├── session/                         # Session management
│   └── mod.rs                       # SessionInfo, persistence
├── commands.rs                      # Command execution
├── daemon/                          # Background service daemon
│   ├── mod.rs                       # Daemon main
│   ├── pid.rs                       # PID tracking/verification
│   ├── process.rs                   # Process management
│   ├── protocol.rs                  # Daemon IPC protocol
│   └── lock.rs                      # Lock file handling
├── git.rs                           # Git integration
├── event_loop.rs                    # Main event processing
├── git.rs                           # Git status/signs
└── utils.rs                         # General utilities

tests/                              # Integration tests
├── advanced_editing_test.rs         # Complex editing operations
├── api_test.rs                      # REST API tests
├── command_mode_test.rs             # Ex command tests
├── editor_test.rs                   # Editor core tests
├── helpers/                         # Test utilities
│   └── mod.rs                       # EditorTest, fluent API
├── insert_mode_test.rs              # Insert mode tests
├── lsp_hover_test.rs                # LSP hover functionality
├── lsp_operations_test.rs           # LSP goto, rename, etc
├── lsp_multi_file_test.rs           # Multi-file LSP tests
├── macros_test.rs                   # Macro recording tests
├── marks_test.rs                    # Mark navigation tests
├── motion_edge_cases_test.rs        # Motion boundary conditions
├── operators_test.rs                # Operator tests
├── replace_mode_test.rs             # Replace mode tests
├── search_test.rs                   # Search/find tests
├── text_objects_test.rs             # Text object tests
├── unicode_edge_cases_test.rs       # Unicode handling tests
└── visual_block_mode_test.rs        # Visual block tests
```

---

## Module Responsibilities

### **main.rs** - Entry Point
- **Responsibility**: Application entry, event loop selection
- **Exports**: `main()` function
- **Contains**:
  - TUI mode event loop (`tui_main`)
  - Headless mode event loop (`headless_main`)
  - Session initialization
  - Signal handlers (SIGINT, SIGTERM)

### **api/** - REST API Server
- **Responsibility**: HTTP endpoint handling for headless mode
- **Framework**: Axum
- **Endpoints**:
  - `/snapshot` - Complete editor state
  - `/keys` - Send key events
  - `/buffer` - Get/set content
  - `/cursor` - Cursor position
  - `/mode` - Current mode
  - `/command` - Execute ex command
  - `/render` - ANSI rendered output
  - `/lsp/status` - Language server status
  - `/health` - Health check
  - `/metrics` - Performance metrics
- **Key Types**:
  - `ApiRequest` - Request variants
  - `ApiResponse` - Response variants
  - `SuccessResponse<T>` - Generic success wrapper
  - `ErrorResponse` - Error details

### **buffer/** - Text Storage
- **Responsibility**: Efficient text buffer management
- **Uses**: ropey crate (rope data structure)
- **Main Type**: `Buffer`
  - `from_str()` - Create from string
  - `insert_text_at()` - Insert text
  - `delete_range()` - Delete range
  - `line()` - Get line by index
  - `char_at()` - Get character
- **Features**:
  - Large file detection (>50K lines)
  - UTF-8 aware
  - UTF-16 conversion for LSP
  - Syntax highlighting
- **Performance**: O(log n) insert/delete

### **editor/** - Core Editing Logic
- **Responsibility**: All editing operations
- **Main Type**: `Editor`
  - State: buffer, cursor, mode, registers, marks, undo stack
  - Operations: `handle_key()`, `delete()`, `yank()`, `change()`
  - Motions: `move_forward()`, `move_word_end()`, etc
  - Text objects: `select_inner_word()`, `select_around_word()`
  - Commands: `write_file()`, `set_option()`, etc
- **Submodules**:
  - `input/` - Key event parsing and dispatch
  - `operators.rs` - d, c, y, >, <, gu, gU, g~, r
  - `motions.rs` - w, e, b, j, k, f, t, %, etc
  - `textobjects.rs` - iw, aw, i", a", i(, a(, etc
  - `commands.rs` - :w, :set, :e, :q, :qa, etc
  - `register.rs` - Named, numbered, special registers
  - `change.rs` - Undo/redo change types
  - `marks.rs` - Mark creation/navigation
  - `visual.rs` - Visual, visual-line, visual-block modes
  - `completion.rs` - Completion menu display
  - `fold.rs` - Code folding
  - `quickfix.rs` - Quickfix list
  - `tabpage.rs` - Tab page management
  - `filetree.rs` - File explorer
  - `window.rs` - Window/split management

### **lsp/** - Language Server Protocol
- **Responsibility**: LSP client implementation
- **Main Types**:
  - `LspManager` - Coordinates multiple servers
  - `LanguageServer` - Individual server instance
  - `LspSupervisor` - Manages server lifecycle
- **Key Methods**:
  - `hover()` - Hover information
  - `goto_definition()` - Jump to definition
  - `goto_implementation()` - Jump to implementation
  - `completion()` - Completions
  - `rename()` - Symbol rename
  - `format()` - Document formatting
- **Supported Languages**:
  - Rust (rust-analyzer)
  - Python (pyright)
  - JavaScript/TypeScript (typescript-language-server)
  - Java (jdtls)
- **Features**:
  - Non-blocking async
  - Debounced text changes (150ms)
  - Multi-server support
  - Full request/response logging
  - Server capability caching
  - Diagnostic aggregation

### **ui/** - Terminal User Interface
- **Responsibility**: Terminal rendering and display
- **Framework**: ratatui + crossterm
- **Components**:
  - `renderer/core.rs` - Main render loop
  - `renderer/buffer.rs` - Buffer content + syntax highlighting
  - `renderer/widgets.rs` - Picker, hover, completion, file tree
  - `renderer/helpers.rs` - Text utilities (truncate, tab expand)
  - `renderer/styles.rs` - Color/styling
  - `ansi.rs` - ANSI export for headless
- **Features**:
  - Syntax highlighting (for multiple languages)
  - Git gutter signs
  - Diagnostics underlines
  - Multiple widgets (hovering, completion, picker)
  - Theme support (10+ themes)
  - Responsive resize handling
- **Rendering Pipeline**:
  1. Create frame
  2. Render gutter (line numbers + git signs)
  3. Render buffer content with highlights
  4. Render diagnostics
  5. Render status line
  6. Render widgets (picker, hover, etc)
  7. Draw cursor
  8. Flush to terminal

### **config/** - User Configuration
- **Responsibility**: Lua configuration support
- **Uses**: mlua crate
- **Location**: `~/.config/ovim/init.lua`
- **Capabilities**:
  - Set options (`vim.opt`)
  - Define keymaps (`vim.keymap.set`)
  - Custom commands
  - Plugin hooks (future)
- **Exports to Lua**:
  - `vim.opt` - Option setters
  - `vim.keymap.set()` - Keybinding
  - `vim.cmd()` - Execute ex command

### **session/** - Session Management
- **Responsibility**: Multi-session support and persistence
- **Main Type**: `SessionInfo`
  - PID and start time (for verification)
  - Port number (for API)
  - File path
  - LSP readiness status
- **Features**:
  - Atomic writes (write-to-temp-then-rename)
  - PID verification (prevents stale sessions)
  - Automatic cleanup on exit
  - Auto-detection via ovim-ctl
- **Storage**:
  - macOS: `~/Library/Caches/ovim/sessions/`
  - Linux: `~/.cache/ovim/sessions/`

### **daemon/** - Background Service
- **Responsibility**: Daemon process for session discovery
- **Main Types**:
  - `DaemonPidInfo` - Process metadata
  - `DaemonLock` - Lock file coordination
- **Features**:
  - Session discovery without polling
  - Lock file for mutual exclusion
  - Process state verification
  - Used by ovim-ctl for session management

### **git.rs** - Git Integration
- **Responsibility**: Git status information
- **Features**:
  - Repository detection
  - File status (modified, staged, untracked)
  - Gutter sign generation
- **Uses**: git2 or direct subprocess

---

## Module Dependency Graph

```
main.rs
  ├── event_loop.rs
  │   ├── editor/mod.rs (Editor)
  │   │   ├── buffer/mod.rs (Buffer)
  │   │   ├── lsp/mod.rs (LspManager)
  │   │   ├── editor/input (Key handling)
  │   │   ├── editor/operators (Operations)
  │   │   └── editor/commands (Ex commands)
  │   ├── ui/renderer (Rendering)
  │   │   ├── ui/ansi (ANSI export)
  │   │   └── syntax/ (Highlighting)
  │   └── session/ (Persistence)
  │
  ├── api/
  │   ├── api/handlers
  │   ├── api/routes
  │   └── api/state
  │
  ├── lsp/
  │   ├── lsp/server (LanguageServer)
  │   ├── lsp/supervisor (Supervision)
  │   ├── lsp/protocol (JSON-RPC)
  │   └── lsp/lsp_init/* (Language-specific)
  │
  └── config/ (Lua configuration)
```

---

## File Size Reference

| File | Lines | Status |
|------|-------|--------|
| editor/input/mod.rs | 5,824 | 🔴 Needs refactoring (too large) |
| editor/lsp_integration.rs | 851 | 🟢 Refactored (was 2,489) |
| editor/lsp_modules/references.rs | 519 | 🟢 OK (extracted from lsp_integration) |
| editor/lsp_modules/actions.rs | 331 | 🟢 OK (extracted from lsp_integration) |
| editor/lsp_modules/workspace_edits.rs | 208 | 🟢 OK (extracted from lsp_integration) |
| lsp/mod.rs | 2,287 | 🟡 Could benefit from split |
| editor/mod.rs | 5,192 | 🟡 Large but manageable |
| buffer/mod.rs | 1,047 | 🟢 OK |
| lsp/server.rs | 1,407 | 🟢 OK |
| editor/textobjects.rs | 707 | 🟢 OK |
| editor/operators.rs | 383 | 🟢 OK |

**Target**: All files <3000 lines (project guideline)

---

## Cross-Module Communication

### **Editor ↔ LSP**
```
Editor::hover()
  → LspManager::hover()
    → LanguageServer::request()
      → JSON-RPC message
        → Language server process
          → Response
            → Cached in Editor::hover_info
              → Rendered in UI
```

### **Editor ↔ Buffer**
```
Editor::delete_range()
  → Buffer::delete_range()
    → Rope::delete()
      → Undo stack updated
        → Editor state dirty flag set
          → UI re-renders
```

### **Editor ↔ UI**
```
Editor state changed
  → render_dirty flag set
    → Event loop detects flag
      → Calls ui::render()
        → Creates ratatui frame
          → Draws to terminal
```

### **Editor ↔ API**
```
API Request
  → event_loop::handle_api_request()
    → Routes to appropriate Editor method
      → Returns response
        → Serialized to JSON
          → Sent to client
```

---

## Module Testing

### Unit Tests (src/*/tests.rs)
- Buffer operations
- Motion calculations
- LSP message parsing
- Session file handling
- Daemon PID verification

### Integration Tests (tests/*.rs)
- Key event sequences
- Editor state transitions
- LSP feature workflows
- API endpoint behavior
- Unicode handling

### Performance Tests
- Large file (>50K lines)
- Many diagnostics
- Rapid key input
- LSP response latency

---

## Migration Status

### ✅ Completed
- UI refactoring (single file → 6 modules)
- Session management (robust with cleanup)
- LSP integration (multi-language)
- Buffer implementation (rope-based)
- LSP integration refactoring (lsp_integration.rs 2,489→851 lines, extracted into lsp_modules/)

### 🟡 In Progress
- input/mod.rs refactoring (needs split)

### 📋 Future
- Plugin system (Lua hooks)
- Custom text objects
- Extended motion library
- Performance improvements

---

**Last Updated**: 2026-01-29
**Module Count**: 40+ modules
**Total Lines**: ~50K lines of Rust code
**Test Coverage**: 55+ unit tests, 40+ integration tests
