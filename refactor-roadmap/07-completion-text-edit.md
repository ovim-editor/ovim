# Completion: Use textEdit Range for Filtering

**Goal:** Completion filtering uses the LSP server's `textEdit` range to determine the completion prefix, instead of the editor's word-boundary guess. This fixes Tailwind CSS completions and any other server where tokens contain non-identifier characters (hyphens, colons, etc.).

## The Problem

When the user types `bg-wh` inside `className="..."` in a TSX file:

1. **Filtering uses wrong prefix.** `completion_trigger_context_from_line` walks backward from cursor until it hits a non-identifier char (`[a-zA-Z0-9_]`). The `-` breaks the word, so the prefix becomes `"wh"` instead of `"bg-wh"`. The filter matches `whitespace-*` (CSS properties) instead of `bg-white` (Tailwind class).

2. **Filtering uses wrong field.** `apply_filter` compares against `insert_text` or `label`. The LSP spec defines `filterText` specifically for filtering. The Tailwind LSP sets `filterText = "bg-white"` on its items.

3. **Ongoing filter recomputes prefix.** When the user types more characters while the menu is visible, `insert_mode.rs:442` calls `completion_trigger_context()` again to recompute the prefix. Even if we fix the initial prefix, subsequent keystrokes re-derive it from word boundaries and get it wrong again.

## What Doesn't Need Fixing

**Apply is already correct.** `accept_completion()` in `ui_features.rs:46-166` already:
- Uses `textEdit.range` for deletion
- Handles `InsertAndReplace` with the `replace` range (broader)
- Positions cursor correctly after insertion
- Records undo via `buffer.record()`
- Handles `additionalTextEdits` bottom-to-top

There's also a broken `apply_completion()` in `lsp_modules/completion.rs:36-68` that ignores the range. It's used from the picker path (`picker_manager.rs:524`). This should be consolidated with `accept_completion` — see Step 3.

## How VS Code / Neovim Do It

The LSP spec says:
- **`filterText`**: "When omitted the label is used as the filter text."
- **`textEdit.range`**: Defines what text the completion replaces. `range.start` is where the token begins.

VS Code derives the filter prefix from `textEdit.range` — reads the text currently in that range from the document. The editor never guesses word boundaries for filtering. Neovim and Helix do the same.

## The Plan

### Step 1: Use `filterText` in `apply_filter`

**File:** `ovim-core/src/editor/completion.rs`

```rust
// Before:
let text = item.insert_text.as_deref().unwrap_or(&item.label);
text.to_lowercase().starts_with(&prefix_lower)

// After:
let filter_text = item.filter_text.as_deref().unwrap_or(&item.label);
filter_text.to_lowercase().starts_with(&prefix_lower)
```

One line change. Falls back to `label` when `filterText` is absent (matches spec). Low risk.

### Step 2: Store server-derived trigger_col when results arrive

**Files:** `ovim-core/src/editor/completion.rs`, `ovim-core/src/editor/lsp_integration.rs`

When completion results arrive in `poll_pending_completion_response`, derive `trigger_col` from the `textEdit.range.start` of the first item, if available:

```rust
// In poll_pending_completion_response:
let (trigger_col, trigger_prefix) = derive_completion_prefix(
    &result.items,
    self.buffer(),
    self,  // for utf16_to_col
);
self.completion_menu_mut().show(result.items.clone(), trigger_col, trigger_prefix);
```

The `derive_completion_prefix` function:
1. Look at the first item's `textEdit.range.start.character`
2. Convert from UTF-16 to char column using `self.utf16_to_col(line, character)`
3. Read the text from that column to the cursor as the prefix
4. If no `textEdit`, fall back to `completion_trigger_context_from_line`

**The "first item" problem in multi-server mode:** When TypeScript and Tailwind both return items, their `textEdit` ranges will differ. Options:

- **Simple:** Use the `textEdit.range.start` that appears most frequently among the items. This is what the majority of items agree on.
- **Correct:** Don't use a single shared prefix. Instead, filter per-item: for each item, derive *its* prefix from *its* `textEdit.range`, and check if `filterText` starts with that prefix. But this requires changing the `CompletionMenu` API.
- **Pragmatic first pass:** Use the first item's range. Items from the other server whose `filterText` doesn't match will be filtered out, which is acceptable — the user is typing in one context, not both.

Start with pragmatic, evolve to correct if needed.

### Step 3: Fix ongoing filter prefix derivation

**File:** `ovim-core/src/editor/input/insert_mode.rs`

Currently, when the user types while the menu is visible:
```rust
// insert_mode.rs:442
let (_, prefix) = editor.completion_trigger_context();
editor.completion_menu_mut().filter(&prefix);
```

This recomputes the prefix from word boundaries, throwing away the server-derived `trigger_col`. Fix: use the stored `trigger_col` to derive the prefix:

```rust
let trigger_col = editor.completion_menu().trigger_col();
let cursor_col = editor.buffer().cursor().col().0;
let line_text = editor.buffer().line(editor.buffer().cursor().line()).unwrap_or_default();
// Read from trigger_col to cursor_col as the current prefix
let prefix = substring_by_grapheme(line_text, trigger_col, cursor_col);
editor.completion_menu_mut().filter(&prefix);
```

This way, `trigger_col` is set once from the `textEdit` range (Step 2) and reused for all subsequent filter updates. For `bg-wh`, `trigger_col` points to the start of `bg`, so as the user types `bg-whi`, `bg-whit`, `bg-white`, the prefix grows correctly.

**Edge case:** If `trigger_col` is past the current cursor (shouldn't happen, but defensive), fall back to `completion_trigger_context`.

### Step 4: Consolidate `apply_completion` and `accept_completion`

**Files:** `ovim-core/src/editor/lsp_modules/completion.rs`, `ovim-core/src/editor/picker_manager.rs`

The broken `apply_completion` in `lsp_modules/completion.rs` should be deleted. The picker path should route through `accept_completion` instead. This may require `accept_completion` to accept an index parameter (currently it uses `selected_item()` from the menu).

```rust
// picker_manager.rs — before:
PickerAction::ApplyCompletion { index } => {
    self.apply_completion(index);
}

// after:
PickerAction::ApplyCompletion { index } => {
    self.completion_menu_mut().select_index(index);
    self.accept_completion();
}
```

Or add `accept_completion_at(index)` that selects and accepts.

## Implementation Order

1. **Step 1** — one-line `filterText` fix, immediate improvement, zero risk
2. **Step 4** — consolidate the two apply functions, delete dead code
3. **Step 2** — derive prefix from `textEdit.range`, the key fix for Tailwind
4. **Step 3** — fix ongoing filter to use stored `trigger_col`

Steps 2+3 are tightly coupled and should ship together.

## Testing

1. **Unit test:** Items with `filterText` are filtered by `filterText`, not `label`
2. **Unit test:** `derive_completion_prefix` extracts correct prefix from `textEdit.range`
3. **Unit test:** Ongoing filter uses stored `trigger_col`, not word-boundary recomputation
4. **Manual: Tailwind.** Type `bg-wh` in className — `bg-white` should appear. Accept — buffer should contain `bg-white`, not `bg-whbg-white`.
5. **Manual: TypeScript.** Type `console.l` — `log` should appear. Regression check.
6. **Manual: Rust.** Type `Vec::n` — `new` should appear. Regression check.

## Files Changed

| File | Change |
|------|--------|
| `ovim-core/src/editor/completion.rs` | `apply_filter` uses `filterText` |
| `ovim-core/src/editor/lsp_integration.rs` | `poll_pending_completion_response` derives prefix from `textEdit` |
| `ovim-core/src/editor/input/insert_mode.rs` | Ongoing filter uses stored `trigger_col` |
| `ovim-core/src/editor/lsp_modules/completion.rs` | Delete broken `apply_completion` |
| `ovim-core/src/editor/picker_manager.rs` | Route through `accept_completion` |
