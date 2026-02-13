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

#### Next up

1. **Visual delete** — undo works (Pattern A). Just needs RepeatAction with selection geometry.
2. **cf/ct with change operator** — wire up PendingChangeRepeat in `char_motion.rs`.
3. **o/O** — no delete phase; needs `RepeatAction::OpenLine`.
4. **LSP rename/code actions** — investigate undo behavior, wrap in `record()` if broken.

#### Remaining `add_change` callsites (~48)

| Area | Count | Notes |
|------|-------|-------|
| `helpers.rs` (visual delete, indent/dedent tracking, o/O) | 14 | Visual delete next; o/O needs RepeatAction::OpenLine |
| `commands.rs` (ex commands: :d, :sort, :g, :s, :r, :t, :m) | 7 | Leave alone (Vim uses `@:`) |
| `text_objects.rs` (change text objects: ci", ca(, etc.) | 2 | Already have semantic repeat |
| `char_motion.rs` (cf, ct with change operator) | 1 | Wire up PendingChangeRepeat |
| `insert_mode.rs` (insert-mode change tracking) | 4 | Core infrastructure, stays |
| `replace_mode.rs` (R replace mode) | 1 | Evaluate separately |
| `ui_features.rs` (LSP rename, code actions) | 3 | Investigate undo behavior |
| `mod.rs` (undo/redo internals, add_change definition) | 3 | Infrastructure, stays |

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

Items 1, 2, 3, 4, 6, 7, 8, 9 are **DONE**. Remaining work:

- **Item 5** (Pattern A→B migration): Ongoing, depends on item 2 (done)
