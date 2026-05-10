# 19: Per-Window Wrap Map (DONE)

> **Shipped** as `2c38903` (19.1+19.2 foundation), `70c7f9c` (19.3 renderer),
> `2db5684` (19.4 `wrap_map()`/`ensure_wrap_map()` → focused window), plus 19.5
> tests (`per_window_wrap_tests` in `ovim-core`, `ovim/tests/per_window_wrap_test.rs`).
> The wrap map now lives on `Window`; `ViewportState::wrap_map` is the headless /
> no-`WindowManager` fallback only. OV-00209 closed. The plan below is kept for
> the "why" trail; the only open follow-on noted (the doubled `scroll_offset`)
> is the unrelated ex-roadmap-10 editor decomposition.

---

**Goal:** Move the soft-wrap map from a single editor-global slot to per-window
state, so each split window wraps at its own width and reports its own visual
geometry.

**Fixes:** OV-00209 — non-focused split windows render with the *focused*
window's wrap break points. With a vertical split where the panes have different
widths, the non-focused pane's gutter row count, content row count, and cursor
position are computed against the wrong width. Switching focus then "fixes" the
pane that was wrong and breaks the one that was right. Also closes a latent
class: any future per-window viewport feature (independent scroll in wrap mode,
`zz`/`zt`/`zb` in a non-focused pane via mouse, etc.) inherits the same bug
until the wrap map is per-window.

**Risk:** Medium. The wrap map itself is well-tested and unchanged. The risk is
in the *plumbing* — `ensure_wrap_map` and ~8 reader sites currently assume "the"
wrap map; they need to learn "which window's". Several readers (`cursor_to_visual`,
scroll math) implicitly mean "the focused window" and stay correct if we keep a
focused-window fast path; the renderer's per-window content loop is the one that
genuinely needs the window-keyed lookup.

**Effort:** Medium. ~5 tasks. Touches `viewport_state.rs`, `window.rs`,
`editor/mod.rs` (`ensure_wrap_map`, scroll/cursor-visibility math),
`window_viewport.rs`, `ui/renderer/{core,buffer,overlays}.rs`,
`editor/input/mouse.rs`, `editor/input/normal/pending_commands.rs`, and the test
harness's `editor.wrap_map()` accessor.

---

## Where the wrap map lives today

```rust
// ovim-core/src/editor/viewport_state.rs
pub struct ViewportState {
    pub viewport_height: usize,
    pub scroll_offset: usize,
    pub skip_scroll_update: bool,
    pub wrap_map: Option<WrapMap>,                 // ← one map for the whole editor
    pub wrap_decoration_generation: u64,           // ← decoration gen it was built at
}
```

`Editor` owns one `ViewportState` (`self.viewport`). Separately, `Editor` owns a
`WindowManager` of `Window`s; each `Window` already stores its own `cursor`,
`scroll_offset`, `horizontal_offset`, `width`, `height`. The focused window's
cursor/scroll are mirrored onto the buffer/editor on focus change
(`save_cursor_to_focused_window` / `restore_cursor_from_focused_window` in
`window_viewport.rs`) — that's the established "focused window's state is the
editor's state" pattern.

`ensure_wrap_map(text_width)` is called from the renderer with the *focused*
window's content width and (re)builds `self.viewport.wrap_map` if the buffer
version / width / line count / decoration generation changed. Then:

- `Editor::wrap_map() -> Option<&WrapMap>` returns `self.viewport.wrap_map`.
- The renderer's per-window content loop (`ui/renderer/buffer.rs::render_buffer`)
  calls `editor.wrap_map()` regardless of which window it's drawing → **this is
  the bug**: the non-focused pane gets break points computed at the focused
  pane's width.
- `cursor_to_visual_with_decorations` (renderer overlays + core) and the
  wrap-mode scroll/cursor-visibility math in `editor/mod.rs` use
  `editor.wrap_map()` and mean "the focused window".
- `editor/input/mouse.rs` (5 sites) and `pending_commands.rs` (`gj`/`gk`) use
  `editor.wrap_map()` and mean "the window the action targets" — which today is
  always the focused one, so they're accidentally fine.

## The shape of the fix

A wrap map is a pure function of `(line_count, wrap_width, tab_width,
buffer_version, decoration_generation, line_text fn, inline_widths fn)`. Two
windows on the *same* buffer at the *same* width could share a map; two windows
at *different* widths can't. The simplest correct model: **one wrap map per
window**, stored on `Window`, keyed on the window's own width.

### Option A — `wrap_map` on `Window`, editor keeps a focused-window accessor (recommended)

```rust
// ovim-core/src/editor/window.rs
pub struct Window {
    buffer_id: usize,
    cursor: Cursor,
    scroll_offset: usize,
    horizontal_offset: DisplayCol,
    width: u16,
    height: u16,
    // new:
    wrap_map: Option<WrapMap>,
    wrap_decoration_generation: u64,
}
```

- `ViewportState` keeps `wrap_map` *only* for the no-window-manager case
  (headless / single-window-before-init / the test harness as it stands today).
  Actually — cleaner: `ViewportState::wrap_map` becomes the "implicit window 0"
  map and `Editor::wrap_map()` returns the focused window's map if a
  `WindowManager` exists, else `self.viewport.wrap_map`. This keeps every
  existing `editor.wrap_map()` caller working with zero changes — they all mean
  "focused window".
- `ensure_wrap_map` gains a sibling `ensure_wrap_maps_for_all_windows()` (or the
  renderer calls `ensure_wrap_map_for_window(window_idx, width)` in its
  per-window loop). The renderer's content loop then renders window *i* against
  *window i's* map.
- Decoration generation is still editor-global (decorations belong to the
  buffer, not the window), so each window's `wrap_decoration_generation` is
  compared against `self.decorations.generation` the same way the single map
  does today.

**Trade-off:** small duplication — N wrap maps for N windows on the same buffer
at the same width. In practice N is 1–4 and wrap maps are cheap (two `Vec<u16>`/
`Vec<usize>` of `line_count` length). Not worth a sharing cache.

### Option B — `WrapMap` keyed by `(buffer_id, width)` in a small editor-side map

A `HashMap<(usize, u16), WrapMap>` on the editor. Windows look theirs up by
`(self.buffer_id, self.width)`. De-dupes shared maps.

**Trade-off:** invalidation gets fiddlier (which entries to drop on resize? on
buffer switch? on decoration change?), and you've reinvented a cache for a
problem (1–4 cheap maps) that doesn't have it. Adds a layer without removing
one. Rejected.

### Option C — keep the global map, render non-focused panes without soft-wrap geometry

i.e. accept that non-focused panes are visually slightly off and just stop the
*crash-shaped* mismatches (gutter vs content row count). This is what the
codebase effectively does now, badly. Rejected — it's the bug, not a fix.

### Decision

**Option A.** It mirrors the existing per-window state pattern (`cursor`,
`scroll_offset` already live on `Window`), keeps `Editor::wrap_map()` meaning
"focused window" so the ~12 existing call sites need no change, and confines the
new work to (1) the `Window` field, (2) `ensure_wrap_map` becoming
per-window-aware, and (3) the renderer's per-window content loop.

## Plan

**19.1 — `Window` gets `wrap_map` + `wrap_decoration_generation`.**
Add the fields (default `None` / `0`). Add `Window::wrap_map()` /
`wrap_map_mut()` / `set_wrap_map()` and a `Window::invalidate_wrap_map()` that
zeroes the generation. No behavior change yet — nothing reads them.

**19.2 — `ensure_wrap_map` becomes per-window.**
Refactor the body of today's `ensure_wrap_map` into
`ensure_wrap_map_for(width: usize, buffer: &Buffer, decorations: &DecorationMap,
slot: &mut Option<WrapMap>, gen_slot: &mut u64, tab_width, wrap_enabled)` —
a free fn / associated fn that operates on a borrowed slot. Then:
- `Editor::ensure_wrap_map(text_width)` (focused window / no-WM): drives the
  focused window's slot (or `self.viewport.wrap_map` if no `WindowManager`).
- new `Editor::ensure_wrap_map_for_window(window_idx, text_width)`: drives that
  window's slot.
The borrow dance (clone the rope so the `line_text` closure doesn't capture
`&self`) is unchanged — `display::line_content` already factored the strip out
(roadmap 18 / OV-00264 partial).

**19.3 — renderer renders each window against its own map.**
`ui/renderer/core.rs`: in the multi-window layout path, after computing each
window's content `text_width`, call `editor.ensure_wrap_map_for_window(i, w)`
before rendering window *i*. `ui/renderer/buffer.rs::render_buffer` takes the
window's `&WrapMap` (or window index) instead of calling `editor.wrap_map()`.
`ui/renderer/overlays.rs`: the cursor overlay is always for the focused window,
so it keeps `editor.wrap_map()` (= focused) — but assert/comment that.

**19.4 — `Editor::wrap_map()` returns the focused window's map.**
`Editor::wrap_map()` → if `WindowManager` exists, focused window's
`wrap_map.as_ref()`, else `self.viewport.wrap_map.as_ref()`. This makes
`mouse.rs`, `pending_commands.rs`, and the wrap-mode scroll math in
`editor/mod.rs` automatically correct (they target the focused window). Delete
`ViewportState::wrap_map` once nothing but the no-WM path reads it — or keep it
as the "window 0" slot if killing it ripples too far; decide during 19.4.

**19.5 — tests.**
- Unit: two windows, different widths, same buffer → each window's map has the
  expected `total_visual_lines` / break points.
- Integration: vertical split, narrow + wide pane, a long line that wraps in the
  narrow pane but not the wide one; cursor in the narrow pane lands on the right
  visual row; switch focus, cursor in the wide pane lands on the right visual
  row; the previously-narrow pane still renders correctly (it's now non-focused).
- Regression: the existing single-window wrap tests must pass unchanged
  (`editor.wrap_map()` still resolves for the headless / single-window case).
- Resize: `resize_updates_viewport_and_wrap_map_and_keeps_cursor_visible`
  (event_loop test) should be extended to a split layout.

## Open questions

- **Does the test harness create a `WindowManager`?** `EditorTest::new` →
  `Editor::new`/`with_content` sets `window_manager: None` until viewport size
  is known. Many wrap tests call `editor.ensure_wrap_map(width)` directly and
  read `editor.wrap_map()` without ever initializing a `WindowManager`. Option A
  keeps `ViewportState::wrap_map` as the no-WM slot precisely so these keep
  working — confirm before 19.4 whether any test *does* init a WM and would then
  read the wrong slot.
- **Headless rendering** uses `render_cache.last_text_width` as a wrap-width
  fallback when `wrap_map` is `None` (`compute_fallback_wrap_scroll_offset`).
  That path is per-editor, not per-window; leave it (headless has one logical
  viewport).
- **`scroll_offset` is doubly stored** (`Window::scroll_offset` *and*
  `ViewportState::scroll_offset`, kept in sync on focus change). The wrap map
  follows the same doubled pattern under Option A. Not ideal, but consistent
  with the established model; unifying the window/editor viewport split is a
  separate, larger piece of work (it's the `editor/mod.rs` decomposition, ex-
  roadmap 10) and out of scope here.

## Why not now / sequencing

This is the only remaining item with active user-facing impact besides roadmap
17. It's bigger than a Goldilocks branch (wide reader blast radius) but smaller
than the editor decomposition. Recommend after 17, or before it if split-window
use is more common than multi-server-companion use in practice. Each task 19.1–
19.5 is its own commit; 19.1 and 19.2 are safe to land independently (no behavior
change until 19.3).
