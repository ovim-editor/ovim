# Pseudocode reading view

Use `:set pseudo` (or `:set pseudocode`) in a Java or Markdown buffer to read
its contents with declaration and formatting noise concealed. The view is built
from the current buffer, including unsaved changes. Your source text and undo
history remain intact.

```vim
:set pseudo
:set nopseudo
:set pseudo!
:set pseudo?
:set pseudo=on
:set pseudo=off
```

Use `!` to toggle and `?` to query. Explicit values also accept `true` / `false`
and `1` / `0`. Both `pseudo` and `pseudocode` accept these forms. The setting applies to the
current reading view, not automatically to every file you open.

Navigate with normal motions, counts, scrolling, and `/` or `?` search. Click to
position the cursor. Press **Enter** to open the corresponding source location,
**r** to refresh from source, or **q** / `:set nopseudo` to return to the original
buffer and cursor. `:set pseudo` reuses the view when you open it again.

The generated buffer is a reading surface. Edit, paste, and save operations
belong in the source buffer; press Enter to get there. If the source changes
while the view is open, Enter first refreshes it rather than using stale
coordinates. Disabling the view still returns to source immediately.

## Java

For example:

```java
public class Totals {
    public static int sum(final List<Integer> values) {
        int total = 0;
        for (Integer value : values) {
            total += value;
        }
        return total;
    }
}
```

appears as:

```text
class Totals:
    sum(values):
        total = 0
        for value in values:
            total += value
        return total
```

The syntax-tree transformation conceals package/import declarations, annotations,
access and declaration modifiers, declared types, generic parameters/arguments,
inheritance clauses, throws clauses, casts, and ordinary block delimiters.
Constructors retain their names; array allocation displays as `array[size]`.
Meaningful operators, constructor calls, `instanceof` tests, array literals,
comments, and string contents remain visible. Empty bodies and inline block
grouping retain their braces so statement boundaries stay clear.
This is a reading aid, not a semantically equivalent program or a translation
intended to run.

Syntax-only lines disappear. Leading/trailing blank lines and repeated empty
lines are compacted, while paragraph separation and source indentation remain.
Whitespace inside multiline literals, comments, and other verbatim regions is
preserved. Incomplete or unrecognized syntax is left visible rather than guessed.

## Markdown

In the terminal, heading markers, emphasis delimiters, inline-code delimiters,
and link syntax are concealed. Headings and bold/italic text retain their styles.
Lists, tasks, quotes, and tables keep their structural markers. Java fenced code
uses the Java transformation; other languages' fenced code, indented code, and
HTML blocks remain verbatim. Ordinary repeated blank lines are compacted.

In **ovim-gui**, Markdown appears as a formatted document: headings, emphasis,
lists, task checkboxes, tables, quotes, and syntax-colored code blocks. Java code
blocks use the same pseudocode transformation as the terminal. Hard line breaks
and code whitespace are preserved. The document follows the editor theme and
scrolls naturally, including wide tables and code blocks in narrow panes.

Click a block (or a code line), then press **Enter** to open its source location.
Normal motions and search still select locations in the underlying reading view.
External HTTP(S)/email links open normally; heading links navigate within the
document. Images appear as descriptive links, and raw HTML is shown literally.
Use **r** to refresh after changing source, and **q** or `:set nopseudo` to return.
Source highlighting and navigation share the same source map in both frontends.

## Lua

With a supported source buffer current:

```lua
ovim.opt.pseudo = true
ovim.opt.pseudocode = false
```

`vim.opt` supports the same boolean assignments. Pseudocode currently supports
Java and Markdown documents up to 2 MiB.
