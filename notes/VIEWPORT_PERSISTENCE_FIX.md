# Viewport Persistence Fix

## Summary

Fixed viewport command persistence by correcting `update_scroll_offset()` to read from the window's scroll offset instead of the stale editor-level field.

## Root Cause

The `update_scroll_offset()` method was reading from the wrong scroll offset source:

```rust
// BEFORE (line 655)
let current_offset = self.scroll_offset;  // ❌ Reads editor-level field (stale!)
```

This caused incorrect scroll calculations because:

1. Viewport commands (`zt`, `zz`, `zb`) update the **window-level** `scroll_offset`
2. They do NOT update the **editor-level** `self.scroll_offset` field
3. On the next keystroke, `update_scroll_offset()` reads the stale editor-level value
4. It calculates a new scroll position based on this stale value, undoing the viewport command

## The Fix

Changed line 655 to call the getter method instead of accessing the field:

```rust
// AFTER (line 655)
let current_offset = self.scroll_offset();  // ✅ Calls method, reads from window!
```

The `scroll_offset()` getter (lines 637-644) correctly prioritizes the window's scroll offset:

```rust
pub fn scroll_offset(&self) -> usize {
    // If we have a window manager, use the focused window's scroll offset
    if let Some(wm) = &self.window_manager {
        if let Some(window) = wm.focused_window() {
            return window.scroll_offset();  // ← Returns window's value
        }
    }
    // Fall back to editor-level scroll offset for headless/test mode
    self.scroll_offset
}
```

## Example: Before the Fix

**Test scenario**: `24j` → `zt` → `j`

1. Press `24j`: cursor at line 24, editor.scroll_offset might be 5
2. Press `zt`: window.scroll_offset = 24, but editor.scroll_offset = 5 (unchanged!)
3. Press `j`: cursor moves to 25
   - `update_scroll_offset()` reads `self.scroll_offset` (5)
   - Calculates: cursor (25) >= offset (5) + viewport (20)? → Yes, scroll down
   - New scroll: 25 - 19 = 6
   - ❌ **Expected scroll=24, got scroll=6**

## Example: After the Fix

**Same test scenario**: `24j` → `zt` → `j`

1. Press `24j`: cursor at line 24
2. Press `zt`: window.scroll_offset = 24
3. Press `j`: cursor moves to 25
   - `update_scroll_offset()` calls `self.scroll_offset()` → returns 24 (from window)
   - Calculates: cursor (25) >= offset (24) + viewport (20)? → No (25 < 44)
   - Cursor is visible, no scroll change needed
   - ✅ **scroll=24 (preserved!)**

## Viewport Persistence Semantics

Viewport commands (`zt`, `zz`, `zb`) now correctly persist as long as the cursor stays **within the viewport**:

1. **Cursor stays in viewport** → scroll doesn't change (viewport preserved)
2. **Cursor goes above viewport** → scroll up to show cursor
3. **Cursor goes below viewport** → scroll down to show cursor

This matches Vim's behavior.

## Test Updates

Updated two test expectations to match correct behavior:

### test_zb_persists_after_j
- Setup: `24j` → `zb` (scroll=5, viewport shows lines 5-24)
- Action: `j` → cursor at 25 (BELOW viewport)
- Expected: scroll=6 (must adjust to keep cursor visible)
- Was expecting: scroll=5 (incorrect - cursor would be invisible!)

### test_zt_then_k_movement
- Setup: `24j` → `zt` (scroll=24, viewport shows lines 24-43)
- Action: `k` → cursor at 23 (ABOVE viewport)
- Expected: scroll=23 (must adjust to keep cursor visible)
- Was expecting: scroll=24 (incorrect - cursor would be invisible!)

## Files Changed

- `/Users/adrian/Projects/ovim/src/editor/mod.rs` (line 655): Fixed scroll offset source
- `/Users/adrian/Projects/ovim/tests/viewport_persistence_test.rs`: Updated test expectations

## Impact

- ✅ Viewport commands now persist correctly when cursor stays in viewport
- ✅ All 13 viewport persistence tests pass
- ✅ All 5 viewport idempotence tests pass
- ✅ No regressions in other tests (15 search_navigation_test failures are pre-existing)

## Related Issues

This fix complements the earlier viewport idempotence fix (see `VIEWPORT_IDEMPOTENCE_FIX.md`), which:
- Prevented `update_scroll_offset()` from running between multi-key sequences (`z` + `t`)
- Ensured viewport commands set `skip_scroll_update = true`

Together, these fixes ensure viewport commands work correctly in all scenarios.
