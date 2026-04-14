# Phase 6: LspState Decomposition

**Goal:** Break the 33-field `LspState` into focused subsystems that each manage their own lifecycle.

**Fixes:** Maintainability. Reduces cognitive load when working on any single LSP feature.

**Risk:** Low. Mechanical refactor -- moving fields into sub-structs and updating access paths. No behavioral change. The compiler verifies correctness.

## The Problem

`LspState` is a flat struct where hover fields, completion fields, diagnostic fields, and navigation fields all sit next to each other with no encapsulation. Every method that touches hover also has access to diagnostics. There's no way to reason about one subsystem in isolation.

## The Decomposition

```rust
pub struct LspState {
    pub connection: LspConnection,
    pub sync: DocumentSync,
    pub hover: HoverState,
    pub completion: CompletionState,
    pub diagnostics: DiagnosticsState,
    pub inlay_hints: InlayHintState,
    pub navigation: NavigationState,
    pub status: String,
}
```

Each sub-struct owns its fields and exposes focused methods. See the overview for the full field mapping.

## When To Do This

This can happen at any time since it's a pure refactor. However, it's most useful *after* Phases 2-3, because those phases change which fields exist and how they're used. Decomposing first and then changing the fields means touching the sub-structs twice.

Recommended: do this after Phase 3, when `DocumentSyncState` has been simplified and the action dispatch has been restructured. The decomposition then reflects the cleaned-up state.

## Migration

The compiler drives this. Move fields into sub-structs. Fix every access error. Run `cargo test`. Done.

Do it one subsystem at a time: extract `HoverState` first, fix all accesses, run tests. Then `CompletionState`. And so on.

## Files Changed

All files that access `editor.lsp.state.*` -- roughly 10-15 files with ~100 access sites total.
