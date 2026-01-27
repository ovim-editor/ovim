# OV-00015: Incremental wrap map invalidation only covers cursor line

**Status:** Pending | **Priority:** MEDIUM | **Complexity:** Medium

## Problem

`ensure_wrap_map` (src/editor/mod.rs) uses `invalidate_line(cursor_line)` when only the buffer version changed (same line count and wrap width). This is correct for single-line edits in insert mode, but multi-line operations change lines the cursor isn't on:

- **Paste** (`p`/`P`) — inserts multiple lines, but only the line count change triggers a full rebuild. If a paste replaces text within lines (e.g., visual block paste), line count stays the same but multiple lines change.
- **Undo/redo** — can restore arbitrary changes spanning many lines.
- **LSP formatting** — reformats a range of lines.
- **`:s///g` with range** — substitutes across multiple lines.
- **Visual mode operations** — delete/change across a selection.

In these cases, only the cursor line gets its visual count updated; all other changed lines remain stale until something triggers a full rebuild (resize, line count change, or wrap toggle).

## Impact

Stale visual counts cause:
- Incorrect total visual line count → scrollbar/viewport miscalculation
- `cursor_to_visual` returns wrong row for lines below the stale region
- Visual jumps when the map eventually gets rebuilt

The impact is moderate because most edits either change the line count (triggering full rebuild) or are single-line (cursor invalidation suffices). The gap is for same-line-count multi-line edits.

## Fix approaches

### Option A: Track dirty line range in Buffer

Add a `dirty_range: Option<(usize, usize)>` to `Buffer` that records the range of lines modified since the last wrap map sync. `ensure_wrap_map` invalidates all lines in the dirty range.

**Pros:** Precise, minimal wasted work.
**Cons:** Requires Buffer to track per-line dirtiness, adds coupling.

### Option B: Fall back to full rebuild for multi-line edits

Detect whether the edit was single-line (cursor line unchanged or only one line modified) vs. multi-line, and full rebuild for multi-line. Could check if `buffer.last_change_line_count() > 1` or similar.

**Pros:** Simple, conservative.
**Cons:** Loses the O(1) optimization for some cases that could be incremental.

### Option C: Always invalidate a range around cursor

Heuristic: invalidate `cursor_line ± N` (e.g., N=5). Catches most paste/undo cases without full rebuild.

**Pros:** Simple, good enough in practice.
**Cons:** Unprincipled, misses changes far from cursor.

**Recommendation:** Option A is cleanest but Option B is pragmatic. Start with Option B — if the edit changed more than one line, do a full rebuild. The insert-mode hot path (single-line edits) still gets O(1) invalidation.

## Files

- `src/editor/mod.rs` — `ensure_wrap_map` decision tree
- `src/buffer/mod.rs` — potentially add change tracking metadata

## Testing

- Unit test: paste 3 lines into a wrapped buffer, verify all 3 lines have correct visual counts
- Unit test: undo a multi-line change, verify wrap map matches full rebuild output
