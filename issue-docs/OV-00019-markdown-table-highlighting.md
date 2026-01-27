# OV-00019: Syntax highlighting for markdown tables

**Status:** Pending | **Priority:** LOW | **Complexity:** Medium

## Problem

Markdown tables get no syntax highlighting. The `|`, header separator (`---`), and alignment markers (`:---:`) all render as plain text.

```markdown
| Name   | Age | Role      |
|--------|-----|-----------|
| Alice  | 30  | Engineer  |
| Bob    | 25  | Designer  |
```

This is because `tree_sitter_md::LANGUAGE` is the CommonMark block grammar, which does not parse GFM (GitHub Flavored Markdown) extensions like tables. The query file (`src/syntax/queries/markdown.scm:40-41`) documents this limitation.

## What should be highlighted

| Element | Example | Highlight group | Rationale |
|---------|---------|-----------------|-----------|
| Pipe delimiters | `\|` | `@punctuation.special` | Structural punctuation, same as list markers |
| Header separator row | `\|---\|:---:\|` | `@punctuation.special` | Structural, not content |
| Alignment colons | `:` in `:---:` | `@punctuation.special` | Part of separator syntax |
| Header cell content | `Name`, `Age` | `@markup.heading` | Table headers are semantically headings |
| Body cell content | `Alice`, `30` | (default) | No special highlighting needed |

## Approach: Regex overlay (recommended)

The codebase already has a pattern for overlay highlighting that doesn't come from tree-sitter: `CodeBlockCache` in `src/syntax/code_blocks.rs` walks the tree to find fenced code blocks, then applies language-specific highlights on top of the base markdown highlights.

Tables can use a similar overlay approach, but regex-based since the tree doesn't contain table nodes:

### Detection

A markdown table is a contiguous block of lines where each line:
- Starts with optional whitespace + `|`
- Contains at least one more `|`
- The second line matches `^\s*\|[\s:|-]+\|` (the separator row)

This is the same heuristic GitHub and most markdown renderers use.

### Implementation

1. **`TableHighlightCache`** — new struct in `src/syntax/` (or extend `code_blocks.rs` to `overlays.rs`)
   - Scans the buffer text for table blocks using the regex pattern above
   - For each table, generates highlight ranges:
     - All `|` characters → `HighlightGroup::PunctuationSpecial`
     - Separator row dashes/colons → `HighlightGroup::PunctuationSpecial`
     - Header row cell content → `HighlightGroup::MarkupHeading`
   - Cached and invalidated on buffer version change (same as `CodeBlockCache`)

2. **Integration point** — `src/buffer/highlighting.rs`
   - Currently the priority chain is: code block cache → LSP semantic → tree-sitter
   - Add table highlights at the same level as code block cache (or just after)
   - Table highlights only apply to lines identified as table rows

3. **No tree-sitter changes needed** — this is purely additive

### Alternative: Switch to GFM grammar

`tree-sitter-md` does ship a separate GFM grammar that parses tables as `pipe_table` nodes. However:
- It's a different grammar (`tree_sitter_md::GFM_LANGUAGE` or similar) — would need to check API compatibility
- Switching the parser changes all markdown parsing, not just tables
- The regex approach is simpler and self-contained
- If we switch to GFM later, the table overlay becomes unnecessary and can be removed cleanly

The regex approach is lower risk and can be done without touching the parser.

## Files

- `src/syntax/code_blocks.rs` or new `src/syntax/table_highlights.rs` — table detection + highlight generation
- `src/buffer/highlighting.rs` — integrate table highlights into priority chain
- `src/syntax/mod.rs` — export new module

## Testing

- Unit test: detect table block in markdown text, verify line ranges
- Unit test: pipe characters get `PunctuationSpecial` highlight
- Unit test: header row content gets `MarkupHeading` highlight
- Unit test: separator row dashes get `PunctuationSpecial`
- Unit test: non-table content with `|` (e.g., in code blocks) is not highlighted as table
- Unit test: table at start/end of file, single-row table (header + separator only)
