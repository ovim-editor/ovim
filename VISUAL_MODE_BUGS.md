# Visual Mode Bug Hunt Report

**Date**: 2025-12-31
**Scope**: Comprehensive review of visual mode implementation in ovim

## Critical Bugs Found

### 1. **Missing `gv` (Reselect Visual) Implementation**
**Location**: `src/editor/input/mod.rs` - `handle_visual_mode()` and `handle_normal_mode()`
**Severity**: High - Core vim feature missing

**Issue**: The `gv` command to reselect the last visual selection is not implemented, despite having tests for it.

**Test Evidence**:
- `/Users/adrian.helvik/Personal/ovim/tests/visual_mode_test.rs:439` - `test_gv_reselect()` expects `gv` to work
- `/Users/adrian.helvik/Personal/ovim/tests/visual_mode_test.rs:452` - `test_gv_after_delete()` expects `gv` to work

**Expected Behavior**: After exiting visual mode, `gv` should restore the previous visual selection (start position, end position, and mode type).

**Actual Behavior**: `gv` is not handled at all - likely just enters visual mode at current cursor position.

**Missing State**: No tracking of last visual selection in Editor struct. Need to store:
- Last visual start position
- Last visual end position
- Last visual mode type (Visual/VisualLine/VisualBlock)

**Fix Required**:
1. Add fields to Editor struct to track last visual selection
2. Store visual selection info when exiting visual mode
3. Implement `gv` handling in normal mode to restore last selection

---

### 2. **Visual Mode Switching Doesn't Preserve Selection Start**
**Location**: `src/editor/input/mod.rs:3673-3683` (handle_visual_mode)
**Severity**: Medium - Incorrect behavior when switching modes

**Issue**: When switching between visual modes (v → V, V → v, v → Ctrl-V), the visual_start position is not properly adjusted for the target mode.

**Code Analysis**:
```rust
// Line 3673-3678
KeyCode::Char('v') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
    editor.set_mode(Mode::VisualBlock);
}
KeyCode::Char('v') => {
    editor.set_mode(Mode::Visual);
}
```

**Problem**: When switching from character-wise visual to line-wise visual (pressing `V` while in visual mode), the code at line 3680-3682 sets `visual_start` to `(cursor.line(), 0)`, but this should preserve the original anchor line. When switching from line-wise to character-wise, the column should be restored, not kept at 0.

**Expected Behavior**:
- `v` in VisualLine mode → Character-wise visual, preserve anchor line, restore anchor column
- `V` in Visual mode → Line-wise visual, preserve anchor line, set column to 0
- `Ctrl-V` in any visual mode → Block visual, preserve both anchor line and column

**Actual Behavior**: Visual start position is incorrectly modified or not preserved.

**Test Gap**: No test for `test_v_to_V_switch()` and `test_V_to_v_switch()` verifying the anchor position.

---

### 3. **Missing Text Object Support in Visual Mode**
**Location**: `src/editor/input/mod.rs` - `handle_visual_mode()`
**Severity**: High - Missing standard vim functionality

**Issue**: Visual mode doesn't handle text objects (`iw`, `aw`, `ip`, `ap`, `i{`, `a{`, etc.).

**Evidence**:
- Test exists: `/Users/adrian.helvik/Personal/ovim/tests/text_objects_test.rs:63` - `test_viw_visual_inner_word()`
- Visual mode handler has NO code for handling `i` or `a` followed by text object specifiers

**Expected Behavior**: In visual mode, pressing `iw` should extend selection to inner word, `ap` to around paragraph, etc.

**Actual Behavior**: `i` and `a` in visual mode likely do nothing or cause unexpected behavior.

**Missing Implementation**: Need to add text object handling in `handle_visual_mode()` similar to how operators handle them in normal mode.

---

### 4. **Missing Search Support in Visual Mode**
**Location**: `src/editor/input/mod.rs` - `handle_visual_mode()`
**Severity**: Medium - Missing standard vim feature

**Issue**: Visual mode doesn't handle `/` (search forward) or `?` (search backward) to extend selection.

**Test Evidence**: Line 424-432 in `tests/visual_mode_test.rs` - `test_v_to_search_result()` exists but implementation is missing.

**Expected Behavior**:
- `/pattern` in visual mode → extend selection to next match
- `?pattern` in visual mode → extend selection to previous match
- `n` and `N` should work to jump between matches

**Actual Behavior**: No search commands handled in visual mode.

---

### 5. **Missing Paste in Visual Mode**
**Location**: `src/editor/input/mod.rs` - `handle_visual_mode()`
**Severity**: Medium - Common operation missing

**Issue**: Visual mode doesn't handle `p` (paste) to replace selection.

**Test Evidence**: Line 247-261 in `tests/visual_mode_test.rs` - `test_v_yank_and_replace()` expects `p` to work but the test result shows incorrect behavior.

**Expected Behavior**: `p` in visual mode should:
1. Delete the visual selection
2. Paste the register contents at that position
3. Exit visual mode

**Actual Behavior**: `p` key is not handled in `handle_visual_mode()`.

**Fix Required**: Add `KeyCode::Char('p')` handler that calls delete_visual_selection, then paste operation.

---

### 6. **Visual Block `r` Replace Edge Case - Empty Lines**
**Location**: `src/editor/input/helpers.rs:586-642` and `mod.rs:3385-3445`
**Severity**: Low - Edge case handling

**Issue**: When visual block replace (`r{char}`) encounters lines shorter than `start_col`, it correctly skips them (line 3399-3400), but the behavior might be confusing to users.

**Code**:
```rust
let line_start = start_col.min(chars.len());
let line_end = (end_col + 1).min(chars.len());
```

**Analysis**: This is actually correct Neovim behavior - short lines are not extended. However, there's no test coverage for this edge case.

**Recommendation**: Add test for visual block operations on ragged-right lines (different line lengths).

---

### 7. **Visual Line Delete at EOF Edge Case**
**Location**: `src/editor/input/helpers.rs:568-585` (delete_visual_selection)
**Severity**: Low - Edge case

**Issue**: Visual line deletion uses `(end_line + 1, 0)` as the end position. If `end_line` is the last line, this could point past EOF.

**Code Analysis**:
```rust
let end_pos = (end_line + 1, 0);
let deleted = editor.buffer_mut().delete_range(start_line, 0, end_line + 1, 0);
```

**Potential Issue**: What happens when deleting the last line? Does `delete_range()` handle `end_line + 1` gracefully when it exceeds line count?

**Recommendation**: Check buffer's `delete_range()` implementation for bounds checking. Add test for `Vd` on the very last line of a file.

---

### 8. **Visual Mode Case Operations Only Handle Single Line for `~`**
**Location**: `src/editor/input/mod.rs:3770-3889`
**Severity**: Medium - Incomplete implementation

**Issue**: The `~` (toggle case) operator in regular visual mode only handles the single-line case (line 3847-3878), but silently does nothing for multi-line selections.

**Code**:
```rust
} else {
    // Regular visual mode - toggle case of selection
    if let Some(((start_line, start_col), (end_line, end_col))) = editor.visual_selection() {
        // ... code ...
        // Handle simple case: same line
        if start_line == end_line {
            // ... implementation ...
        }
    }
    // NOTE: No else branch - multi-line case is silently ignored!
    editor.clear_visual_start();
    editor.set_mode(Mode::Normal);
}
```

**Expected Behavior**: `~` should toggle case across multiple lines in visual mode.

**Actual Behavior**: Multi-line case toggle silently does nothing.

**Fix Required**: Implement multi-line case toggle similar to visual block implementation.

---

### 9. **Visual Mode `U` and `u` (Uppercase/Lowercase) Only Work in VisualBlock**
**Location**: `src/editor/input/mod.rs:3891-3994`
**Severity**: High - Missing functionality

**Issue**: `U` (uppercase) and `u` (lowercase) operators only have implementations for VisualBlock mode. Regular visual mode has NO implementation.

**Code**:
```rust
KeyCode::Char('U') => {
    if editor.mode() == Mode::VisualBlock {
        // ... implementation ...
    }
    // No else branch - Visual and VisualLine modes are not handled!
}
KeyCode::Char('u') => {
    if editor.mode() == Mode::VisualBlock {
        // ... implementation ...
    }
    // No else branch!
}
```

**Expected Behavior**: `U` and `u` should work in all visual modes (Visual, VisualLine, VisualBlock).

**Actual Behavior**: `U` and `u` only work in VisualBlock mode. In other visual modes, they do nothing.

**Fix Required**: Add else branches to handle Visual and VisualLine modes.

---

### 10. **Visual Selection Edge Case: Empty Selection**
**Location**: `src/editor/visual_mode.rs:36-96` (visual_selection method)
**Severity**: Low - Edge case handling

**Issue**: If visual_start equals cursor position, the selection is empty or single-character. The normalization logic handles this, but edge cases exist.

**Analysis**:
```rust
// Line 88-92 in Visual mode
if start.0 < end.0 || (start.0 == end.0 && start.1 <= end.1) {
    (start, end)
} else {
    (end, start)
}
```

**Edge Case**: When `start == end`, the selection includes the single character at that position. This is correct, but not explicitly tested.

**Recommendation**: Add tests for:
- `v` then immediately `d` (delete single char)
- `V` then immediately `d` (delete single line)
- `Ctrl-V` then immediately `d` (delete single char as block)

---

### 11. **Visual Mode Count Handling May Be Incomplete**
**Location**: `src/editor/input/mod.rs:4067-4074` (handle_visual_mode)
**Severity**: Low - Limited test coverage

**Issue**: Visual mode accepts count prefixes (e.g., `v3j`, `V5k`), but there's limited test coverage.

**Code**:
```rust
KeyCode::Char(c) if c.is_ascii_digit() => {
    let digit = c.to_digit(10).unwrap() as usize;
    if digit != 0 || editor.count().is_some() {
        editor.append_count(digit);
    }
}
```

**Tests**: Only basic count tests exist (lines 401-418 in visual_mode_test.rs).

**Recommendation**: Test edge cases:
- Very large counts (e.g., `v9999999j`)
- Count that moves past EOF
- Count with operators (e.g., `v3wd`)

---

### 12. **VisualLine Mode: Start Column Should Always Be 0**
**Location**: `src/editor/visual_mode.rs:42-66` (visual_selection method)
**Severity**: Low - Consistency issue

**Issue**: The visual_selection() method correctly adjusts start column to 0 for VisualLine mode, but when entering VisualLine from Normal mode, the visual_start is set to (line, 0) only sometimes.

**Code Analysis**:
```rust
// Entering VisualLine from Normal mode (mod.rs:2682-2686)
KeyCode::Char('V') => {
    let cursor = editor.buffer().cursor();
    editor.set_visual_start(cursor.line(), 0);  // Correctly sets col to 0
    editor.set_mode(Mode::VisualLine);
}

// Switching to VisualLine from within visual mode (mod.rs:3680-3683)
KeyCode::Char('V') => {
    let cursor = editor.buffer().cursor();
    editor.set_visual_start(cursor.line(), 0);  // Also correct
    editor.set_mode(Mode::VisualLine);
}
```

**Analysis**: Actually handled correctly in both cases. No bug here.

---

## Missing Features (Not Bugs, But Gaps)

### 13. **No `f`/`F`/`t`/`T` Find Character Support in Visual Mode**
**Location**: `src/editor/input/mod.rs` - `handle_visual_mode()`
**Severity**: Medium

Not implemented: `f`, `F`, `t`, `T` motions to extend visual selection to character.

### 14. **No `{`/`}` Paragraph Motion in Visual Mode**
**Location**: `src/editor/input/mod.rs` - `handle_visual_mode()`
**Severity**: Low

Not implemented: Paragraph motions `{` and `}` to extend selection.

### 15. **No `%` Matching Bracket Motion in Visual Mode**
**Location**: `src/editor/input/mod.rs` - `handle_visual_mode()`
**Severity**: Medium

Not implemented: `%` to extend selection to matching bracket.

---

## Tests That Need to Be Added

1. **Visual mode with text objects**: `viw`, `vaw`, `vip`, `vap`, `vi{`, `va{`, etc.
2. **Visual mode paste**: `vyiwvp` should replace word with yanked text
3. **Visual mode search**: `v/pattern`, `v?pattern`, then `n` and `N`
4. **gv reselection**: After any visual operation, `gv` should restore last selection
5. **Multi-line case toggle**: `v2j~` should toggle case across 3 lines
6. **Visual mode U/u**: `vwU` should uppercase word, `vwu` should lowercase
7. **Visual line delete at EOF**: `VGd` on last line should work correctly
8. **Empty visual selection**: `vx` should delete single char
9. **Visual block ragged edges**: Block operations on lines of different lengths
10. **Visual mode switching**: `vwV` and `Vjv` should preserve anchor correctly
11. **Find motions in visual mode**: `vfx`, `vTa`, etc.
12. **Paragraph motions in visual mode**: `v}`, `v{`
13. **Bracket matching in visual mode**: `v%`

---

## Summary of Findings

**Critical Issues**: 3
- Missing `gv` implementation
- Missing text object support in visual mode
- Missing `U`/`u` in Visual and VisualLine modes

**High Priority Issues**: 2
- Missing paste support in visual mode
- Visual mode switching doesn't preserve anchor properly

**Medium Priority Issues**: 4
- Missing search support in visual mode
- Incomplete case toggle (multi-line)
- Missing find character motions
- Missing bracket matching motion

**Low Priority Issues**: 3
- Visual block on ragged lines (edge case)
- Visual line delete at EOF (edge case)
- Empty selection handling (edge case)

**Total Issues Found**: 12 bugs + 3 missing features

---

## Recommended Fix Priority

1. Implement `gv` (reselect visual) - most critical missing feature
2. Add text object support in visual mode (`viw`, `vap`, etc.)
3. Implement `U`/`u` for all visual modes
4. Add paste (`p`) in visual mode
5. Fix multi-line case toggle (`~`)
6. Add search support (`/`, `?`, `n`, `N`)
7. Fix visual mode switching to preserve anchor
8. Add find character motions (`f`, `F`, `t`, `T`)
9. Add paragraph and bracket motions
10. Add edge case tests and fixes

---

## Files Requiring Changes

1. **`src/editor/mod.rs`**: Add fields for last visual selection tracking
2. **`src/editor/visual_mode.rs`**: Potentially add helper methods
3. **`src/editor/input/mod.rs`**:
   - Add `gv` handling in normal mode
   - Add text object handling in visual mode
   - Add paste handling in visual mode
   - Add search handling in visual mode
   - Fix `U`/`u`/`~` for all visual modes
   - Add find/paragraph/bracket motions
   - Fix mode switching logic
4. **`tests/visual_mode_test.rs`**: Add comprehensive tests for all findings
