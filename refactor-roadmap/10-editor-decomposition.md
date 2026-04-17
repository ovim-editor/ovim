# 10: Editor Struct Decomposition (BACKGROUND)

> **No dedicated roadmap.** `LspSubsystem` established the pattern — follow it when touching other areas. See the overview for context.

**Goal:** Break the `Editor` god struct into focused subsystems so that each concern has a clear owner and adding features doesn't grow a single 3k-line file.

**Fixes:** Maintainability. No user-facing bugs, but reduces the cognitive overhead of every change touching the same type.

**Risk:** Low per-step if done as extract-struct refactors (no behavior change). High if done all at once.

## The Problem

`Editor` is the central struct (~2,300 lines in `mod.rs`, 38 `impl Editor` blocks across files) with every piece of state hung off it:

- Buffer management (buffers, current_buffer_index, tab list)
- LSP subsystem (already extracted to `LspSubsystem` — good)
- UI panels (file tree, quickfix, location list, path completion)
- Navigation (marks, jump list, search state)
- Input state (mode, pending operator, repeat action, macros)
- AI/chat state
- Debug/DAP state
- Decorations
- Options/configuration
- Clipboard/registers

`impl Editor` blocks span 20+ files. Each file adds methods to the same type. Finding "what can modify the decoration map" requires grepping across all of them.

## What's Already Done Right

`LspSubsystem` is a good example of extraction done well:
- Groups `LspState`, `LspSlots`, `LspIntents`, channels, UI panel state
- Accessed via `self.lsp.*`
- Has a clear boundary: LSP concerns stay in the subsystem

## The Design

Extract subsystems the same way `LspSubsystem` was done. Each subsystem is a struct field on `Editor`, accessed via `self.subsystem.*`. No trait abstractions, no dependency injection — just grouped fields.

### Priority order

1. **UI Panels** — `file_tree`, `quickfix_list`, `location_list`, `path_completion`, `substitute_confirm` → `UiPanels` (partially done — `ui_panels` exists but could absorb more)

2. **Navigation** — `marks`, `jump_list`, `search_state`, `last_search_direction` → `NavigationState` (partially done — `nav` exists)

3. **Input/Mode** — `mode`, `pending_operator`, `count_prefix`, `input_state`, `macro_manager`, `repeat_action`, `keymap_manager` → already partially grouped, could be tighter

4. **Decorations** — `decorations` field is already standalone (`DecorationMap`), but the methods that create/update decorations are scattered across `lsp_integration.rs`, `change_tracking.rs`, and `ui_features.rs`

### Non-goals

- Don't introduce traits or abstractions — this is a data organization change
- Don't move methods that legitimately need cross-subsystem access (e.g., undo needs both buffer and decorations)
- Don't force everything into subsystems — some state is genuinely editor-wide

## When to Do This

This is background cleanup, not blocking any feature. Good candidate for incremental work alongside other changes — extract one subsystem per PR rather than a big-bang refactor.
