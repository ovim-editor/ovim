# Priorities

## Architectural Roadmap

Six structural problems emerged from a systematic bug hunt (48 issues, OV-00063–OV-00110). These aren't independent bugs — they cluster around architectural gaps. Fixing them individually creates whack-a-mole; fixing the structures prevents entire categories of bugs.

Ordered by impact and dependency. Earlier items unblock later ones.

---

### 1. Buffer content replacement (reload problem) — DONE

`reset_derived_state()` implemented: resets ChangeManager, folds, git state, highlight caches, version counter. `reload_from_disk` and `reload_if_changed` both go through it. LSP notified via `mark_buffer_modified_force_send()`.

**Issues resolved:** OV-00099, OV-00100, OV-00103, OV-00104, OV-00105, OV-00106, OV-00107, OV-00108, OV-00109

---

### 2. Undo stack safety (transaction-based manipulation) — DONE

Token-based approach implemented: delete phase returns a handle, merge phase redeems it. Empty edits handled safely.

**Issues resolved:** OV-00063, OV-00064, OV-00065

---

### 3. Viewport/scroll unification — DONE

`viewport_command_active` removed. Ctrl-e/Ctrl-y now move buffer cursor. Window-level heights used for scroll. Sidescrolloff clamped.

**Issues resolved:** OV-00075, OV-00076, OV-00077, OV-00078, OV-00079

---

### 4. Motion contract enforcement — DONE

All motion bugs fixed individually across multiple commits.

**Issues resolved:** OV-00080, OV-00081, OV-00082, OV-00083, OV-00084, OV-00085, OV-00087, OV-00088, OV-00089 (OV-00086 remains: `%` forward-search + `<`/`>`)

---

### 5. Pattern A → B migration (undo/repeat)

Migrating operations from Pattern A (manual `Change::delete` + `add_change`) to Pattern B (`record_operation()` + `RepeatAction`). Pattern B gives atomic undo for free and semantic dot-repeat that re-evaluates at cursor position.

**Should complete item 2 (undo stack safety) first** to avoid introducing more boundary bugs at the Pattern A/B interface.

#### Done

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

#### Current state (updated)

- [x] `cf/ct` with change operator now uses `PendingChangeRepeat` in `char_motion.rs`.
- [x] Visual delete undo path uses `record()` + `push_recorded_undo()` in `helpers.rs`.
- [x] Visual delete dot-repeat now uses `RepeatAction` across char/line/block selections.
- [x] `o/O` now use `RepeatAction::OpenLine`; legacy `Change::Composite` open-line repeat fallback removed.
- [x] LSP/workspace text edits now record undo entries per edited buffer (current + non-current) without polluting dot-repeat templates.
- [x] Visual block change dot-repeat (`Ctrl-V ... c ... .`) now uses semantic repeat geometry with active regression coverage.
- [x] LSP workspace `ResourceOp` (create/rename/delete) now snapshots filesystem state and integrates with undo/redo.
- [x] Substitute-confirm (`:s/.../.../c`) now records each confirmed replacement as a single recorded undo unit.
- [x] Text-object changes (`ciw`, `ca"`, etc.) now use `PendingChangeRepeat` + `RepeatAction::Change` instead of legacy pending semantic change path.
- [x] Completion accept path now records undo via `record()` + `push_recorded_undo()` instead of manual composite `add_change`.
- [x] Text-object operator handlers now require concrete `TextObjectType`; dead fallback `add_change` path for change-operator text objects removed.
- [x] Added macro regression coverage for text-object change repeat/undo granularity (`ci(` + `.` + `u`) in `dot_repeat_test`.
- [x] Text-object case operators (`gu/gU/g~` + text objects) migrated to recorded undo + `RepeatAction` semantic dot-repeat.
- [x] Added macro regression coverage for semantic `guiw` dot-repeat + undo granularity.
- [x] `:global ... d` delete path now records undo via `record()` + `push_recorded_undo()` instead of manual composite `add_change`.
- [x] Added macro regression coverage for `:global` delete undo/redo flow.
- [x] Ranged Ex delete (`:1,3d` / `:delete`) now records undo via `record()` + `push_recorded_undo()`.
- [x] Added macro regression coverage for ranged delete undo/redo flow.
- [x] Shell filter path (`:%!cmd` / `:.!cmd`) now records undo via `record()` + `push_recorded_undo()`.
- [x] Added macro regression coverage for shell filter undo/redo flow (`:%!sort`, `u`, `<C-r>`).
- [x] Remaining Ex command mutation paths in `commands.rs` (`:r !cmd`, `:sort`, `:copy`, `:move`) now record undo via `Edit` entries + `push_recorded_undo()`.
- [x] Added macro regression coverage for `:sort`, `:copy`, and `:move` undo/redo flows.
- [x] Insert-mode helper operations (`Ctrl-W/U/T/D`) now use `apply_change_and_record()` instead of manual buffer mutation + `add_change`.
- [x] Added macro regression coverage for `Ctrl-W/U/T/D` insert-mode undo/redo flows.

#### Remaining `add_change` callsites (current snapshot: 10 in `ovim-core/src`)

| Area | Count | Notes |
|------|-------|-------|
| `input/insert_mode.rs` | 4 | Core insert-mode batching and semantic change finalization; intentional |
| `editor/mod.rs` | 3 | Infrastructure (`apply_change_and_record`, wrapper methods) |
| `change.rs` | 2 | ChangeManager internals (`add_change` implementation/docs) |
| `input/replace_mode.rs` | 1 | Replace-mode tracking |

#### Practical migration targets

1. No open Pattern A→B migration blockers remain; remaining `add_change` callsites are intentional or infrastructural.

---

### 6. Register system type fidelity — DONE

All three register bugs fixed: `delete_history` is `Vec<RegisterContent>`, named register ops update unnamed, uppercase append updates type.

**Issues resolved:** OV-00094, OV-00095, OV-00096

---

### 7. Indentation option wiring — DONE

All indentation bugs fixed: expandtab/shiftwidth consulted, empty lines skipped, cursor positioned correctly.

**Issues resolved:** OV-00066, OV-00067, OV-00068, OV-00069, OV-00070, OV-00071, OV-00072, OV-00073, OV-00074

---

### 8. Command dispatch consolidation — DONE

`:e`/`:e!` consolidated in commands.rs with unsaved-changes check, tilde expansion, and force-reload support. No duplicate handler in input/commands.rs.

**Issues resolved:** OV-00101, OV-00102, OV-00110

---

### 9. Paste behavior fixes — DONE

All paste bugs fixed: count implemented, P cursor corrected, visual paste updates unnamed register, visual-line uses paste_before.

**Issues resolved:** OV-00090, OV-00091, OV-00092, OV-00093, OV-00097, OV-00098

---

## Status

Items 1 through 9 are **DONE**.
