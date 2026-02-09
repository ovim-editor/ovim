# ovim Design Philosophy

## Core Principle

**ovim is not a Neovim clone — it's a better alternative with the same muscle memory.**

- **Keep keybindings**: All Vim keybindings work as expected
- **Improve behavior**: When Vim has inconsistent or confusing behavior, ovim chooses the consistent, intuitive option
- **Better UX**: Don't cargo-cult Vim's bugs and quirks

## Design Decisions

### 1. Cursor positioning after number operations

Cursor always positioned on the **last digit** of the modified number.

Vim is inconsistent: hex increment puts cursor on last digit, decimal increment puts cursor on first digit. ovim always uses last digit — consistent, predictable, better muscle memory.

### 2. Number finding: backward then forward

When Ctrl-A/Ctrl-X is pressed and cursor is not on a number, search backward on the current line first, then forward. More forgiving than Vim's forward-only search.

### 3. Octal numbers: explicit prefix required

Require `0o` prefix for octal (unlike Vim's implicit "leading zero = octal"). `007` is decimal 7, not octal. Matches Rust, Python, JavaScript conventions.

### 4. Deliberate Vim divergences

When we diverge from Vim, we document why. The bar is:
- Vim's behavior is **inconsistent** across similar operations, OR
- Vim's behavior is a **historical accident** that can't be fixed due to backwards compatibility

We never diverge on keybindings or core motions. Users should feel at home immediately.

## Anti-patterns we avoid

1. **Cargo-cult programming** — Don't blindly copy Vim behavior without understanding why
2. **Feature completeness over UX** — Fewer features that work intuitively > many features that work inconsistently
3. **Breaking muscle memory** — Never change keybindings or core motion semantics

## Testing philosophy

Tests verify ovim's intended behavior, not Vim's output. If a test expects inconsistent behavior, the test is wrong. See CLAUDE.md testing guidelines for the full policy.

## Architecture

See `PRIORITIES.md` for the architectural roadmap and known structural issues. The key subsystems:

```
Buffer (ovim-core/src/buffer/)
├── Rope-based text storage (ropey)
├── Cursor management
├── ChangeManager (undo/redo stacks)
├── FoldManager
├── Syntax (tree-sitter)
└── File I/O (encoding detection, reload)

Editor (ovim-core/src/editor/)
├── Input handling (normal/, insert_mode.rs, visual_mode.rs, commands.rs)
├── Motions (motions.rs)
├── Operators (operators.rs)
├── RepeatAction (repeat_action.rs) — Pattern B semantic dot-repeat
├── Registers (register.rs)
├── Viewport/scroll (window_viewport.rs, viewport_state.rs)
├── Window management (window.rs, window_manager.rs)
└── LSP integration (lsp_state.rs, ui_features.rs)

UI (ovim/src/ui/)
├── Renderer (ratatui + crossterm)
├── Event loop
└── Key conversion

CLI (ovim/src/)
├── Argument parsing (cli.rs)
├── Subcommand handlers (subcommands.rs)
├── REST API (api/)
└── MCP server (api/mcp.rs)
```

### Undo system (Pattern A vs Pattern B)

Two coexisting patterns, mid-migration:

- **Pattern A**: Manual `Change::insert/delete` + `add_change()`. Used for insert-mode tracking and ex commands.
- **Pattern B**: `buffer.record()` + `RepeatAction`. Gives atomic undo for free and semantic dot-repeat.

See `PRIORITIES.md` item 5 for migration status.

### Buffer state management

Buffer accumulates derived state (highlights, git status, folds, caches) that must be reset on content replacement. Currently no single reset method exists — `reload_from_disk` handles some fields, misses others. See `PRIORITIES.md` item 1.

### Viewport model

Buffer cursor is the source of truth. `update_scroll_offset` runs after every keystroke to keep the viewport centered. Viewport commands (zt, zz, zb, Ctrl-e, Ctrl-y) must reconcile with this model. See `PRIORITIES.md` item 3.
