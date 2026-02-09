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

#### 2. Change operators — semantic dot-repeat (medium effort, high value)

**Key insight: undo already works.** The ChangeBuilder accumulates all changes (delete + insert-mode typing) into one `Change::Composite`. Single undo is correct today. The problem is only dot-repeat — it replays raw position-dependent changes instead of re-evaluating semantically.

**What already works:** `cw`, `ci"`, `cgn` have semantic repeat via `PendingSemanticChange`. On Esc, `exit_insert_mode()` pops the composite, extracts inserted text, creates a semantic `Change` variant (`ChangeWord`, `ChangeTextObject`, `ChangeSearchMatch`).

**What doesn't:** `cc`, `C`, `cj`, `ck`, `c}`, `c{`, `s`, `S` don't set `PendingSemanticChange`. They rely on raw composite replay.

**Approach — `RepeatAction::Change`:**

```rust
RepeatAction::Change {
    delete: Box<RepeatAction>,  // reuses existing variants (DeleteLines, DeleteWordForward, etc.)
    inserted_text: String,       // captured on Esc
}
```

The flow:
1. Change operator sets a `PendingChangeRepeat(RepeatAction)` before entering insert mode (e.g., `cc` stores `DeleteLines { count }`)
2. Undo path unchanged — ChangeBuilder composite stays on the undo stack
3. On Esc, `exit_insert_mode()` reads the pending repeat, captures inserted text, creates `RepeatAction::Change { delete, inserted_text }`
4. On `.`: execute the delete (semantic, re-evaluated at cursor), insert the captured text, wrap in `record_operation()` for atomic undo. No insert mode entry during replay.

This decouples undo (ChangeBuilder, untouched) from repeat (RepeatAction, new). About a dozen callsites to wire up, but the infrastructure is a small `RepeatAction` variant + a field on the editor.

#### 3. Ex commands — leave alone

`:d`, `:sort`, `:g`, `:s`, `:r`, `:t`, `:m`, `:!` don't use `.` (Vim uses `@:` for ex-command repeat). Undo already works via composites. Different world, not worth migrating.

### Remaining `add_change` callsites (~48)

| Area | Count | Notes |
|------|-------|-------|
| `operators.rs` (change ops: cc, cw, c$, cj, ck, c}, c{, cG, cgg) | 9 | Phase 2: wire up PendingChangeRepeat |
| `helpers.rs` (visual delete, indent/dedent tracking, s/S/C, o/O) | 14 | Phase 1 (visual) + Phase 2 (s/S/C/o/O) |
| `commands.rs` (ex commands: :d, :sort, :g, :s, :r, :t, :m) | 7 | Phase 3: leave alone |
| `text_objects.rs` (change text objects: ci", ca(, etc.) | 2 | Already have semantic repeat |
| `char_motion.rs` (cf, ct with change operator) | 1 | Phase 2: wire up PendingChangeRepeat |
| `insert_mode.rs` (insert-mode change tracking) | 4 | Core infrastructure, stays |
| `editing_commands.rs` (s, S line substitute) | 2 | Phase 2: wire up PendingChangeRepeat |
| `replace_mode.rs` (R replace mode) | 1 | Phase 2: evaluate separately |
| `ui_features.rs` (LSP rename, code actions) | 3 | Leave alone (LSP-driven) |
| `mod.rs` (undo/redo internals, add_change definition) | 3 | Infrastructure, stays |
