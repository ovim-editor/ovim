# Viewport Command Idempotence Fix

## Problem

Viewport commands (`zt`, `zz`, `zb`) were **not idempotent** - pressing the same command multiple times caused the scroll position to change when it shouldn't.

**Expected behavior**:
- `zt` should position current line at top. Pressing `zt` again should do nothing.
- `zz` should center current line. Pressing `zz` again should do nothing.
- `zb` should position current line at bottom. Pressing `zb` again should do nothing.

**Actual behavior**:
Each press of `zt`/`zz`/`zb` changed the scroll position, causing visible jitter.

## Root Cause

The issue was caused by **TWO separate bugs** working together:

### Bug #1: Intermediate Scroll Updates

The input handler calls `update_scroll_offset()` at the end of **every** key event:

```rust
// src/editor/input/mod.rs (before fix)
fn handle_key_event(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    // ... process key event ...

    if !editor.skip_scroll_update {
        editor.update_scroll_offset();  // ← Called EVERY key press
    }
}
```

For multi-key sequences like `zt`:
1. Press 'z': Sets `pending_command = 'z'`, then **calls `update_scroll_offset()`**
2. Press 't': Executes `move_cursor_line_to_top()`, sets `skip_scroll_update = true`

The problem: `update_scroll_offset()` is called **between** the 'z' and 't' keys, changing the scroll position before the viewport command executes!

### Bug #2: Cursor Desync

The Window cursor was not being synced with the Buffer cursor during normal navigation:

```
Starting state:
- Buffer.cursor: line 24
- Window.cursor: line 0  ← STALE!
```

When `update_scroll_offset()` calculated scroll position, it used the **Buffer cursor** (line 24), but the Window cursor was stale (line 0), causing incorrect scroll calculations.

## Execution Trace (Before Fix)

**First `zt`:**
```
Press 'z':
  Buffer.cursor: 24, Window.cursor: 0 (stale!)
  Set pending_command = 'z'
  update_scroll_offset() → scroll changes from 15 to 14

Press 't':
  move_cursor_line_to_top() → scroll becomes 24 ✓
  skip_scroll_update = true
```

**Second `zt`:**
```
Press 'z':
  Buffer.cursor: 24, Window.cursor: 24 (now synced)
  Set pending_command = 'z'
  update_scroll_offset() → scroll changes from 24 to 15 (!)

Press 't':
  move_cursor_line_to_top() → scroll becomes 24 again
```

Result: Scroll goes 24 → 15 → 24, causing visible jitter.

## The Fix

**Skip scroll updates when viewport pending command is active:**

```rust
// src/editor/input/mod.rs (after fix)
let is_viewport_pending = matches!(editor.pending_command(), Some('z') | Some('Z'));
if !editor.skip_scroll_update && !is_viewport_pending {
    editor.update_scroll_offset();
}
```

This prevents `update_scroll_offset()` from being called between the 'z' and 't' key presses.

### Why Only 'z' and 'Z'?

Initially I tried blocking scroll updates for **all** pending commands:

```rust
if !editor.skip_scroll_update && editor.pending_command().is_none() {
    editor.update_scroll_offset();
}
```

This broke register operations (`"ayy`, `"bp`, etc.) because those commands DO need scroll updates after the second key.

The refined fix only blocks scroll updates for **viewport-specific** pending commands ('z' and 'Z'), allowing all other multi-key sequences to work correctly.

## Execution Trace (After Fix)

**First `zt`:**
```
Press 'z':
  Set pending_command = 'z'
  is_viewport_pending = true
  update_scroll_offset() → SKIPPED
  scroll stays at 15

Press 't':
  move_cursor_line_to_top() → scroll becomes 24 ✓
```

**Second `zt`:**
```
Press 'z':
  Set pending_command = 'z'
  is_viewport_pending = true
  update_scroll_offset() → SKIPPED
  scroll stays at 24

Press 't':
  move_cursor_line_to_top() → scroll stays at 24 ✓ IDEMPOTENT!
```

Result: Scroll stays at 24 - no jitter!

## Tests

Created `/Users/adrian/Projects/ovim/tests/viewport_idempotence_test.rs` with comprehensive tests:

- `test_zt_is_idempotent` - Tests `zt` command
- `test_zz_is_idempotent` - Tests `zz` command
- `test_zb_is_idempotent` - Tests `zb` command

All tests pass ✓

## Side Effects

This fix also **improved** register operations tests:
- Before fix: 38 failures in `register_operations_test.rs`
- After fix: 5 failures in `register_operations_test.rs`

The fix prevented spurious scroll updates from interfering with cursor positioning in register operations.

## Related Issues

The cursor desync issue (Bug #2) was not fully addressed in this fix. While viewport commands now sync the Window cursor to the Buffer cursor, normal navigation still leaves the Window cursor stale.

This should be addressed separately to ensure scroll calculations are always based on the correct cursor position. Consider:
- Syncing Window cursor after every motion
- Using Buffer cursor directly for scroll calculations
- Eliminating the redundant cursor tracking

## Files Changed

- `src/editor/input/mod.rs` - Added viewport pending command check
- `tests/viewport_idempotence_test.rs` - New comprehensive tests

## Date

2026-01-03
