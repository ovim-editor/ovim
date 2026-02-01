# Crate Split Plan: ovim-core + ovim

## Goal

Split ovim into a workspace with two crates:
- **`ovim-core`** — library crate with all editor logic, no terminal dependencies
- **`ovim`** — binary crate with TUI, event loops, CLI

This enables any Rust frontend (terminal, browser engine, test harness) to embed ovim as a library.

## Current Coupling Points

There are exactly **two external types** leaking into editor core:

| Type | Where | Count |
|------|-------|-------|
| `crossterm::event::KeyEvent` | 25 files in `editor/input/`, `macros.rs`, `api/state.rs` | ~50 import sites |
| `ratatui::layout::Rect` | `render_cache.rs`, `PickerLayout`, `set_last_layout()`, `mouse.rs` | 5 sites |

Plus one method (`render_to_ansi`) that constructs a ratatui terminal inline.

Everything else — buffer, LSP, session, syntax, mode, commands, git, unicode — is already clean.

## Proposed Structure

```
ovim/
├── Cargo.toml          (workspace root)
├── ovim-core/
│   ├── Cargo.toml      (library)
│   └── src/
│       ├── lib.rs
│       ├── key.rs          ← NEW: OvimKeyEvent, OvimKeyCode, OvimModifiers
│       ├── rect.rs         ← NEW: Rect { x, y, width, height }
│       ├── editor/
│       ├── buffer/
│       ├── lsp/
│       ├── api/            (state types, request/response enums)
│       ├── session.rs
│       ├── mode/
│       ├── syntax/
│       ├── commands.rs
│       ├── display.rs
│       ├── git/
│       ├── unicode/
│       ├── language_config/
│       ├── log.rs
│       └── modeline.rs
└── ovim/
    ├── Cargo.toml      (binary, depends on ovim-core)
    └── src/
        ├── main.rs
        ├── event_loop.rs
        ├── ui/             (ratatui renderer)
        ├── cli.rs          (clap definitions)
        ├── client.rs       (HTTP client for subcommands)
        ├── subcommands.rs
        ├── daemon.rs
        ├── mcp_stdio_server.rs
        └── key_convert.rs  ← NEW: From<crossterm::KeyEvent> for OvimKeyEvent
```

## Phase 1: Define Core Input Types (the hard part)

Create `ovim-core::key`:

```rust
// ovim-core/src/key.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: Modifiers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Char(char),
    Enter,
    Esc,
    Backspace,
    Tab,
    BackTab,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
    Null,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Modifiers: u8 {
        const SHIFT   = 0b0001;
        const CONTROL = 0b0010;
        const ALT     = 0b0100;
        const NONE    = 0b0000;
    }
}
```

This covers every key combination used in ovim's input handlers today. It's intentionally smaller than crossterm's KeyEvent (no media keys, no modifier-only events) because ovim doesn't use those.

**Conversion** lives in the binary crate:

```rust
// ovim/src/key_convert.rs
impl From<crossterm::event::KeyEvent> for ovim_core::KeyEvent { ... }
impl From<crossterm::event::KeyCode> for ovim_core::KeyCode { ... }
```

Similarly define `ovim-core::Rect`:

```rust
// ovim-core/src/rect.rs
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}
```

## Phase 2: Mechanical Replacement (biggest diff, lowest risk)

For each of the ~25 input handler files:

```diff
- use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
+ use ovim_core::key::{KeyCode, KeyEvent, Modifiers};
```

And update pattern matches:

```diff
- KeyEvent { code: KeyCode::Char('j'), modifiers: KeyModifiers::NONE, .. }
+ KeyEvent { code: KeyCode::Char('j'), modifiers: Modifiers::NONE }
```

The `.. ` (struct rest pattern) disappears because our KeyEvent has exactly two fields — no `kind` or `state` fields to ignore.

The `KeyModifiers` → `Modifiers` rename is the most pervasive change. Could keep the name `KeyModifiers` to minimize diff, but `Modifiers` is cleaner since it's already namespaced under `key::`.

**MacroManager** (`macros.rs`): Change `Vec<crossterm::event::KeyEvent>` → `Vec<ovim_core::KeyEvent>`. Trivial, 4 lines.

**api/state.rs** `parse_key_string`: Change to produce `ovim_core::KeyEvent` instead of crossterm's. Same logic, different constructor.

## Phase 3: Remove ratatui from Core

1. Replace `ratatui::layout::Rect` with `ovim_core::Rect` in:
   - `render_cache.rs` (1 field)
   - `PickerLayout` in `editor/mod.rs` (3 fields)
   - `set_last_layout()` parameter
   - `rect_contains()` in `mouse.rs`

2. Move `render_to_ansi()` out of `Editor`:
   ```rust
   // ovim/src/ui/ansi_render.rs (binary crate)
   pub fn render_to_ansi(editor: &mut Editor, width: u16, height: u16) -> String { ... }
   ```
   The API endpoint that calls this lives in the binary's event loop anyway.

## Phase 4: Workspace Setup

Convert to workspace:

```toml
# Root Cargo.toml
[workspace]
members = ["ovim-core", "ovim"]
```

Move files physically. Update all `crate::` paths to `ovim_core::` in the core crate. The binary crate uses `use ovim_core::editor::Editor;` etc.

**Dependency split:**

| Dep | ovim-core | ovim (binary) |
|-----|-----------|---------------|
| ropey | ✓ | |
| tree-sitter + grammars | ✓ | |
| lsp-types | ✓ | |
| tokio | ✓ | ✓ |
| axum | | ✓ |
| serde/serde_json | ✓ | ✓ |
| ratatui | | ✓ |
| crossterm | | ✓ |
| clap | | ✓ |
| reqwest | | ✓ |
| mlua (optional) | ✓ | |
| bitflags | ✓ | |
| git2 | ✓ | |
| arboard | ✓ | |
| dirs | ✓ | ✓ |

Note: `axum` stays in the binary because the HTTP server, routes, and handlers are wired into the event loop. The core crate only defines `ApiRequest`/`ApiResponse` enums and the state types — the actual server lives in the binary.

## Execution Order

The phases above are presented for understanding. The actual execution order should minimize broken intermediate states:

1. **Create workspace + ovim-core crate with just `key.rs` and `rect.rs`**
   - Everything still compiles, binary crate unchanged

2. **Move modules to ovim-core one group at a time**, starting with the clean ones:
   - `mode/` → compiles immediately
   - `buffer/` → compiles immediately
   - `unicode/`, `display.rs`, `commands.rs` → compiles immediately
   - `syntax/` → compiles immediately
   - `lsp/` → compiles immediately
   - `session.rs`, `git/`, `language_config/`, `log.rs`, `modeline.rs`

3. **Swap KeyEvent in editor/** — do this as one atomic change:
   - Update all imports in `editor/input/**` to use `ovim_core::key::*`
   - Update `macros.rs`
   - Update `api/state.rs::parse_key_string`
   - The binary crate adds `From` impls and converts at the boundary (event loop)

4. **Swap Rect in editor/** — small follow-up:
   - 5 sites, mechanical

5. **Move `render_to_ansi` to binary crate**

6. **Move remaining modules** (`editor/`, `api/` state types) to ovim-core

7. **Clean up**: update `lib.rs` exports, verify `cargo test`, verify `cargo clippy`

## What NOT to Do

- **Don't introduce a `RenderBackend` trait.** There's no second backend yet. When Ligero is ready, the trait can be extracted from the concrete implementation. Until then, it's speculative abstraction.
- **Don't abstract over async runtimes.** Tokio is fine for both terminal and browser-engine use cases.
- **Don't try to make ovim-core `no_std`.** It uses filesystem, networking, and allocation everywhere. Not a meaningful constraint.
- **Don't split further** (ovim-lsp, ovim-buffer, etc.) yet. Two crates is the right granularity for now. Further splits can happen later if the core crate grows unwieldy.

## Risk Assessment

- **Mechanical risk is low.** The KeyEvent swap is ~50 import changes and pattern match updates across 25 files. No logic changes.
- **Build breakage risk is moderate.** During the move, `crate::` paths all need updating. Doing it module-by-module with compiles between each step keeps this manageable.
- **Behavioral risk is zero.** No logic changes, just type boundaries. The conversion between crossterm's KeyEvent and ovim's is lossless for all keys ovim actually handles.
