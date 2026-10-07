# Project Tools

Project-level workflows: replace in files, quickfix commands, recent files and
buffers, symbol search, breadcrumbs and outline, git staging and history, merge
conflicts, and the Problems list. Everything here works in the terminal UI, the
GUI and headless sessions.

## Replace in files

`<Space>sr` (or `:SearchReplace`) opens a review panel prefilled with the word
under the cursor. Type the text to find, `Tab` to the replacement and to a file
filter, and the matches appear grouped by file with the replacement previewed
inline (old text struck through, new text after it).

| Key | Action |
|---|---|
| `Tab` / `Shift-Tab` | Next / previous field: Find, Replace, Files, Results |
| `Alt-c` / `Alt-w` / `Alt-r` | Toggle match case / whole word / regular expression |
| `Ctrl-T` | Check or uncheck the selected match (on a file row: the whole file) |
| `Ctrl-A` | Check or uncheck everything |
| `Up` / `Down`, `Ctrl-N` / `Ctrl-P` | Move through the matches |
| `Enter` | In a field: go to the results; on a match: open it (the panel stays available) |
| `Alt-Enter`, `Ctrl-R` | Replace every checked match |
| `Esc` | Hide the panel (`:SearchReplace` brings it back) |

In the results, `j`/`k` move, `Space` toggles, `a` toggles all and `A` replaces.

The Files field takes comma or space separated globs; a plain glob includes
(`*.java`), a `!` prefix excludes (`!build`, `!**/generated/**`). `.gitignore`
and hidden files are respected, binary files and files over 4 MB are skipped,
and open buffers are searched from memory so unsaved edits are found.

Regular expression mode expands `$1` / `\1` / `${name}` in the replacement
(`$1_x` is group 1 followed by `_x`); in literal mode the replacement is
inserted verbatim.

Applying edits every touched file through its own buffer, including files that
were not open (they are loaded as hidden buffers), so each buffer has one undo
step: `u` undoes it in the current buffer and `:ReplaceUndo` undoes the whole
replacement in all touched buffers (buffers you edited since are left alone).
Changed files are then saved, like `:cfdo s/x/y/ | update`. Lines that changed
since the search are skipped and reported instead of being edited blindly.

Command form: `:SearchReplace /find/replace/[cwr] [globs]` runs the search
immediately (`c` case, `w` whole word, `r` regex); `:ReplaceApply` applies it.

## Quickfix commands

| Command | Action |
|---|---|
| `:grep pattern [-- globs]` | Fill the quickfix list from a project search (regex, smart case, `.gitignore` aware) |
| `:cdo {cmd}` | Run `{cmd}` at every quickfix entry (stops at the first error) |
| `:cfdo {cmd}` | Run `{cmd}` once per file in the quickfix list |
| `:update` | Write the buffer only if it changed |

`{cmd}` owns the rest of the line, so `:cfdo %s/foo/bar/ge | update` chains per
file. A bar after a complete `:s/pat/rep/flags` separates the next command, and
`:s` reports `E486: Pattern not found` (add the `e` flag to silence it).

## Recent files, buffers, symbols

| Key | Command | Action |
|---|---|---|
| `<Space>sh` | `:Recent` | Files recently opened in this project, across sessions |
| `<Space>sb` | `:Buffers` | Open buffers, current first then most recently used, `[+]` when modified |
| `<Space>S`, `<Space>sS` | `:Symbols [query]` | Workspace symbols, re-queried as you type |
| `<Space>o` | `:Outline` | The file's symbol tree (indented) |
| `<Space>sd` | `:Problems [all\|warnings\|errors]` | Every diagnostic the servers published, grouped by file |

Recent files are stored in `<data dir>/ovim/recent-files.json`
(`$OVIM_RECENT_FILES` overrides the path), keyed by the project's git root, and
remember the cursor where you left each file. Headless sessions (unless
`$OVIM_RECENT_FILES` is set) and projects under the system temp directory are
not recorded. In the Problems list `Ctrl-T`
cycles the severity filter and `Enter` jumps to the location. A server only
publishes diagnostics for documents it has open; Ovim keeps every loaded buffer
open on the server, so files you switched away from stay in the list until the
buffer is deleted.

Symbol rows read `<kind glyph> Name  kind · container      relative/file:line`, in
the language server's order.

## Completion

In insert mode the menu opens by itself after a server trigger character (such
as `.`) and after two identifier characters; `Ctrl-Space` (or `Ctrl-N`/`Ctrl-P`
when no menu is open) always asks. Rows are in the server's own order
(`sortText`), so the best guess is first; as you keep typing the list is refined
locally: prefix matches, then camelHump / word-initial matches (`gEm` finds
`getEmail`, `NPE` finds `NullPointerException`), then loose subsequences, with
the server's order kept inside each group. An lowercase pattern ignores case; a
capital demands one. A list the server marks `isIncomplete` is asked for again
as you type.

Each row reads `<kind> label details ...... description`: the kind glyph
(`m` method, `f` function, `C` class, `I` interface, `v` variable, `F` field,
`k` keyword, `s` snippet, ...), the label with the characters you typed
emphasised, the server's `labelDetails.detail` (a signature) right after it and
its `description` (for example the package) right-aligned and dimmed.
Deprecated items are struck through. The selected item's signature and
documentation appear in a popup beside the list (fetched with
`completionItem/resolve` when the server supports it).

`Enter`, `Ctrl-Y` accept the item and insert it at the cursor; `Tab` accepts it
and replaces the rest of the identifier under the cursor when the server offers
a replace range. `Ctrl-N`/`Ctrl-P` or the arrow keys move the selection, `Esc`
leaves insert mode. Accepting applies the item's extra edits (such as an
`import` line) in the same undo step, and a server's commit characters accept the
item you chose before the typed character is inserted.

Snippet completions (`foo(${1:arg})$0`) expand in place: the first tab stop is
selected, typing replaces its placeholder, `Tab` / `Shift-Tab` jump between tab
stops, and `Esc` leaves the snippet together with insert mode. A choice stop
(`${1|public,private|}`) opens a small chooser over the inserted first choice:
`Up`/`Down` move, `Enter` or `Tab` picks, typing narrows it or overwrites the
placeholder.

```vim
:set noautocomplete        " only open the menu with Ctrl-Space
:set autocompletemin=3     " identifier characters typed before it opens (default 2)
:set autocompletedelay=80  " milliseconds to wait for a pause in typing (default 40)
```

## Signature help and folding

Typing `(` or `,` in a call shows the signature above the line with the active
parameter highlighted and an `(n/m)` marker when there are overloads; every edit
or cursor move keeps it in sync, `Esc` or leaving the call dismisses it. Moving
the cursor back into an unfinished call (arrows, `Tab`) and accepting a method
completion bring it back.

Folds use Vim's commands: `zc`/`zo`/`za` (one level), `zC`/`zO`/`zA`
(recursive), `zR`/`zM` (all, and the fold level), `zr`/`zm` (one level less or
more folded), `zx`/`zX` (re-apply the level), `zv`, `zn`/`zN`/`zi`,
`zd`/`zD`/`zE`, `zj`/`zk`, `[z`/`]z` (start/end of the open fold) and
`zf{motion}` for manual folds. Automatic folds come from the language server
(`foldingRange`), else from the syntax tree (tree-sitter), else from
indentation. A closed fold is one line: `j`/`k` step over it, and `dd`, `yy`,
`>>`, `cc`, `x`, `D`, `p`, `o` and Visual selections that reach into it act on
all of it, as in Vim. The header shows `⋯ N lines`.

The fold column in the gutter (`:set foldcolumn=auto:1`, the default; `0` hides
it, `N` fixes the width) marks fold headers with `-` (open) or `+` (closed) and
the lines inside with `|`; click a mark to toggle the fold. It appears once a
buffer has folds.

Sources that only a package manager or the language server owns (JDK and
dependency sources, a class-file stub, `~/.cargo/registry`, `~/.gradle/caches`,
`~/.m2`) open with `modifiable` off: edits are refused with `E21` and `:w` fails;
`:set modifiable` overrides it.

`Ctrl-O` / `Ctrl-I` (Tab) walk the jump list across files, including jumps made
with `gd`, `gi` and `gr`. In the rename prompt and the `:` and `/` prompts
`Ctrl-U` deletes to the start, `Ctrl-W` the previous word, `Ctrl-H` is backspace
and `Ctrl-C` cancels.

## Breadcrumbs

The class and method around the cursor (`Circle › area`) show in the status
line, after the path in the GUI breadcrumb bar, and as `view.breadcrumbs` in the
headless snapshot. They come from the language server's document symbols and
fall back to tree-sitter (Rust, TypeScript/JavaScript, Python, Java, Kotlin)
until the server answers or when there is none.

## Git

| Key | Command | Action |
|---|---|---|
| `<Space>gs` / `<Space>gu` | `:GitStageHunk` / `:GitUnstageHunk` | Stage / unstage the change under the cursor |
| `<Space>gS` / `<Space>gU` | `:GitStage` / `:GitUnstage` | Stage / unstage the file (`:GitStageAll` stages everything) |
| `<Space>gc` | `:GitCommit` | Write a commit message; `:w`/`ZZ` commits, `:q!`/`ZQ` aborts |
| `<Space>gC` | `:GitAmend` | Amend the last commit, starting from its message |
| `<Space>gg` | `:GitStatus` | Changed files; `Enter` opens the diff, `Ctrl-T` stages/unstages, `Ctrl-E` edits |
| `<Space>gl` | `:GitLog` / `:GitLogAll` | History of the current file / of the repository |
| `<Space>gL` | `:GitLineLog` | History of the current line, followed back through edits |
| `<Space>gd` | `:GitDiff` | The branch diff review (see [Getting Started](getting-started.md)) |

Unsaved buffers are written before staging so git sees what you see. `Enter` on
a history entry shows that commit's diff in the diff review, on the file. The
message buffer lists staged, unstaged and untracked files as `#` comments (they
are dropped from the message); a failed commit (empty message, nothing staged,
unresolved conflicts, missing `user.name`) keeps the buffer open.

### Merge conflicts

`]n` / `[n` jump between conflict blocks (`<<<<<<<` ... `>>>>>>>`, default and
diff3 style). `<Space>gmo` keeps ours, `gmt` theirs, `gmb` both and `gmx`
neither for the block under or below the cursor (`:ConflictOurs`, `:ConflictTheirs`,
`:ConflictBoth`, `:ConflictNone`), each as one undo step. Then stage the file
(`<Space>gS`) and commit; a merge in progress is completed by the commit.
