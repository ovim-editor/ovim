# Editor Options

Configure ovim behavior with `:set` commands.

## Text Width

Control the maximum width of text content. Useful for prose editing where you don't want text stretching to the terminal edges.

```vim
:set textwidth=80    " Limit content to 80 columns, centered
:set tw=80           " Short form
:set textwidth=0     " Disable (use full terminal width)
:set textwidth?      " Show current value
```

When set, the buffer content is horizontally centered with margins on both sides. The gutter (line numbers, git signs) stays attached to the text.

**Minimum value:** 20 columns (or 0 to disable)

## Line Numbers

```vim
:set number          " Show absolute line numbers
:set nu              " Short form
:set nonumber        " Hide line numbers
:set nonu

:set relativenumber  " Show relative line numbers
:set rnu
:set norelativenumber
:set nornu
```

## Tabs and Indentation

```vim
:set tabstop=4       " Tab display width (1-16)
:set ts=4

:set shiftwidth=4    " Indent width for >> and << (1-16)
:set sw=4

:set expandtab       " Insert spaces instead of tabs
:set et
:set noexpandtab     " Insert actual tab characters
:set noet
```

## Scrolling

```vim
:set scroll=10       " Lines to scroll with Ctrl-D/Ctrl-U
```

## Querying Options

Add `?` to any option name to see its current value:

```vim
:set textwidth?      " Shows: textwidth=80
:set tabstop?        " Shows: tabstop=4
:set number?         " Shows: number or nonumber
```
