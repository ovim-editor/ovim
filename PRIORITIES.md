# Priorities

## Active: Pattern A → B Migration (Undo/Repeat)

Migrating operations from Pattern A (manual `Change::delete` + `add_change`) to Pattern B (`record_operation()` + `RepeatAction`). Pattern B gives atomic undo for free and semantic dot-repeat that re-evaluates at cursor position.

### Done

- [x] Infrastructure: `Edit`, `buffer.record()`, `record_operation()` helper
- [x] `RepeatAction` enum for semantic dot-repeat
- [x] J/gJ, >>, <<, ~, Ctrl-A/Ctrl-X (indent, dedent, toggle case, number ops)
- [x] dd, D/d$, dw, dj, dk, d}, d{ (line/word/paragraph deletes)
- [x] p, P (paste after/before)
- [x] x, X (delete char forward/backward)
- [x] df, dt, dF, dT (char motion deletes)
- [x] diw, daw, di", da", di(, da(, etc. (text object deletes)
- [x] dG, dgg (delete to first/last line)
- [x] d% (delete to matching bracket)
- [x] r (replace character)
- [x] Change operators: cc, C/c$, s, S, cj, ck, c}, c{, cG, cgg (`RepeatAction::Change` with `PendingChangeRepeat`)

### Next up

#### 1. Visual delete (smallest, self-contained)

Undo already works (Pattern A). Just needs a RepeatAction with selection geometry:

```rust
RepeatAction::VisualDelete {
    mode: VisualMode,  // Char, Line, Block
    lines: usize,
    cols: usize,
}
```

`execute()` computes the deletion range from stored dimensions at current cursor. No interaction with the change-building system.

#### 2. Undo/redo for LSP rename and code actions

LSP rename (`ui_features.rs`) and code actions apply edits via `add_change` but may not integrate well with the undo system. Investigate whether multi-file edits, workspace edits, and code action changes produce correct undo behavior. If not, wrap them in `record()` for atomic undo.

#### 3. Ex commands — leave alone

`:d`, `:sort`, `:g`, `:s`, `:r`, `:t`, `:m`, `:!` don't use `.` (Vim uses `@:` for ex-command repeat). Undo already works via composites. Different world, not worth migrating.

### Remaining `add_change` callsites (~48)

| Area | Count | Notes |
|------|-------|-------|
| `operators.rs` (change ops: cc, c$, cj, ck, c}, c{, cG, cgg) | 9 | ✅ Done — migrated to PendingChangeRepeat |
| `editing_commands.rs` (C, s, S) | 3 | ✅ Done — migrated to PendingChangeRepeat |
| `helpers.rs` (visual delete, indent/dedent tracking, o/O) | 14 | Visual delete next; o/O needs RepeatAction::OpenLine |
| `commands.rs` (ex commands: :d, :sort, :g, :s, :r, :t, :m) | 7 | Leave alone (Vim uses `@:`) |
| `text_objects.rs` (change text objects: ci", ca(, etc.) | 2 | Already have semantic repeat |
| `char_motion.rs` (cf, ct with change operator) | 1 | Wire up PendingChangeRepeat |
| `insert_mode.rs` (insert-mode change tracking) | 4 | Core infrastructure, stays |
| `replace_mode.rs` (R replace mode) | 1 | Evaluate separately |
| `ui_features.rs` (LSP rename, code actions) | 3 | Investigate undo behavior |
| `mod.rs` (undo/redo internals, add_change definition) | 3 | Infrastructure, stays |
