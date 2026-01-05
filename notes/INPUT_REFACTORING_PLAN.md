# Input Module Refactoring Plan

## Current State

`/Users/adrian/Projects/ovim/src/editor/input/mod.rs` is **3,454 lines** after Phase 2.

### Already Extracted Modules

| Module | Lines | Purpose |
|--------|-------|---------|
| `case.rs` | 203 | Case conversion operations (toggle, upper, lower) |
| `char_motion.rs` | 210 | Character motion handlers (f/t/F/T) |
| `commands.rs` | 1,215 | Ex commands (`:w`, `:q`, etc.) |
| `helpers.rs` | 1,129 | Helper functions for cursor movement and editing |
| `leader.rs` | 146 | Leader key sequences (`<Space>...`) |
| `numbers.rs` | 423 | Number increment/decrement (Ctrl-A, Ctrl-X) |
| `search_mode.rs` | 50 | Search mode handler (/, ?) - **Phase 1** |
| `replace_mode.rs` | 165 | Replace mode handler (R) - **Phase 1** |
| `picker_mode.rs` | 186 | Picker mode (file finder, grep, code actions) - **Phase 1** |
| `hover_mode.rs` | 94 | Hover preview/navigate modes - **Phase 1** |
| `filetree_mode.rs` | 56 | File tree navigation - **Phase 1** |
| `substitute_mode.rs` | 40 | Substitute confirm mode (:s/c) - **Phase 1** |
| `dashboard_mode.rs` | 124 | Dashboard menu navigation - **Phase 1** |
| `insert_mode.rs` | 407 | Insert mode handler - **Phase 2** |
| `visual_mode.rs` | 850 | Visual/VisualLine/VisualBlock modes - **Phase 2** |

### mod.rs Structure Analysis (after Phase 2)

The file contains the `InputHandler` struct with these major sections:

1. **Lines 53-100**: Entry point `handle_key_event()` - routes to mode handlers
2. **Lines 102-3426**: `handle_normal_mode()` - THE MONSTER (~3,325 lines!)
3. **Lines 3428-3438**: `poll_event()` (~10 lines)
4. **Lines 3440-3454**: Wrapper methods (~15 lines)

**Extracted to separate modules:**
- `insert_mode.rs` (407 lines) - Insert mode handler
- `visual_mode.rs` (850 lines) - Visual modes handler
- Plus all Phase 1 extractions (search_mode, replace_mode, picker_mode, etc.)

**The main problem**: `handle_normal_mode()` at ~3,300 lines is doing way too much.

### Normal Mode Breakdown

Within `handle_normal_mode()`:

| Section | Lines (approx) | Purpose |
|---------|----------------|---------|
| Legacy leader handling | 70-225 | Pending leader sequences (legacy) |
| Operator + motion combos | 225-1605 | dd, dw, d$, yy, yw, cc, cw, >{motion}, <{motion}, text objects, etc. |
| Pending command handlers | 1980-2525 | g*, z*, Z*, m, ', `, @, f/F/t/T (legacy), [/], window commands |
| Direct key handlers | 2525-3390 | i, a, I, A, o, O, h/j/k/l, word motions, search, operators, etc. |

---

## Extraction Plan

### Target

Get `mod.rs` under **2,000 lines** (ideally ~1,500).

### Priority Order (Low Risk First)

#### Phase 1: Low-Risk Mode Handler Extractions [COMPLETED]

**Status**: All 7 mode handlers extracted successfully.
**Lines removed from mod.rs**: ~610 lines (5,281 -> 4,671)
**New modules created**: 7 files totaling ~715 lines

**1. Extract `search_mode.rs`** (~50 lines) [DONE]
- Move `handle_search_mode()`
- Simple, self-contained, no dependencies on other handlers
- **Estimated effort**: 15 minutes

**2. Extract `replace_mode.rs`** (~160 lines) [DONE]
- Move `handle_replace_mode()`
- Self-contained, uses helpers for movement

**3. Extract `picker_mode.rs`** (~180 lines) [DONE]
- Move `handle_picker_mode()`
- Self-contained picker interaction

**4. Extract `hover_mode.rs`** (~85 lines) [DONE]
- Move `handle_hover_preview_mode()` and `handle_hover_navigate_mode()`
- Note: HoverPreview returns key to forward to normal mode

**5. Extract `filetree_mode.rs`** (~55 lines) [DONE]
- Move `handle_filetree_mode()`
- Simple tree navigation

**6. Extract `substitute_mode.rs`** (~35 lines) [DONE]
- Move `handle_substitute_confirm_mode()`
- Simple y/n/a/q/l dispatch

**7. Extract `dashboard_mode.rs`** (~125 lines) [DONE]
- Move `handle_dashboard_mode()` and `execute_dashboard_action()`
- Menu navigation and action dispatch

**Phase 1 Complete**: ~715 lines in new modules, mod.rs reduced by ~610 lines

---

#### Phase 2: Medium-Risk Extractions [COMPLETED]

**8. Extract `insert_mode.rs`** (~400 lines) [DONE]
- Move `handle_insert_mode()`
- Contains Esc handling with visual block state, completion menu
- Dependencies: `helpers::*` for movement, change building
- **Actual**: 407 lines

**9. Extract `visual_mode.rs`** (~850 lines) [DONE]
- Move `handle_visual_mode()`
- Contains text object selection, visual block operations
- Dependencies: `helpers::*`, `numbers::*`, `TextObjects::*`
- **Actual**: 850 lines

**Phase 2 Complete**: ~1,257 lines in new modules, mod.rs reduced from 4,671 to 3,454 lines (1,217 lines removed)

---

#### Phase 3: High-Impact Normal Mode Decomposition

This is where the real work is. `handle_normal_mode()` needs to be split into logical units.

**10. Extract `operators.rs` (input context)** (~800 lines)
- Move all operator+motion combinations from lines 225-1605:
  - `dd`, `dl`, `dw`, `d$`, `dj`, `dk`, `d{`, `d}`, `d%`, `dG`, `dgg`
  - `yy`, `yw`, `y$`, `yj`, `yk`, `y{`, `y}`
  - `cc`, `cw`, `c$`, `cj`, `ck`, `c{`, `c}`, `cG`, `cgg`
  - `>>`, `>j`, `>k`, `>G`, `>gg`
  - `<<`, `<j`, `<k`, `<G`, `<gg`
  - `zf` fold operator combinations
  - Case operator combinations (gu*, gU*, g~*)
- Create function: `handle_pending_operator(editor, operator, key_event, count) -> Result<bool>`
- **Estimated effort**: 2 hours

**11. Extract `text_objects.rs` (input context)** (~400 lines)
- Move text object handling from lines 1605-1975:
  - Inner/around word, paragraph, sentence
  - Quoted strings
  - Paired delimiters (parens, brackets, braces, angles)
  - Tags, indentation blocks, functions
- Create function: `handle_text_object_with_operator(editor, operator, prefix, key) -> Result<bool>`
- **Estimated effort**: 1.5 hours

**12. Extract `pending_commands.rs`** (~600 lines)
- Move pending command (multi-key sequence) handling from lines 1980-2525:
  - `g*` commands (gg, gd, gD, gy, gR, gc, gq, gJ, ge, gE, g_, gu, gU, g~, gr, gi, gv, gI, g;, gt, gT)
  - `z*` commands (zo, zc, za, zR, zM, zd, zE, zf, zz, zt, zb)
  - `Z*` commands (ZZ, ZQ)
  - `"*` register selection
  - `m*` mark setting
  - `'*` and `` `* `` mark jumping
  - `q*` and `@*` macro recording/playback
  - `f/F/t/T` character motions (legacy handlers)
  - `[*` and `]*` section navigation
  - `W*` (Ctrl-W) window commands
- Create function: `handle_pending_command(editor, pending_char, key_event) -> Result<bool>`
- **Estimated effort**: 2 hours

**13. Extract `motions_input.rs`** (~300 lines)
- Move standalone motion handling from the main match block:
  - h/j/k/l, arrow keys
  - 0, $, ^, _, +, -
  - w, W, b, B, e, E
  - G, gg handling
  - %, {, }, (, )
  - ;, , (find repeat)
  - n, N (search)
  - *, # (word search)
- Create function: `handle_motion(editor, key_event) -> Result<bool>`
- **Estimated effort**: 1 hour

**14. Extract `mode_transitions.rs`** (~200 lines)
- Move mode entry commands:
  - i, a, I, A, o, O (insert mode entries)
  - v, V, Ctrl-V (visual mode entries)
  - R (replace mode)
  - :, /, ? (command/search modes)
  - Space (leader)
- Create function: `handle_mode_transition(editor, key_event) -> Result<bool>`
- **Estimated effort**: 45 minutes

**15. Extract `editing_commands.rs`** (~300 lines)
- Move direct editing commands:
  - x, X (delete char)
  - D, C (delete/change to EOL)
  - s, S (substitute)
  - p, P (paste)
  - Y (yank line)
  - J (join)
  - r (replace char)
  - ~ (toggle case)
  - u, Ctrl-R (undo/redo)
  - . (repeat)
- Create function: `handle_editing_command(editor, key_event) -> Result<bool>`
- **Estimated effort**: 1 hour

**Phase 3 Total**: ~2,600 lines reorganized, ~8.5 hours

---

## New Module Structure

After refactoring:

```
src/editor/input/
  mod.rs              (~800 lines)  - Entry point, dispatch logic, count handling

  # Mode handlers
  normal/
    mod.rs            (~400 lines)  - Normal mode dispatcher
    operators.rs      (~800 lines)  - Operator+motion combos
    text_objects.rs   (~400 lines)  - Text object handling
    pending_commands.rs (~600 lines) - Multi-key sequences (g*, z*, etc.)
    motions.rs        (~300 lines)  - Motion commands
    mode_transitions.rs (~200 lines) - Mode entry commands
    editing_commands.rs (~300 lines) - Direct editing commands
  insert_mode.rs      (~400 lines)  - Insert mode handler
  visual_mode.rs      (~850 lines)  - Visual mode handler
  search_mode.rs      (~50 lines)   - Search mode handler
  replace_mode.rs     (~160 lines)  - Replace mode handler
  picker_mode.rs      (~180 lines)  - Picker mode handler
  hover_mode.rs       (~85 lines)   - Hover preview/navigate modes
  filetree_mode.rs    (~55 lines)   - File tree mode handler
  substitute_mode.rs  (~35 lines)   - Substitute confirm mode
  dashboard_mode.rs   (~125 lines)  - Dashboard mode handler

  # Already extracted
  case.rs             (203 lines)   - Case operations
  char_motion.rs      (210 lines)   - f/t/F/T handling
  commands.rs         (1215 lines)  - Ex commands
  helpers.rs          (1129 lines)  - Helper functions
  leader.rs           (146 lines)   - Leader sequences
  numbers.rs          (423 lines)   - Number operations
```

---

## Interface Patterns

### Return Convention

Each handler function should return `Result<bool>`:
- `Ok(true)` - Key was handled
- `Ok(false)` - Key was not handled (try next handler)
- `Err(_)` - Error occurred

### Normal Mode Dispatcher Pattern

```rust
// In normal/mod.rs
pub fn handle_normal_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    let count = editor.effective_count();

    // 1. Check for pending operator + motion
    if editor.pending_operator().is_some() {
        if operators::handle_pending_operator(editor, key_event, count)? {
            return Ok(());
        }
    }

    // 2. Check for pending command (multi-key sequence)
    if editor.pending_command().is_some() {
        if pending_commands::handle_pending_command(editor, key_event)? {
            return Ok(());
        }
    }

    // 3. Try mode transitions
    if mode_transitions::handle_mode_transition(editor, key_event)? {
        return Ok(());
    }

    // 4. Try editing commands
    if editing_commands::handle_editing_command(editor, key_event)? {
        return Ok(());
    }

    // 5. Try motions
    if motions::handle_motion(editor, key_event)? {
        return Ok(());
    }

    // 6. Set up operators/pending commands
    match key_event.code {
        KeyCode::Char('d') => editor.set_pending_operator(Operator::Delete),
        KeyCode::Char('g') => editor.set_pending_command('g'),
        // ...
    }

    Ok(())
}
```

---

## Migration Strategy

### Step-by-Step Order

1. **Create new files with empty shells** - Set up module structure first
2. **Phase 1** - Extract simple mode handlers (search, replace, picker, hover, filetree, substitute, dashboard)
3. **Phase 2** - Extract insert and visual mode handlers
4. **Phase 3** - Decompose normal mode:
   a. Extract `pending_commands.rs` first (most self-contained)
   b. Extract `operators.rs`
   c. Extract `text_objects.rs`
   d. Extract `motions_input.rs`
   e. Extract `mode_transitions.rs`
   f. Extract `editing_commands.rs`
5. **Verify tests pass after each extraction**
6. **Clean up mod.rs** - Should just be dispatcher logic

### Testing After Each Step

```bash
cargo test
cargo clippy
./target/release/ovim test.txt --headless --session test &
./target/release/ovim send test "ggdd"
./target/release/ovim kill test
```

---

## Estimated Total Effort

| Phase | Lines Moved | Time |
|-------|-------------|------|
| Phase 1 | ~690 | 1.5 hours |
| Phase 2 | ~1,250 | 2 hours |
| Phase 3 | ~2,600 | 8.5 hours |
| Testing & Polish | - | 2 hours |
| **Total** | ~4,540 | **14 hours** |

---

## Risk Mitigation

1. **Commit after each extraction** - Easy rollback if something breaks
2. **Run tests after each step** - Catch regressions early
3. **Keep public API stable** - `InputHandler::handle_key_event()` stays the same
4. **Document dependencies** - Each new module lists what it depends on
5. **Use `pub(super)`** - Keep extracted functions visible only within input module

---

## Final Goal

| File | Lines |
|------|-------|
| `mod.rs` | ~800 |
| `normal/mod.rs` | ~400 |
| Other files | <1,000 each |

No single file over 1,200 lines. Total input module stays maintainable.

## Phase 3: Complete ✅

Successfully decomposed `handle_normal_mode()` into `normal/` subdirectory.

**Modules created:**
- normal/mod.rs (180 lines)
- normal/operators.rs (1,270 lines)
- normal/text_objects.rs (323 lines)
- normal/pending_commands.rs (473 lines)
- normal/motions_input.rs (429 lines)
- normal/mode_transitions.rs (149 lines)
- normal/editing_commands.rs (326 lines)

**Result:** input/mod.rs reduced from 3,454 → 297 lines

**Commit:** b790d79
