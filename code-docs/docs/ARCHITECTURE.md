# ovim Architecture Overview

This document describes the high-level architecture of ovim and how its major components interact.

## 🏗️ System Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         ovim System                             │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌──────────────────┐          ┌──────────────────┐           │
│  │   TUI Mode       │          │  Headless Mode   │           │
│  │  (ratatui)       │          │  (REST API)      │           │
│  └────────┬─────────┘          └────────┬─────────┘           │
│           │                             │                      │
│           ├─────────────┬───────────────┘                      │
│           │             │                                      │
│           v             v                                      │
│    ┌─────────────────────────┐                                │
│    │   Input Handler         │  ← KeyEvents / API Requests   │
│    │   (key parsing)         │                                │
│    └───────────┬─────────────┘                                │
│               │                                               │
│               v                                               │
│    ┌─────────────────────────┐                                │
│    │   Editor Core           │                                │
│    │ ┌─────────────────────┐ │                                │
│    │ │ Operations          │ │  (d, c, y, >, <, etc)        │
│    │ │ Motions             │ │  (w, e, b, j, k, etc)        │
│    │ │ Text Objects        │ │  (iw, aw, i", a", etc)       │
│    │ │ Commands            │ │  (:w, :set, :e, etc)        │
│    │ └─────────────────────┘ │                                │
│    └───────────┬─────────────┘                                │
│               │                                               │
│    ┌──────────┴──────────┐                                    │
│    │                     │                                    │
│    v                     v                                    │
│ ┌─────────┐          ┌─────────────┐                          │
│ │ Buffer  │          │ LSP Manager │                          │
│ │ (Rope)  │          │             │                          │
│ └────┬────┘          └──────┬──────┘                          │
│      │                      │                                 │
│      │         ┌────────────┼────────────┐                    │
│      │         │            │            │                    │
│      v         v            v            v                    │
│ ┌──────────────────────────────────────────────┐             │
│ │        State & Persistence                   │             │
│ │ ┌──────────┐ ┌──────────┐ ┌──────────────┐ │             │
│ │ │ Buffer   │ │ Cursor   │ │ Diagnostics  │ │             │
│ │ │ Content  │ │ Position │ │ Marks, Regs  │ │             │
│ │ └──────────┘ └──────────┘ └──────────────┘ │             │
│ └──────────────────────────────────────────────┘             │
│      │                                                       │
│      └──────────────────┬───────────────────────┐            │
│                         │                       │            │
│    ┌────────────────────v──────────────────┐   │            │
│    │        Session Manager               │   │            │
│    │ (persistence, discovery)             │   │            │
│    └────────────────────────────────────────┘   │            │
│                                                 │            │
│                                  ┌──────────────v────┐       │
│                                  │  Terminal UI      │       │
│                                  │  (Rendering)      │       │
│                                  └───────────────────┘       │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

---

## 🔄 Data Flow

### 1. **Input Processing Pipeline**

```
KeyEvent / API Request
    ↓
Input Handler (src/editor/input/mod.rs)
  - Parse key modifiers (Ctrl, Shift, Alt)
  - Detect mode (Normal, Insert, Replace, Visual, etc)
  - Buffer pending counts (e.g., "10dw")
    ↓
Editor Operations (src/editor/mod.rs)
  - Apply operators (delete, yank, change, etc)
  - Move cursor (motions)
  - Modify selections (visual mode)
    ↓
State Changes
  - Buffer modified
  - Cursor moved
  - Selection changed
    ↓
LSP Actions (if applicable)
  - Flush pending changes
  - Request hover/goto/completion
  - Handle diagnostics
    ↓
Render
  - Update UI (TUI) or return API response (Headless)
```

### 2. **LSP Integration Pipeline**

```
LSP Manager (src/lsp/mod.rs)
  - Coordinates multiple language servers
  - Manages per-language LanguageServer instances
    ↓
Language Server (src/lsp/server.rs)
  - Spawns LSP subprocess
  - Sends initialize request
  - Listens for notifications/responses
    ↓
Request/Response Handler
  - Sends textDocument/hover, etc
  - Receives responses
  - Caches results (hover, completion, etc)
    ↓
Notification Handler
  - Receives publishDiagnostics
  - Stores diagnostics in editor
  - Updates UI
    ↓
UI Rendering
  - Display gutter signs
  - Underline diagnostics
  - Show hover windows
  - Completion popups
```

---

## 🧩 Major Components

### **Editor Core** (`src/editor/`)
- **Main responsibility**: Core editing logic
- **Key files**:
  - `mod.rs` - Editor state, main operations
  - `input/mod.rs` - Key event handling (5824 lines, needs refactoring)
  - `operators.rs` - Delete, yank, change, case, etc
  - `motions.rs` - Word, line, paragraph movements
  - `textobjects.rs` - Inner/around text objects
  - `commands.rs` - Ex commands (`:w`, `:set`, etc)
  - `lsp_integration.rs` - LSP core: init, polling, document sync, action dispatcher
  - `lsp_modules/` - LSP feature implementations (hover, goto, diagnostics, completion, actions, references, workspace edits)
- **Exports**: `Editor` struct, operation functions

### **Buffer** (`src/buffer/`)
- **Main responsibility**: Text storage and manipulation
- **Key files**:
  - `mod.rs` - Buffer struct with rope
- **Implementation**: Uses ropey crate for efficient text operations
- **Features**:
  - Efficient for large files (50K+ lines)
  - Rope-based O(log n) operations
  - UTF-8 aware
  - UTF-16 conversion for LSP

### **LSP System** (`src/lsp/`)
- **Main responsibility**: Language Server Protocol integration
- **Key files**:
  - `mod.rs` - LspManager (coordinator)
  - `server.rs` - LanguageServer (per-language)
  - `supervisor.rs` - Multi-server management
  - `protocol.rs` - JSON-RPC message handling
  - `logger.rs` - Request/response logging
- **Features**:
  - Multi-language support (Rust, Python, JS, Java)
  - Non-blocking async with Tokio
  - Debounced text changes (150ms)
  - Hover, goto, completion, diagnostics, rename

### **UI System** (`src/ui/`)
- **Main responsibility**: Terminal rendering
- **Key files**:
  - `renderer/mod.rs` - Main rendering orchestration
  - `renderer/core.rs` - Frame rendering
  - `renderer/buffer.rs` - Buffer content + syntax highlighting
  - `renderer/widgets.rs` - Picker, hover, completion, etc
  - `renderer/helpers.rs` - Text utilities
  - `renderer/styles.rs` - Color/style generation
  - `ansi.rs` - ANSI export for headless mode
- **Framework**: ratatui for TUI, crossterm for terminal control
- **Features**:
  - Syntax highlighting
  - Git gutter signs
  - Diagnostics rendering
  - Picker/fuzzy finder
  - Multiple widgets (hover, completion, file tree)

### **Session Management** (`src/session.rs`)
- **Main responsibility**: Multi-session support and persistence
- **Features**:
  - Session file storage (JSON)
  - Atomic writes (write-to-temp-then-rename)
  - PID verification to prevent stale sessions
  - Auto-cleanup on exit (SIGINT, SIGTERM, panic)
  - Session discovery via ovim-ctl
- **File location**: `~/.cache/ovim/sessions/` (Linux) or `~/Library/Caches/ovim/sessions/` (macOS)

### **API System** (`src/api/`)
- **Main responsibility**: REST API for headless mode
- **Framework**: Axum web server
- **Endpoints**:
  - `/snapshot` - Complete editor state
  - `/keys` - Send keystrokes
  - `/buffer` - Get/set buffer content
  - `/cursor` - Cursor position
  - `/mode` - Current mode
  - `/command` - Execute ex command
  - `/lsp/status` - Server states
  - `/health` - Health check
  - `/metrics` - Performance metrics
- **Port**: Randomly assigned (sent via session file)

### **Configuration** (`src/config/`)
- **Main responsibility**: User configuration
- **Format**: Lua (via mlua)
- **Location**: `~/.config/ovim/init.lua`
- **Features**:
  - vim.opt (options)
  - vim.keymap (key mappings)
  - Custom commands
  - Plugin system (future)

---

## 🎯 Design Patterns

### **1. State Management**
- All state centralized in `Editor` struct
- Main thread owns editor (single-threaded)
- Background tasks (LSP, API) communicate via channels
- No global mutable state

### **2. Async Patterns**
- Tokio for async runtime
- `tokio::sync::Mutex` for shared state
- Channels for cross-thread communication
- Background tasks isolated from main loop

### **3. Error Handling**
- `Result<T, Error>` throughout
- `anyhow` crate for error context
- Graceful degradation (LSP errors don't crash editor)
- Error logging via eprintln! to stderr

### **4. Memory Safety**
- Rope data structure prevents buffer copying
- Clone-on-write for change history
- Drop guards for resource cleanup
- Owned data (no lifetimes required)

### **5. Modularity**
- Clear module boundaries
- Public APIs only where needed
- Private implementation details
- Refactoring in progress (input/mod.rs too large)

---

## 🔄 Event Loop

### **TUI Mode Event Loop** (`src/main.rs:tui_main`)
```rust
loop {
  timeout(50ms)
    - Check for terminal resize
    - Check for keyboard input
    - Check for LSP responses
    - Process pending actions

  if input_available:
    - Parse key event
    - Call Editor::handle_key()
    - Update editor state
    - Enqueue LSP actions if needed

  if lsp_response_ready:
    - Process LSP response
    - Update editor state
    - Enqueue UI refresh

  if state_changed:
    - Render to terminal
    - Update display
    - Draw cursor
}
```

### **Headless Mode Event Loop** (`src/main.rs:headless_main`)
```rust
loop {
  timeout(50ms for API requests)
    - Check for API request
    - Check for LSP response
    - Process pending actions

  if api_request_available:
    - Route to handler
    - Call appropriate Editor method
    - Enqueue LSP actions if needed
    - Send response back to client

  if lsp_response_ready:
    - Process LSP response
    - Update editor state
    - Enqueue response to pending request if applicable

  if shutdown_signal:
    - Cleanup session file
    - Exit
}
```

---

## 📊 State Management

### **Editor State** (main thread)
```rust
pub struct Editor {
  buffer: Buffer,              // Text content (rope)
  cursor: (usize, usize),      // (line, col)
  mode: Mode,                  // Normal, Insert, Visual, etc
  registers: HashMap<char, String>,
  marks: HashMap<char, (usize, usize)>,
  undo_stack: Vec<Change>,     // Undo history
  redo_stack: Vec<Change>,     // Redo history

  // LSP
  lsp_manager: LspManager,
  diagnostics: HashMap<usize, Vec<Diagnostic>>,
  hover_info: Option<String>,

  // UI
  should_quit: bool,
  render_dirty: bool,
}
```

### **LSP State** (background task)
```rust
pub struct LspManager {
  servers: DashMap<Language, LanguageServer>,
  pending_requests: HashMap<RequestId, PendingRequest>,
}

pub struct LanguageServer {
  process: Child,
  stdin: Sender,
  receiver: Receiver,
  capabilities: ServerCapabilities,
  initialized: bool,
}
```

---

## 🚀 Performance Optimizations

### **1. Render Dirty Flag**
- Only re-render when state changes
- Reduces CPU usage at idle to near-zero

### **2. Large File Detection**
- Disables syntax highlighting for >50K lines
- Disables git gutter for >5MB files
- Still responsive for huge files

### **3. LSP Debouncing**
- 150ms debounce on text changes
- Reduces server load during typing
- Flush before hover/goto for accurate results

### **4. Buffer Rope**
- O(log n) insert/delete operations
- No full buffer reallocation
- Efficient even for 1MB+ files

### **5. Async Non-blocking**
- Main event loop never blocks on LSP
- API requests don't block editor
- Background tasks isolated

---

## 🔗 Integration Points

### **1. Custom Keybindings**
```lua
-- ~/.config/ovim/init.lua
vim.keymap.set('n', '<leader>f', ':Picker<CR>')
vim.keymap.set('n', 'K', function() vim.lsp.buf.hover() end)
```

### **2. Plugin System** (Future)
- Lua callbacks for custom behavior
- Hooks for editor events
- Custom commands

### **3. External Tools**
- Language servers (rust-analyzer, pyright, etc)
- Git integration (for gutter)
- File pickers (fzf, ripgrep)

---

## 📈 Scalability

### **What scales well:**
- Large files (gigabytes possible with rope)
- Many LSP servers (DashMap handles concurrent access)
- Many sessions (session files are lightweight)
- High keyboard input rate (non-blocking processing)

### **What has limits:**
- Undo stack (unbounded, could add limits)
- Visible regions (terminal width/height)
- Diagnostics per file (scales with file size)
- Number of marks/registers (fixed, minimal memory)

---

## 🔒 Safety & Reliability

### **Crash Safety**
- SessionGuard ensures cleanup on panic
- SIGINT/SIGTERM handlers ensure cleanup on kill
- Session files survive crashes

### **Data Integrity**
- Atomic writes with fsync for durability
- UTF-8 validation on file load
- Rope prevents data corruption
- Change history for undo/redo

### **Security**
- Session files have restrictive permissions (0o600)
- No TOCTOU races in file operations
- Input validation for API requests
- Sandbox not required (single-user tool)

---

## 🎓 Learning Path

**New contributor?** Follow this learning path:

1. **Understand the structure**
   - Read this file
   - Explore [MODULES.md](./MODULES.md)

2. **Understand key systems**
   - Start with Buffer ([BUFFER_SYSTEM.md](./BUFFER_SYSTEM.md))
   - Then Editor ([EDITOR_SYSTEM.md](./EDITOR_SYSTEM.md))
   - Then LSP ([LSP_SYSTEM.md](./LSP_SYSTEM.md))
   - Then UI ([UI_SYSTEM.md](./UI_SYSTEM.md))

3. **Understand data flow**
   - Run with `RUST_LOG=debug`
   - Set breakpoints in event loop
   - Trace a simple operation (e.g., typing 'a')

4. **Make a change**
   - See [EXTENDING.md](./EXTENDING.md)
   - Start with a simple fix
   - Add tests

---

**Last Updated**: 2026-01-29
**Architecture Status**: Stable
**Test Coverage**: 55/55 unit tests passing ✅
