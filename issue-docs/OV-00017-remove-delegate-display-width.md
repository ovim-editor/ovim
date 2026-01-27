# OV-00017: Remove delegate display_width in editor/mod.rs

**Status:** Pending | **Priority:** LOW | **Complexity:** Low

## Problem

After the OV-00008 fix, `display_width` in `src/editor/mod.rs` is now just:

```rust
fn display_width(text: &str, tab_width: usize) -> usize {
    crate::display::display_width(text, tab_width)
}
```

This is a one-line delegate with no additional logic. It adds an unnecessary indirection.

## Fix

1. Remove the `display_width` function from `src/editor/mod.rs`
2. Replace call sites in `ensure_wrap_map` with `crate::display::display_width` directly
3. Verify no other callers in the editor module

## Files

- `src/editor/mod.rs` — remove function (~line 147), update call site in `ensure_wrap_map` (~line 640)
