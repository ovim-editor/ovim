# Long-line performance implementation plan

## Objective

Keep local cursor operations and a fixed-size viewport responsive as logical lines grow. Preserve Unicode grapheme semantics, tab stops, wide glyphs, selection, syntax overlap precedence, conceal, inlay hints, split-window geometry, undo and macro behavior. A logical line must no longer be the mandatory allocation or rendering unit.

The investigation identified repeated whole-line coordinate scans, GUI per-grapheme linear style searches, full wrapped-line materialization, and document-wide wrap rebuilds after edits. A debug input-only probe reproduced linear scaling with line length even with wrap, syntax and clipboard I/O disabled. Absolute debug times are not release latency targets.

## Architecture and ownership

### Buffer text index

`Buffer::line_index(line) -> Arc<LineIndex>` provides source-coordinate queries and bounded slices. The index owns a rope snapshot, uses verified grapheme checkpoints, and exposes byte/character/grapheme conversion, raw display coordinates, and local traversal. Coordinate types remain explicit. A slice carries its original byte, character, grapheme and display origins; shortened strings must never masquerade as column zero.

ASCII text has a metadata-based path so insertion into a megabyte of ordinary text does not rescan its unchanged suffix. Unicode segmentation across rope chunks must supply context through `GraphemeCursor`; segmentation cannot assume a fixed lookbehind or independently segment chunks. General Unicode edits may invalidate a suffix where boundary state changes. Measure this separately and retain a correct fallback rather than guessing boundaries.

The buffer owns mutation invalidation. A bounded line-splice journal records post-edit version, starting logical line, and removed/inserted row counts. Reset and raw mutable-rope access invalidate history explicitly. Consumers must rebuild on lost history. The existing decoration edit log is not a substitute: it has different coordinates and reset semantics.

### Window layout

Layout is separate from text indexing. Tab width, wrap width, conceal state and inline decorations affect geometry, while cursor/selection/search/theme are styling inputs. Shared core layout returns only requested display-column or visual-row fragments, retaining source spans and original display origins. Raw text tab stops do not depend on inserted decoration widths or wrapping padding.

A window's wrap map maintains visual counts in a prefix-sum index. Local text edits recompute affected rows, not every document line; line insertion/removal updates structure. Width/tab policy changes and missing history can rebuild. Counts use `usize`, avoiding the current 65,536-row overflow. Decoration and conceal invalidation must stay correct before making it more selective.

### Frontend projection

GUI and TUI consume the same source/layout fragments. They style only visible runs and preserve existing overlay precedence. Ordered interval processing replaces repeated per-grapheme scans through every highlight/match. Wrapped scrolling seeks to requested rows instead of emitting and discarding the prefix. Geometry caches exclude transient overlays, so cursor/selection changes do not rebuild text layout.

## Delivery sequence

1. **Baseline and correctness guard** — reproducible command-line probe with configurable lengths/iterations; row-count overflow regression. Record input, typing, word movement and layout separately. No timing-based unit-test thresholds.
2. **Text index and mutation lifecycle** — source coordinate API, rope-aware Unicode tests, ASCII edit fast path, explicit reset and splice history. Test independent oracle conversions and repeated-query reuse.
3. **Input migration** — route cursor validation, horizontal scrolling, character/word movement and insertion coordinate conversions through the index. Remove whole-line materialization from local hot paths. Re-run baseline before renderer work is assessed.
4. **Incremental wrap accounting** — make production wrap refresh consume the splice journal; preserve per-window settings, inline decoration widths and conceal. Test local edits, split/join, reload, tab-width change, old-history fallback and large row counts.
5. **Visible-fragment rendering** — core layout and GUI migration, then TUI migration. Replace full-line tab/byte arrays, full-line styled splitting and linear style lookup. Compare fragments against the existing complete-line oracle on small adversarial fixtures.
6. **Integrated verification and review** — focused tests, existing macro/register/Unicode/wrap/render suites, GUI/headless checks, release scaling measurements, and independent architectural review. Commit completed verified work on the current branch.

## Verification matrix

- Lengths: 1 KiB, 10 KiB, 100 KiB, 1 MiB; larger overflow fixture as needed.
- Positions: beginning, middle, end, large horizontal offset, deep wrapped subrow.
- Operations: h/l, counted word motions, typing/deleting, macros, cursor-only render, selection/search overlays, local edits and newline split/join.
- Text: plain ASCII, tabs, CJK, combining marks, regional indicators, ZWJ emoji, CRLF and CR, and graphemes spanning rope chunks.
- Presentation: wrap/nowrap, multiple window widths, dense overlapping syntax, dense matches, conceal, inline/EOL decorations.
- Lifecycle: clone/snapshot isolation, undo/redo, reload/full replacement, raw rope mutation, journal eviction.

Deterministic tests assert semantic parity, bounded fragment output, cache/index reuse and affected-line recomputation. Measurements distinguish cold indexing from warm queries and unavoidable changed-suffix work. The goal is near-viewport work for warm rendering and local coordinate operations, with no document-wide text scan for an ordinary single-line edit.

## Execution notes

- Astra owns the context-sensitive text-index architecture/implementation and critical review.
- Sol owns frontend projection and shared fragment layout.
- Terra owns baseline/overflow probes, input migration, and independent wrap-invalidation review.
- The coordinating agent owns integration, wrap-map changes, verification and commits.
- No branch switches or pull requests. Do not introduce arbitrary long-line truncation or silently disable editor features to pass benchmarks.
- Preserve existing deferred external-command scheduling and unrelated macro semantics.

## Progress

- Implemented buffer-owned rope indexes and line-splice history, indexed motions/input, incremental wrap counts, shared visible fragments, and GUI/TUI integration.
- Syntax spans now have cached interval queries; regex matches cache exact whole-line results and renderers seek into the sorted results. Cold regex/highlight work still depends on the logical line.
- Review added regression coverage for old conceal-cursor coordinates after line splices, scrolling after edits before redraw, cross-chunk Unicode, partial wide-glyph clipping, and Unicode inlay hints at wrap boundaries.
- General Unicode edits can still rebuild the changed line's suffix/index and rich layout. Width changes and decoration-generation changes can rebuild window geometry; structural edits rebuild the count tree without re-reading unaffected text.
- GUI retains its existing raw Markdown presentation, with cached raw layout when the core map contains concealed text. TUI uses the cached conceal transform. This work does not unify their Markdown presentation policies.
- GUI projection caches retain the current snapshot's working set, with exact inline-content equality; obsolete text/layout snapshots are released after the next frame. Highlight caches discard obsolete generations. Search caches retain up to 256 live indexed lines.

## Verification results

2026-09-11, local macOS arm64, optimized Rust 1.97.1 build, wrap width 80, ten iterations per measurement. The final measurement ran after this task's compilation finished. These are core command/layout timings, not end-to-end frame latency; short microbenchmarks are sensitive to scheduling. The CSV records operation counts and checksums, and the example is reproducible:

```sh
cargo run -p ovim-core --release --example long_line_probe -- --sizes 1000,10000,100000,1000000 --iterations 10
```

Times below are microseconds per event. Warm measurements build indexes, geometry, and search/highlight caches before timing. Insertions occur near the start of the line. The Unicode fixture repeats `世界 `; the ASCII fixture repeats `word `.

| Operation | 1,000 characters | 1,000,000 characters |
| --- | ---: | ---: |
| ASCII h/l, per key | 0.57 | 0.91 |
| ASCII insertion, per key | 30.18 | 4.39 |
| Unicode h/l, per key | 0.93 | 1.44 |
| Unicode local w/b, per key | 1.02 | 1.55 |
| ASCII deep viewport, 8 rows | 2.12 | 2.75 |
| Unicode deep viewport, 8 rows | 2.08 | 2.60 |
| ASCII far nowrap viewport, 80 columns | 5.05 | 5.47 |
| Unicode far nowrap viewport, 80 columns | 429.75 | 453.00 |
| Dense cached syntax, visible range | 0.38 | 0.44 |
| Unicode cold wrap-map construction | 119.85 | 46478.65 |
| Unicode insertion near start | 51.80 | 49224.35 |
| Traverse one long ASCII word | 5.61 | 4818.46 |

Results support fixed-viewport warm projection and local movement independent of logical-line length. Ordinary ASCII edits avoid full-line and full-document layout work. Cold rich layout and edits near the start of a Unicode line still scale with the changed line; traversing a single long word must inspect that word. Further work on Unicode typing should incrementally repair rich row geometry and reuse converged segmentation state, retaining the existing chunk-context correctness tests.

Validation:

- Core library: 1,729 passed, 1 ignored.
- Frontend library: 299 passed across the final suite and focused cache-test rerun, 2 ignored. The cache test initially assumed an inline hint remained a single fragment; its assertion now joins hint fragments across wrap boundaries and verifies text and lifetime separately.
- 21 selected integration suites: 420 passed, covering macros, grapheme editing, selection, inlays, conceal, multiple windows, wrap scrolling, viewport commands, and property-tested motions.
- CI-toolchain GUI and headless workspace/all-target Clippy checks pass with warnings denied; formatting and diff checks pass.
- Independent review used Astra for segmentation/layout correctness, Sol for GUI projection/caches, and Terra for motion/probe work and wrap-invalidation review.

Raw data: [long-line-release-results.csv](long-line-release-results.csv).

## Live demo-project verification

A release build was exercised through an actual headless session in `~/Testing/demo-project`, including the terminal renderer endpoint. The real `app.js` passed edit/save/undo/redo and was restored byte-for-byte. Dedicated ASCII, Unicode, macro, and Markdown fixtures passed 62 live assertions, including exact saved contents, grapheme deletion, line split/undo, reload invalidation, search, resize, and both split directions.

This caught two gaps beyond the core microbenchmarks:

- Cursor, hover, and completion placement still used full-line text scans. They now share indexed screen-coordinate projection, including raw-to-concealed hover coordinates.
- Splitting a deeply wrapped line discarded its subrow and failed to keep the cursor visible after reflow. Split state now carries a one-shot cursor-row anchor; each pane applies it at its actual dimensions. Rendering also updates the split tree with the whole buffer-region dimensions rather than the focused pane's dimensions.

Identical headless render requests can hit a whole-frame response cache. The live performance probe therefore moves the cursor before each timed render, invalidating that cache. Its final median fresh-frame times, including local HTTP overhead, were 0.98 ms for a million-character ASCII fixture and 0.58 ms for the Unicode fixture (previously 35.2 ms and 44.0 ms in the same harness). Dense-search rendering on the million-character ASCII line took 0.89 ms. A batch of 999 recorded macro replays took 11.6 ms; 99 insertion-macro replays on the million-character ASCII line took 3.45 ms. These short local measurements are not native GUI frame timings or latency guarantees.

The Unicode early-insertion limitation remains: this live run measured 55.6 ms. This work does not claim incremental Unicode suffix repair is complete.

Validation for the follow-up fixes: 1,729 core unit tests; 431 frontend unit and selected integration tests, with 8 ignored tests including 6 requiring a live rust-analyzer setup; focused indexed-cursor and actual split-render assertions. Full live evidence and a reproducible harness remain in `~/Testing/demo-project/ovim-smoke/` (`README.md`, `report.json`, `run.py`, fixtures, and rendered grids).
