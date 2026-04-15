# 08: Column Coordinate Correctness

**Goal:** Every function that takes a column parameter uses the correct coordinate system. No implicit conversions, no silent mismatches between UTF-16 / char / grapheme columns.

**Fixes:** Latent buffer corruption with combining characters in completion, potential off-by-one in decoration adjustment for non-ASCII text.

**Risk:** Low per-fix, but requires careful audit. Most fixes are one-line changes (calling the right conversion function).

## The Problem

Three coordinate systems coexist:

| System | Used by | Example: `e` + combining accent `e\u{0301}` |
|--------|---------|----------------------------------------------|
| UTF-16 code units | LSP protocol | 2 (one per code unit) |
| Char indices | Rope operations (`insert_text_at`, `delete_range`) | 2 (e + combining mark) |
| Grapheme indices | Cursor (`GraphemeCol`), trigger_col, display | 1 (one visible cluster) |

For ASCII text, all three are identical. Bugs only manifest with combining characters, emoji, or CJK text.

### Known mismatches

**1. `accept_completion_item` fallback path** (`ui_features.rs`)

```rust
let cursor_col = self.buffer().cursor().col().0;  // grapheme
let trigger_col = self.completion_menu.trigger_col();  // grapheme
// ...
buf.delete_range(cursor_line, trigger_col, cursor_line, cursor_col);  // expects char
```

`delete_range` takes char indices. `trigger_col` and `cursor_col` are grapheme indices. For `cafе\u{0301}` (café with combining accent), grapheme col 4 is char col 5. The deletion range would be wrong.

**2. `completion_prefix_from_trigger_col`** (`lsp_modules/completion.rs`)

Uses `byte_offset_for_grapheme` to extract the prefix string — this is correct for producing the string. But `trigger_col` is stored as a grapheme index and later used in `delete_range` (char context). The prefix string is right, the column is wrong.

**3. `completion_trigger_context_from_line`**

Returns `trigger_col` as a grapheme count (`grapheme_count(&line_text[..start_byte])`). This value flows into `CompletionMenu.trigger_col` and eventually into `delete_range` in the fallback path.

### Why it hasn't been caught

- Completions almost exclusively operate on ASCII identifiers
- The Tailwind fix uses `textEdit` ranges (UTF-16, converted correctly via `utf16_to_col`), so the textEdit path is fine
- Only the *fallback* path (no textEdit on the completion item) uses the wrong coordinate

## The Fix

### Principle: make the type system enforce correctness

Introduce a `CharCol(usize)` newtype alongside the existing `GraphemeCol(usize)`. Functions that take char columns take `CharCol`. Functions that take grapheme columns take `GraphemeCol`. The compiler catches mismatches.

This is the ideal but invasive approach. A pragmatic alternative:

### Pragmatic: audit and fix each call site

1. **Audit every call to `delete_range` and `insert_text_at`** — verify the column arguments are char indices, not grapheme indices
2. **Fix `CompletionMenu.trigger_col`** — store as char col (from `utf16_to_col` on the textEdit path, from a new `grapheme_to_char_col` conversion on the fallback path)
3. **Fix `accept_completion_item` fallback** — convert `cursor.col()` from grapheme to char before passing to `delete_range`
4. **Add a test with combining characters** — completion on a line containing `café` to verify the range is correct

### 4. Substitute command byte offset bug

In `input/commands.rs:467-479`, the `:s` (substitute) command uses `regex.find_iter()` which returns byte offsets (`mat.start()`, `mat.end()`). These byte offsets are passed to `delete_range` which expects char indices. For ASCII text they're identical, but for multibyte characters the range is wrong.

## Scope

| Call site | Current | Correct? | Fix |
|-----------|---------|----------|-----|
| `accept_completion_item` textEdit path | `utf16_to_col` → char | Yes | None |
| `accept_completion_item` fallback | grapheme | **No** | Convert to char |
| `record_operation` closures | Mixed (caller-dependent) | Audit needed | Per-caller |
| `apply_lsp_text_edit` | `utf16_to_col` → char | Yes | None |
| `completion_trigger_context_from_line` return | grapheme | OK for prefix, wrong for delete_range | Store char col |
| Substitute command (`:s`) | byte offset | **No** | Convert to char via `str.chars().count()` |

## Files

- `ovim-core/src/editor/ui_features.rs` — accept_completion_item fallback path
- `ovim-core/src/editor/completion.rs` — trigger_col storage type
- `ovim-core/src/editor/lsp_modules/completion.rs` — trigger context, prefix derivation
- `ovim-core/src/editor/input/commands.rs` — substitute match positions
- `ovim-core/src/unicode/mod.rs` — conversion functions (already exist)
