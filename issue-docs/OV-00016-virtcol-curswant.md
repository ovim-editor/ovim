# OV-00016: No virtcol/curswant tracking for gj/gk

**Status:** Pending | **Priority:** LOW | **Complexity:** Medium

## Problem

Neovim maintains a "wanted virtual column" (`curswant`) that remembers the display column the user was at before moving vertically. When you move through lines of different lengths (or wrapped rows of different widths), the cursor snaps back to the original column when possible.

ovim's `gj`/`gk` compute the target column fresh from the current position each time:

```rust
let target_disp_col = sub_line * wrap_map.wrap_width() + (disp_col % wrap_map.wrap_width());
```

This means: if you're at display column 50, move down through a short wrapped row (width < 50, cursor clamps to end), then move down again to a long row, your cursor stays at the clamped position instead of returning to column 50.

## Example

```
Line 1: "aaaaaaaaaa|bbbbb"     (cursor at display col 10, marked |)
         "bbbbb"               (wrapped row, width 5)
Line 2: "cccccccccccccccccc"   (long line)
```

- `gj` from col 10 → wrapped row "bbbbb", col 10 % 16 = 10 but row is only 5 wide, clamps to 4
- `gj` again → Line 2, col should return to 10 but goes to 4 instead

Neovim would remember `curswant = 10` and restore it on Line 2.

## Scope

This affects more than just `gj`/`gk` — regular `j`/`k` in Neovim also use `curswant`. ovim's `j`/`k` implementation should be checked for the same issue.

## Fix approach

1. Add `curswant: Option<usize>` to `InputContext` or `Cursor`
2. Set `curswant` to the current display column on horizontal movement (any motion that isn't purely vertical)
3. In `gj`/`gk`/`j`/`k`, use `curswant` as the target display column instead of the current column
4. Clear `curswant` on any non-vertical motion (horizontal movement, search, etc.)

The cursor should store this as a display column, not a character column.

## Files

- `src/editor/input/normal/pending_commands.rs` — gj/gk handlers
- `src/editor/input/helpers.rs` — move_up/move_down (j/k)
- `src/editor/input_context.rs` or `src/buffer/cursor.rs` — add curswant field
- `src/editor/motions.rs` — potentially clear curswant on horizontal motions

## Testing

- Integration test: move to col 50, gj through short row, gj to long row → cursor at col 50
- Integration test: move to col 50, gj, then type `l` (horizontal), gj → cursor at col 51 (curswant reset)
- Integration test: j/k through lines of varying length → cursor returns to original column
