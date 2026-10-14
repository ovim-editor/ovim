# Getting Started

## Build and Run

From the repo root:

```bash
cargo build --release
./target/release/ovim path/to/file.txt
```

Open a project directory to start in the file explorer:

```bash
ovim path/to/project
```

## File Explorer

Press `-` from a file to reveal it in the project tree. Press `Tab` to move
focus back to the buffer while leaving the tree open, or `q` to close it.

| Key | Action |
|---|---|
| `j` / `k`, arrows | Move down / up |
| `Enter`, `o`, `l` | Open a file or expand a directory |
| `h` | Collapse a directory or select its parent |
| `a` | Create a file; end the name with `/` to create a directory |
| `R` / `d` | Rename / delete (delete requires confirmation) |
| `y` / `X` / `p` | Copy / cut / paste |
| `f` or `/` / `F` | Filter loaded entries / clear the filter |
| `H` / `I` | Toggle hidden / git-ignored entries |
| `r` | Refresh the tree |
| `gg` / `G` | Select first / last entry |
| `?` | Toggle the explorer key reference |
| `Tab` / `q` | Focus the buffer / close the explorer |

Create, rename, and paste operations refuse to overwrite existing paths.
Rename and create prompts also reject paths that escape the selected directory,
and the explorer root cannot be renamed, moved, or deleted.

## Reviewing Branch Changes

Press `<Space>gd` to open a review of everything your branch changed compared
with the default branch, including uncommitted work. The review is a read-only
patch in its own tab with a file summary at the top. Each hunk is coloured with
the grammar of the file it came from — the way `delta` renders a patch — and
added and removed rows carry a background tint.

Reviews open in split layout by default. Your layout choice is remembered for
the rest of the session. The toolbar above the patch switches between two layouts. Click a button, press
`s`, or run `:GitDiffLayout split` / `:GitDiffLayout unified`:

- **Unified** shows the raw patch, so the buffer still yanks as a valid patch.
- **Split** puts the old file on the left and the new one on the right, with
  source line numbers on both sides. Long lines wrap inside their column, and
  the view re-flows when the window changes width. Split reviews use the full
  available width, ignoring `textwidth`; unified reviews and ordinary files
  retain the configured limit.

Add comments while reviewing with `o` below a source line or `O` above it.
Only comment text is editable; the patch stays read-only. `Enter` adds a line,
`Esc` saves, and `Ctrl-C` cancels in the terminal. On a saved comment, `o` edits
and `D` deletes it. In the GUI, click a code line and press `o`/`O`, or use its
**+** button; comments have Edit and Delete buttons. `Esc` or `Ctrl+Enter` saves.

Opening AI chat from the diff attaches the comments with their captured source,
file, line, side, and comparison. Add your question and send. Comments clear
once the message is accepted for an agent turn; queued messages keep their
comments until that turn starts. A failed submission keeps them available.
The GUI attachment expands for review and can be removed without deleting the
comments. In terminal chat, Backspace on an empty input removes the attachment.
Use **Clear comments** or `:DiffCommentsClear` to discard annotations. Unsent
comments survive closing and reopening the same comparison within the session.
Saved comparison controls remain available with `Alt-o` / `Alt-O` in the terminal
and the existing comparison button in the GUI.

In the GUI, side-by-side review uses two compact code panes. Curved ribbons
connect matching change sections, so a short replacement does not leave a tall
blank space beside a long removal. Scrolling follows corresponding sections;
long lines scroll horizontally within each pane. Stronger highlights mark the
changed text within replacement lines. Use the file selector and change arrows
to navigate, click a ribbon to bring its section into view, or click a source
line number to open the working file. Old-side lines map to their corresponding
location in the current file; deleted files stay read-only.

The GUI also supports `n` / `N` (next / previous hunk), `j` / `k` to scroll,
and the review keys below. `Enter` or `gf` opens the active change in the GUI.
`:` and `<Space>` return to Ovim's command input.
Text can be selected and copied directly from either pane.
In the GUI, `/` or `Cmd/Ctrl+F` opens search across the displayed diff.
Type to highlight matching lines; Enter/Shift+Enter or `n`/`N` moves between
matches. Escape returns focus to the diff. Search follows the selected view and filter.

In the GUI, press `w` or click **Hide equal changes** to hide paired changes
that are equal after ignoring spaces and tabs, including equal sections paired
by the agent in a curated diff. Real edits within a pair remain visible.
Pairs across different files are filtered too; unpaired additions and deletions stay visible. This applies to
both layouts, Guided sections, change navigation, and image exports. Toggle it
off to restore the complete review. The original patch and its change totals
are unchanged.

Mark a file or reassigned section **Reviewed** to hide it while you work through
regular or custom comparisons. In the terminal, press `x` anywhere in the item
or click its checkbox. In the GUI, use the **Reviewed** checkbox or `x` on the
current item. Press `X` / **Show reviewed** to bring checked items back and
uncheck them. Checks survive refreshing, switching layouts, and reopening the
review in the same editor session. Changed content becomes unchecked again.
File and Guided views share progress; exports omit hidden reviewed items.

For `gd` in a GUI diff, click the symbol in the code first, then press `gd`.
The terminal uses the symbol under its cursor. Definition lookup requires the
saved line to match live source; changed historical lines must be opened and
navigated in their current source instead.

In the GUI, `Ctrl+Tab` / `Ctrl+Shift+Tab` cycles through the visible workbench
tabs, including source, diff, and browser tabs. This also works from browser
inputs with browser Vim keys disabled. Browser `:` opens the browser command
line; `Esc` returns focus to the page.

Refinements are saved immediately and shared by the terminal and GUI. After a
restart, open the same comparison with `<Space>gd` or `:GitDiff`: Ovim reapplies
its saved pairing only after verifying the resolved base, complete patch and
full content fingerprints of the compared files. This needs no chat resume.
Changed or unverifiable comparisons keep the saved review separate; use
**View saved review** (`O` in the terminal) to inspect or export that frozen version.

In a managed diff, use `K` / `J` to reveal ten more source lines above / below
the current block. The GUI also has small gutter arrows at the block edges;
they do not add spacer rows. Context comes from the captured comparison, stays
with its own file on each side, and stops at a neighboring block or the end of
the captured source. Older saved reviews may not contain surrounding source.

| Key | Action |
|---|---|
| `<Space>gd` | Open the review, return to it from a file, or leave it |
| `K` / `J` | Reveal ten surrounding lines above / below the current diff block (GUI and terminal) |
| `x` / `X` | Check/uncheck the current file or section / show or hide checked items |
| `gd` | Go to the definition of the selected live-source symbol |
| `s` | Switch between the unified and side-by-side layouts |
| `w` | Hide/show equal paired changes, ignoring whitespace (GUI and terminal curated reviews) |
| `]c` / `[c` | Next / previous hunk (in ordinary files: next / previous git change) |
| `]f` / `[f` | Next / previous file |
| `Enter`, `gf` | Open the file at the line under the cursor, in the tab you came from |
| `Enter` on a summary row | Jump to that file's section of the patch |
| `r` | Refresh the patch |
| `q` | Close the review |
| `<Space>gf` | Fetch the base branch from its remote, then refresh |

The base is picked automatically: the default branch from `origin/HEAD` (or
`main`, `master`, `develop`, `trunk`), compared at its merge-base with your
branch so commits that landed on main after you branched are not shown. When
both `main` and `origin/main` exist, the one with the newer merge-base wins,
so a stale local or un-fetched remote branch never pollutes the review. The
header shows how many commits you are ahead and behind, and when the remote was
last fetched. On the default branch itself the review shows uncommitted changes.

`:GitDiff` does the same from the command line, `:GitDiff <ref>` compares
against any ref (for example `:GitDiff HEAD` for uncommitted work only, or
`:GitDiff main..feature`), `:GitDiffLayout [split|unified]` picks the layout,
and `:GitFetch` fetches the base branch.

`Enter` works in both layouts: in the side-by-side view the column your cursor
is in decides which side you jump from, and the cursor keeps its column.

## AI Chat and Editing

Press `Space Space` in normal mode to open AI chat. Select text visually and
press `Space Space` to open the same chat with that code attached. The built-in Codex profiles use your
ChatGPT subscription.

On first use, Ovim checks its credentials and opens a sign-in dialog. Press
`Enter` to continue in your browser, or `Esc` to dismiss it. Ovim resumes the
draft or selection after sign-in and refreshes its own credentials
automatically; installing or signing in to Codex CLI is not required.

See [AI Setup](ai.md) for profiles, tools, approval behavior, and alternative
providers.

## Running and Debugging Code

| Keys | Runs |
|------|------|
| `Space r r` (or `:Run`) | the code at the cursor, no debugger |
| `F5` / `Space r d` (or `:Debug`) | the code at the cursor under the debugger |
| `Space r l` | the last run again |
| `Space r c` / `Space r C` | pick a configuration from `.ovim/debug.toml` to run / debug |
| `Space r s` (or `Shift-F5`) | stop it |
| `Space r f` | focus the run console (scroll, open stack frames) |

Builds run in the background; compile errors go to the quickfix list. See
[Running and Debugging](running-and-debugging.md).

## Running Tests

vim-test style bindings, prefixed with `Space t` in normal mode:

| Keys | Command | Runs |
|------|---------|------|
| `Space t n` | `:TestNearest` | the test at/near the cursor |
| `Space t f` | `:TestFile` | the current file's tests |
| `Space t a` / `Space t s` | `:TestSuite` | the whole suite |
| `Space t d` | `:TestDebug` | the nearest test under the debugger (Java / Kotlin) |
| `Space t D` | `:TestDebugFile` | the file's tests under the debugger (Java / Kotlin) |
| `Space t l` | `:TestLast` | the previous test command again (same mode: run or debug) |
| `Space t v` | `:TestVisit` | nothing — jumps back to the last-tested spot |
| `Space t t` | `:TestPanel` | nothing — toggles the test panel |
| `Space t o` | `:TestOutput` | nothing — opens the raw log in a buffer |

Running a test opens the **test panel** on the right side of the editor:
output streams in live, the header shows a spinner while the run is in
flight, then flips to a pass/fail verdict with a parsed summary
("12 passed, 1 failed") and duration. The panel keeps a short history of
recent runs. Close it with `Escape` or toggle it with `Space t t`; open it
before any run to see the keybinding cheat sheet. Closing the panel does not
stop a test that is still running.

Commands run in the background in the file's own project root (nearest
`Cargo.toml`, `package.json`, `go.mod`, or pytest marker — resolved per
file, so monorepos just work). Failures also populate the quickfix list
(`:cn` to hop between them); `:TestOutput` shows the raw log.

Built-in runners: `cargo test` (Rust, workspace-aware, with `--test`/`--bin`
target selection), vitest/jest/bun/Node test (detected from dependencies,
config files, lockfiles, and `node:test` imports), pytest (with
uv/poetry/pipenv/pdm prefixes), and `go test`
(with subtest and table-entry `-run` patterns). The nearest test is found via
tree-sitter, so Rust `mod` nesting, nested `describe` blocks, Python classes,
and Go `t.Run` subtests all resolve to correct filters.

**Java and Kotlin** tests (JUnit 4/5, TestNG, kotlin.test) run through Gradle or
Maven. The language server (`hyperion.resolveLaunch`, target `test`) says how
to run the test at the cursor; when it cannot, ovim composes the command from
the file: tree-sitter finds `@Test` / `@ParameterizedTest` methods and nested
classes, the nearest `build.gradle(.kts)` / `pom.xml` picks the module, and
Gradle `--tests` / Maven `-Dtest=` selects them (`./gradlew` only when its
wrapper jar exists). The test panel lists every test with pass/fail, failure
message and stack frames from the JUnit XML reports; failing frames go to the
quickfix list. See [Running and Debugging](running-and-debugging.md).

Other languages can be wired up with `[language.test]` in `languages.toml` —
see [configuration.md](configuration.md).

## Headless Mode (for automation)

Headless mode runs ovim without the TUI and exposes a local REST API for driving the editor.

```bash
ovim path/to/file.rs --headless --session dev
```

Then, in another terminal:

```bash
ovim session list
ovim send -s dev "iHello<Esc>"
ovim snapshot -s dev --format pretty
ovim session kill -s dev
```

## Configuration (Quick)

- Lua config: `~/.config/ovim/init.lua`
- Language config override: `~/.config/ovim/languages.toml`

See `configuration.md` for details.
