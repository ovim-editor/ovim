# 18: Unified `Buffer::line_text` Accessor (DONE)

> **Migration + cleanup shipped 2026-04-28.** `Buffer::line_text(idx) -> Option<Cow<'_, str>>` (strips `\r?\n$` and lone trailing `\r`) and `Buffer::line_grapheme_count(idx)` were added; `Buffer::line()` was deleted; all 228 compile errors were fixed by mechanical migration. The 195 dead `trim_end_matches('\n')` calls were swept by a brute-force sed pass; the 80+ resulting type errors were patched by a programmatic `&` insertion script; the 5 cases where the trim was actually live (raw `Rope::line()` results in renderer / motion / event_loop, AI extraction, `&mut String` patterns) were restored individually. **10 live calls remain, all justified by their receiver type.** A regression guard in `buffer::line_ending::tests::trim_end_matches_n_count_is_bounded` pins the count and lists the live sites — adding a new direct call requires bumping the bound (forcing a code review). Full test suite (3,471 tests) green; release build clean. The bug class is structurally closed.

---

**Goal:** Replace the 196 hand-rolled `line.trim_end_matches('\n')` sites with a single canonical accessor that strips `\r?\n$` once.

**Fixes:** Closes the line-ending bug class. Today's `trim_end_matches('\n')` is the project's de-facto "give me the visible content of this line" call, but it doesn't strip `\r`. OV-00250/251 stopped CRs from entering the rope through paste and LSP edits, but anything that *does* slip in (Mac-classic load — OV-00252, AI tool insertions, mixed-ending files — OV-00254) still propagates a `^M` artifact into rendering, motion bounds, search/regex matches, operator targets, and undo cursor positions.

**Risk:** Low per site (mechanical), wide blast radius (motions, rendering, search, substitute, paste, LSP, AI tools). Each file is its own incremental commit.

**Effort:** Medium. ~20 files, ~196 callsites. Mostly substitution; a handful need real thought (callers that slice into the trimmed `&str` past where a `\r` would have changed the byte length).

## The Asymmetry

```bash
$ grep -rn "trim_end_matches('\\n')" ovim-core/src ovim/src | wc -l
196

$ grep -rn "strip_suffix('\\r')\|trim_end_matches('\\r')" ovim-core/src ovim/src | wc -l
4
```

196 sites strip `\n` only. Four sites also strip `\r`:

- `ovim-core/src/lsp/utils.rs:34,38,250` — three call sites that handle CR because they convert LSP-supplied positions across line boundaries
- `ovim-core/src/ai/stream_parsers.rs:31` — strips `\r` because it parses SSE streams

Every other line-content read in the codebase is silently wrong on a line that contains a trailing `\r`. The bug is latent today only because OV-00250/251 plugged the two highest-volume input seams.

### Top of the distribution

```
32  ovim-core/src/editor/input/helpers.rs
17  ovim-core/src/buffer/text_ops.rs
15  ovim-core/src/textobjects.rs
11  ovim-core/src/repeat_action.rs
 9  ovim-core/src/editor/input/commands.rs
 8  ovim-core/src/editor/motions/word.rs
 8  ovim-core/src/editor/motions/sentence.rs
 8  ovim-core/src/editor/input/normal/pending_commands.rs
 7  ovim-core/src/editor/motions/screen.rs
 7  ovim-core/src/editor/motions/bracket.rs
 6  ovim-core/src/editor/mod.rs
 5  ovim-core/src/editor/input/insert_mode.rs
 4  ovim-core/src/editor/visual_mode.rs
 4  ovim-core/src/editor/search_manager.rs
 4  ovim-core/src/editor/motions/char_find.rs
... (remainder spread across motion + render code)
```

### The two common shapes

Roughly 70% of sites are one of these two forms:

```rust
// Shape A: get grapheme length of visible content
let line_len = grapheme_count(line.trim_end_matches('\n'));

// Shape B: get visible content as &str for further ops
let line_text = line.trim_end_matches('\n');
// ... search, regex, slice, char_indices ...
```

The remaining ~30% are minor variants — `chars().count()` instead of `grapheme_count` (some legitimate, some still-uncaught grapheme bugs like OV-00246 was), `find()` on the trimmed slice, etc.

## The Fix

Add two methods to `Buffer`:

```rust
impl Buffer {
    /// Visible content of line `idx`, with the trailing line terminator
    /// (`\r?\n` or bare `\n`) removed. Returns `None` for out-of-range.
    ///
    /// This is the canonical accessor for "the text the user sees on this
    /// line". Prefer this over `line()` + `trim_end_matches('\n')` — the
    /// hand-rolled form silently leaves `\r` in the slice, which corrupts
    /// motions, search, and rendering on any line whose terminator is
    /// `\r\n` (mixed line endings, Mac-classic content, AI tool output).
    pub fn line_text(&self, idx: usize) -> Option<Cow<'_, str>> { ... }

    /// Grapheme count of the visible content on line `idx`.
    ///
    /// Equivalent to `grapheme_count(line_text(idx)?.as_ref())`, but
    /// avoids the temporary `String` allocation when the line ends in
    /// `\r\n` (the rope slice is converted at the boundary, not earlier).
    pub fn line_grapheme_count(&self, idx: usize) -> usize { ... }
}
```

`Cow<'_, str>` is the right return type:

- LF-only (the common case) → `Cow::Borrowed` over the rope slice's `to_string()` minus the `\n` — actually, since `RopeSlice::as_str()` only works for contiguous slices, we may need to `to_string()` once. **Investigate during implementation** whether `RopeSlice::chunks()` lets us avoid the allocation in the common case. If not, the cost is one allocation per line read, identical to today's `line()` cost.
- `\r\n` terminator → strip `\r\n` from the owned string before returning.

`line_grapheme_count` exists because Shape A is so dominant that giving callers a one-call form is worth a method. It also gives us a single place to enforce "grapheme count" instead of "char count", which would have caught OV-00202/246 if it had existed.

## Migration

This is the opposite of a "do it all in one PR" refactor. The blast radius covers core invariants (cursor clamping, operator targets, search positions). Each file should land independently with the existing tests passing.

### Suggested ordering

1. **Add the accessor + tests.** New method, no callers yet. ~30 LOC + unit tests covering LF, CRLF, bare-CR-trailer (which OV-00252 says shouldn't happen post-load but will if Mac-classic detection lands), out-of-range.

2. **Migrate `buffer/text_ops.rs` first** (17 sites). It's the closest to the rope, and many of its callers go through it anyway. Get the foundation right.

3. **Then migrate by file, biggest first.** `editor/input/helpers.rs` (32) → `textobjects.rs` (15) → `repeat_action.rs` (11) → motion files. One commit per file. Run `cargo test` after each.

4. **Sweep the long tail.** Files with 1–4 sites each. These are mechanical.

5. **Add a clippy lint or guard test.** Once the migration is done, a simple check (`! grep -rn "trim_end_matches('\\\\n')" ovim-core/src ovim/src`) should run in CI to prevent regression. Or a `Buffer::line()` deprecation pointing callers at `line_text()`.

### Sites that need real thought

A small subset of callers do `line.trim_end_matches('\n')` and then index by byte offset. After the migration, those byte offsets need to remain valid — confirm that no caller is relying on the byte-length difference between trimmed and untrimmed content (i.e., relying on the trailing `\n` being still in the slice for arithmetic). The audit during step 2 will surface these.

## Why now

OV-00250/251 created `buffer::normalize_for_buffer` as the **input** seam — text flowing into the rope is canonicalized to LF. This roadmap creates the **output** seam — text flowing out of the rope into motion / search / render code is read through one canonical accessor. Together they form bookends: the rope is LF-only by convention, and *every* read that wants visible content goes through one place that enforces that convention.

Without this phase, the contract is one-sided: we trust that nothing puts `\r` into the rope, and 196 callers will silently corrupt anything that does.

## What NOT to do

- Don't add a third "strip everything weird" mode. The accessor strips `\r?\n$` and nothing else. Mid-line `\r` should NOT be stripped — that masks OV-00252 (Mac-classic load) which deserves its own fix.
- Don't merge `line_text` and `line_slice` (the existing `RopeSlice<'_>` accessor). The slice form is useful for zero-alloc reads where the caller will iterate chars or graphemes itself; `line_text` is for "I want the visible content as a `&str`-ish thing". Two different jobs.
- Don't migrate render hot paths and motion paths in the same commit. Render is allocation-sensitive (tight per-frame loop); motion is correctness-sensitive (cursor clamping invariants). Land separately so a regression bisects to one or the other.

## Files

Add the accessor:
- `ovim-core/src/buffer/mod.rs` — `line_text`, `line_grapheme_count`

Migrate (in suggested order, one commit per file):
- `ovim-core/src/buffer/text_ops.rs` (17)
- `ovim-core/src/editor/input/helpers.rs` (32)
- `ovim-core/src/textobjects.rs` (15)
- `ovim-core/src/repeat_action.rs` (11)
- `ovim-core/src/editor/input/commands.rs` (9)
- `ovim-core/src/editor/motions/{word,sentence,screen,bracket,char_find}.rs` (~38 combined)
- `ovim-core/src/editor/input/normal/pending_commands.rs` (8)
- `ovim-core/src/editor/{mod,visual_mode,search_manager}.rs` (~14 combined)
- `ovim-core/src/editor/input/insert_mode.rs` (5)
- Long tail (~50 sites in ~30 files)

Out of scope:
- `ovim-core/src/lsp/utils.rs` — already CR-aware, has its own conventions
- `ovim-core/src/ai/stream_parsers.rs` — parses SSE, not rope content
