# Priorities

## Architectural Roadmap

Six structural problems emerged from a systematic bug hunt (48 issues, OV-00063–OV-00110). These aren't independent bugs — they cluster around architectural gaps. Fixing them individually creates whack-a-mole; fixing the structures prevents entire categories of bugs.

Ordered by impact and dependency. Earlier items unblock later ones.

---

### 1. Buffer content replacement (reload problem)

**Problem:** Buffer holds ~20 independently-managed fields (rope, cursor, change_manager, fold_manager, git_status, git_blame, cached_highlights, semantic_highlights, code_block_cache, syntax, version, encoding, line_ending, file_mtime, modified, pending_rehighlight...). `reload_from_disk` replaces the rope and touches *some* of these. It misses undo history, folds, git state, highlight caches, version counter, LSP sync — 7 bugs from one root cause.

**Fix:** `Buffer::replace_content(rope, source)` that resets all derived state. Either group derived state into a sub-struct that gets wholesale replaced, or construct a fresh buffer and transplant identity fields (file_path, window assignment). Make `reload_from_disk`, `reload_if_changed`, and any future content-replacement operations go through this single method.

**Issues resolved:** OV-00099, OV-00100, OV-00103, OV-00104, OV-00105, OV-00106, OV-00107, OV-00108, OV-00109

**Depends on:** Nothing. Self-contained.

---

### 2. Undo stack safety (transaction-based manipulation)

**Problem:** The undo stack is `Vec<Change>` with convention-based positional access. `exit_insert_mode` does two `pop_last_change()` calls assuming a specific stack layout (insert composite on top, delete Recorded below). When the delete phase produces no edits, the second pop grabs an unrelated earlier change. `save_point` (a stack index) becomes invalid after pops. The dual undo paths (`Buffer::undo` vs `ChangeManager::undo`) can diverge.

**Fix:** Replace blind pops with transaction tokens. The delete phase returns a handle; the merge phase redeems it. If the handle is empty (no edits), the merge adjusts. `save_point` should be a generation counter, not an index.

**Issues resolved:** OV-00063, OV-00064, OV-00065

**Depends on:** Nothing. Self-contained. Should complete before more Pattern A→B migration to avoid introducing more boundary bugs.

---

### 3. Viewport/scroll unification

**Problem:** Two sources of truth for "where the user is looking": window scroll offset and buffer cursor. Ctrl-e/Ctrl-y update window scroll without moving the buffer cursor. `update_scroll_offset` reads the buffer cursor and recalculates, undoing the viewport command's scroll. The `viewport_command_active` flag was a patch to suppress this — but it's never cleared, permanently disabling scrolloff.

**Fix:** Viewport scroll commands (Ctrl-e, Ctrl-y) must also move the buffer cursor when necessary (Vim does this — Ctrl-e moves cursor down if it would go above viewport). Remove `viewport_command_active` entirely. `update_scroll_offset` should use the focused window's height, not the editor-level `viewport_height` (fixes splits too). Clamp `sidescrolloff` the same way `scrolloff` is clamped.

**Issues resolved:** OV-00075, OV-00076, OV-00077, OV-00078, OV-00079

**Depends on:** Nothing. Self-contained.

---

### 4. Motion contract enforcement

**Problem:** 10 motion bugs from independent implementations with no shared boundary rules. G/gg don't clamp. `b` from col 0 lands wrong. `ge` moves forward. `$` and `_` ignore count. `+` uses wrong line count. `%` doesn't search forward. `t` returns success without moving. `]}` matches cursor char.

**Fix:** Define explicit contracts:
- All motions must leave cursor on valid buffer coordinates (line < line_count, col < line_len)
- Backward motions must not increase cursor position; forward motions must not decrease it (or return false)
- Count means N-1 lines down for line-targeting motions ($, _)
- Forward-searching motions skip the character under cursor

Enforce via a test harness that asserts these properties after every motion call, or via a `MotionResult` type that encodes success/failure and new position.

**Issues resolved:** OV-00080, OV-00081, OV-00082, OV-00083, OV-00084, OV-00085, OV-00086, OV-00087, OV-00088, OV-00089

**Depends on:** Nothing. Can be done incrementally per-motion.

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

### 6. Register system type fidelity

**Problem:** `delete_history` is `Vec<String>` — numbered registers (1-9) lose `RegisterType`. Named register ops don't update unnamed register. Uppercase append doesn't update type.

**Fix:** Change `delete_history` to `Vec<RegisterContent>`. Make `yank_to_register_with_type` and `delete_to_register_with_type` always update the unnamed register (matching Vim). Fix uppercase append to update type.

**Issues resolved:** OV-00094, OV-00095, OV-00096

**Depends on:** Nothing. Self-contained.

---

### 7. Indentation option wiring

**Problem:** `expandtab` and `shiftwidth` exist in `EditorOptions` but no indentation code reads them. All indent operations hardcode spaces and use `tabstop`. 9 bugs from this one gap.

**Fix:** `indent_lines_at` should consult `expandtab` (tabs vs spaces) and `shiftwidth` (indent width). Cursor positioning after `>>` / `<<` should go to first non-blank of the starting line (not last line at fixed column).

**Issues resolved:** OV-00066, OV-00067, OV-00068, OV-00069, OV-00070, OV-00071, OV-00072, OV-00073, OV-00074

**Depends on:** Nothing. Self-contained.

---

### 8. Command dispatch consolidation

**Problem:** `:e filename` handled in two places with different features (tilde expansion in one, not the other). `:e! filename` falls through the cracks. `:e filename` doesn't check for unsaved changes. The two-tier dispatch (commands.rs → input/commands.rs) creates dead code.

**Fix:** Single dispatch for each command. `:e`/`:e!` should go through one handler that checks the `!` flag, handles filenames, checks `is_modified()`, and calls `replace_content()` (from item 1). Remove dead duplicate handlers.

**Issues resolved:** OV-00101, OV-00102, OV-00110

**Depends on:** Item 1 (buffer content replacement) for proper reload behavior.

---

### 9. Paste behavior fixes

**Problem:** Count ignored, P cursor off-by-one, visual paste doesn't update unnamed register, visual-line paste one line off.

**Fix:** Implement count for p/P. Fix P cursor positioning. Visual paste should store replaced text in unnamed register. Visual-line paste should use paste_before (not paste_after).

**Issues resolved:** OV-00090, OV-00091, OV-00092, OV-00093, OV-00097, OV-00098

**Depends on:** Item 6 (register system) for correct type handling during visual paste.

---

## Suggested execution order

```
1. Buffer content replacement  ──┐
2. Undo stack safety             │ ← foundations, unblock everything
3. Viewport/scroll unification   │
                                 │
4. Motion contracts              │ ← independent, incremental
6. Register type fidelity        │
7. Indentation option wiring     │
                                 │
5. Pattern A→B migration  ───────┤ ← depends on 2
8. Command dispatch  ────────────┤ ← depends on 1
9. Paste fixes  ─────────────────┘ ← depends on 6
```

Items 1–3 are foundations. Items 4, 6, 7 are independent and can run in parallel. Items 5, 8, 9 have dependencies.
