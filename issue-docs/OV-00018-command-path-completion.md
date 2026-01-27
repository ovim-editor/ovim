# OV-00018: Tab completion for file paths in command mode

**Status:** Pending | **Priority:** MEDIUM | **Complexity:** High

## Summary

Add file path tab-completion for ex commands that take file arguments (`:e`, `:tabe`, `:w`, `:saveas`, `:source`, `:split`/`:sp`, `:vsplit`/`:vsp`). Show a popup menu of completions, always visible when path context is active.

## Behavior specification

### Triggering

The completion popup appears whenever the command line contains a recognized file-argument command and the cursor is in the path portion. It should always be visible once you're in a path context — typing `./` immediately shows the current directory contents.

Completion activates for these commands:
- `:e`, `:edit`
- `:tabe`, `:tabedit`
- `:w`, `:write` (when given a path argument)
- `:saveas`
- `:source`
- `:sp`, `:split`
- `:vsp`, `:vsplit`

### Tab cycling

- **`<Tab>`** — if popup is visible, insert the top/selected match into the command line. If multiple matches, cycle to next.
- **`<S-Tab>`** — cycle to previous match.
- When a **single match** remains, `<Tab>` completes it fully:
  - If it's a file: inserts the full filename
  - If it's a directory: inserts the directory name + `/` and immediately shows contents of that directory

### Typing narrows the list

As the user types characters, the popup narrows to show only entries matching the current prefix. The popup stays open — no need to press `<Tab>` again to re-summon it.

Example flow:
1. `:e ./` — popup shows all entries in `./`
2. User types `s` — popup narrows to entries starting with `s` (e.g., `src/`, `scripts/`)
3. User types `r` — popup narrows to `src/`
4. `<Tab>` — completes to `:e ./src/`, popup shows contents of `src/`
5. User types `m` — popup narrows to `main.rs`, `mod.rs`, etc.

### Backspace behavior

- `<BS>` deletes a character and widens the filter
- If deleting back to a `/`, the popup shows the parent directory contents
- If deleting the entire path, popup dismisses

### Arrow keys

- **`<Down>`** / **`<Up>`** — navigate within the popup to select a specific entry
- Selected entry is highlighted visually
- `<Tab>` with a selection inserts that specific entry (not just the top match)

### Enter

- **`<Enter>`** always executes the command line as-is, regardless of popup state
- This matches Neovim behavior: the popup is a visual aid for tab-cycling, not an independent modal selection widget
- If a directory is the current command line text, `:e src/` will error naturally — the user is expected to keep completing

### Popup appearance

- Positioned above the command line (similar to existing completion menu)
- Max height: 10-15 items, scrollable if more
- Each entry shows:
  - Icon or suffix to distinguish files vs directories (trailing `/` for directories)
  - Directories sorted before files
  - Hidden files (`.git`, `.env`) shown but sorted after non-hidden entries
- Matched prefix highlighted (bold or different color)

### Path resolution

- `~` expands to home directory
- Relative paths resolve from the editor's current working directory
- Symlinks are followed; shown as their target type (file/dir)
- Unreadable directories are skipped silently (no error popup)
- Non-existent intermediate paths show empty completions (no popup)

### Edge cases

- **Empty path** (`:e <Tab>`) — show current working directory contents
- **Trailing slash** (`:e src/<Tab>`) — show contents of `src/`, don't cycle directory itself
- **No matches** — popup dismisses, `<Tab>` is a no-op
- **Very long filenames** — truncate with `…` in popup, but insert full name on completion
- **Spaces in paths** — handle correctly (may need quoting or escaping, check Neovim behavior)
- **Root path** (`:e /`) — show root directory contents

## Architecture

### New components

1. **`PathCompleter`** — filesystem interaction layer
   - `fn completions(partial_path: &str, cwd: &Path) -> Vec<PathEntry>`
   - `PathEntry { name: String, is_dir: bool, is_hidden: bool }`
   - Handles `~` expansion, relative path resolution, sorting

2. **Command argument parser** — determines if current command expects a file path
   - Lives in or near `src/commands.rs`
   - Returns the path portion of the command line for completion

3. **Popup widget** — renders the completion list
   - Can likely reuse/extend the existing `CompletionMenu` in `src/editor/completion.rs`
   - Needs to support the file-specific display (dir indicators, sorting)

4. **Input handling** — Tab/S-Tab/arrow integration in command mode
   - Lives in `src/editor/input/` command mode handler

### State machine

```
Idle
  ├─ user types file command + space → Active(path="")
  │
Active(path, entries, selected_index)
  ├─ char typed → update path, re-filter entries, reset selection
  ├─ <BS> → shorten path, re-filter entries
  ├─ <Tab> → insert selected entry; if dir, append "/" and reload
  ├─ <S-Tab> → select previous entry
  ├─ <Down> → select next entry
  ├─ <Up> → select previous entry
  ├─ <Enter> → execute command (exits command mode)
  ├─ <Esc> → dismiss popup, return to command line
  └─ non-path command edit → Idle
```

## Files (expected)

- `src/editor/completion.rs` — extend or add PathCompleter
- `src/editor/input/command.rs` — tab/arrow handling in command mode
- `src/commands.rs` — identify which commands take file arguments
- `src/ui/renderer/widgets/` — popup rendering for path completions

## Testing

- Unit test: PathCompleter returns sorted entries (dirs first, hidden last)
- Unit test: `~` expansion resolves correctly
- Unit test: narrowing filter works with prefix matching
- Unit test: directory completion appends `/`
- Integration test: `:e src/<Tab>` cycles through src/ contents
- Integration test: `:e src/m<Tab>` completes to `main.rs` or shows `main.rs`, `mod.rs` etc.
- Integration test: backspace widens filter correctly
