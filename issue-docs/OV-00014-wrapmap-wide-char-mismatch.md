# OV-00014: WrapMap compute_visual_lines disagrees with renderer for wide chars

**Status:** Pending | **Priority:** HIGH | **Complexity:** Medium

## Problem

`WrapMap::compute_visual_lines` uses a simple formula:

```rust
fn compute_visual_lines(display_width: usize, wrap_width: usize) -> u16 {
    if display_width == 0 { 1 }
    else { ((display_width + wrap_width - 1) / wrap_width) as u16 }
}
```

This is `ceil(display_width / wrap_width)`. It assumes characters pack perfectly into rows.

However, `split_line_into_rows` (the renderer) pads rows when a wide character (width 2) doesn't fit at a row boundary. This padding wastes space and can produce **more rows** than the formula predicts.

## Reproduction

Content: `"世世世世世"` (five CJK characters, each width 2), `wrap_width = 3`.

**WrapMap says:** `display_width = 10`, `compute_visual_lines(10, 3) = ceil(10/3) = 4`

**Renderer produces:**
- Row 1: `"世 "` — 世(2) + pad(1) = 3. Next 世 would be 2+2=4 > 3, doesn't fit.
- Row 2: `"世 "` — same
- Row 3: `"世 "` — same
- Row 4: `"世 "` — same
- Row 5: `"世 "` — same
- **Result: 5 rows**, not 4.

## Impact

When WrapMap undercounts visual lines:
- **Scrolling** jumps or skips lines — viewport calculations use the wrong total
- **Cursor positioning** via `cursor_to_visual` returns wrong visual row
- **gj/gk** land on wrong positions
- The error compounds over multiple CJK-heavy lines

This is a regression introduced by the OV-00013 fix (which correctly added padding to `split_line_into_rows` but didn't update `compute_visual_lines` to match).

## Fix approach

Replace the arithmetic formula with a simulation that matches the renderer's packing logic:

```rust
fn compute_visual_lines(display_width: usize, wrap_width: usize) -> u16 {
    // ... but this needs per-character widths, not just total display_width
}
```

The problem is that `compute_visual_lines` only receives `display_width` (a single number), but the padding behavior depends on where wide characters fall relative to row boundaries. Two options:

### Option A: Change line_len closure to return row count directly

Instead of `line_len: Fn(usize) -> usize` returning display width, have it return the visual line count directly, computed by simulating the row-packing:

```rust
fn compute_visual_rows(text: &str, wrap_width: usize, tab_width: usize) -> u16
```

This walks the characters, tracking `current_width`, and increments the row count whenever a character doesn't fit.

### Option B: Pass per-character widths

Change the closure signature to pass the actual text or a width iterator. More invasive.

**Recommendation:** Option A. Add `compute_visual_rows` to `src/display.rs` and use it in WrapMap construction. The existing `compute_visual_lines(display_width, wrap_width)` can remain as a fast path for ASCII-only lines (where no padding waste occurs).

## Files

- `src/editor/wrap_map.rs` — `compute_visual_lines`, `new`, `rebuild`, `invalidate_line`
- `src/ui/renderer/buffer.rs` — `split_line_into_rows` (reference implementation)
- `src/display.rs` — new `compute_visual_rows` function
- `src/editor/mod.rs` — `ensure_wrap_map` closure

## Testing

- Unit test: `"世世世世世"` at `wrap_width=3` → 5 rows (not 4)
- Unit test: `"abc世d"` at `wrap_width=4` → 2 rows (no change, padding doesn't add rows here)
- Unit test: ASCII-only line → same result as before (no regression)
- Property test: `compute_visual_rows` output matches `split_line_into_rows` row count for random Unicode inputs
