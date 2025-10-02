# Tree-sitter Syntax Highlighting Implementation Plan

## Overview

Adding tree-sitter based syntax highlighting to ovim for fast, accurate, incremental highlighting.

## Architecture

### Components

```
src/syntax/
├── mod.rs              # Main syntax module
├── highlighter.rs      # SyntaxHighlighter implementation
├── languages.rs        # Language detection & registry
├── theme.rs            # Color theme mapping
└── queries/            # Highlight query files (or embed them)
    ├── rust.scm
    ├── javascript.scm
    └── python.scm
```

### Core Types

```rust
// src/syntax/highlighter.rs
pub struct SyntaxHighlighter {
    language: Language,              // tree-sitter Language
    parser: Parser,                  // tree-sitter Parser
    tree: Option<Tree>,              // Current syntax tree
    highlight_config: HighlightConfig, // Queries
}

impl SyntaxHighlighter {
    pub fn new(language: Language, highlight_query: &str) -> Self;
    pub fn parse(&mut self, source: &str);
    pub fn update(&mut self, edit: InputEdit, source: &str);
    pub fn highlights_for_line(&self, line_idx: usize) -> Vec<(Range, HighlightGroup)>;
}

// src/syntax/languages.rs
pub enum SupportedLanguage {
    Rust,
    JavaScript,
    TypeScript,
    Python,
    Markdown,
}

pub struct LanguageRegistry;

impl LanguageRegistry {
    pub fn detect_from_path(path: &str) -> Option<SupportedLanguage>;
    pub fn get_language(lang: SupportedLanguage) -> Language;
    pub fn get_highlight_query(lang: SupportedLanguage) -> &'static str;
}

// src/syntax/theme.rs
#[derive(Debug, Clone, Copy)]
pub enum HighlightGroup {
    Keyword,
    Function,
    Type,
    String,
    Number,
    Comment,
    Operator,
    Variable,
    // ... more groups
}

pub struct Theme {
    colors: HashMap<HighlightGroup, Color>,
}

impl Theme {
    pub fn default() -> Self; // Nice default theme
    pub fn get_color(&self, group: HighlightGroup) -> Color;
}
```

### Integration with Buffer

```rust
// src/buffer/mod.rs
pub struct Buffer {
    rope: Rope,
    cursor: Cursor,
    modified: bool,
    file_path: Option<String>,
    syntax: Option<SyntaxHighlighter>, // NEW!
}

impl Buffer {
    pub fn enable_syntax_highlighting(&mut self) {
        if let Some(path) = &self.file_path {
            if let Some(lang) = LanguageRegistry::detect_from_path(path) {
                let highlighter = SyntaxHighlighter::new(
                    LanguageRegistry::get_language(lang),
                    LanguageRegistry::get_highlight_query(lang),
                );
                highlighter.parse(&self.rope.to_string());
                self.syntax = Some(highlighter);
            }
        }
    }

    pub fn update_syntax(&mut self, edit: TreeSitterEdit) {
        if let Some(ref mut syntax) = self.syntax {
            syntax.update(edit, &self.rope.to_string());
        }
    }

    pub fn highlights_for_line(&self, line_idx: usize) -> Vec<(Range, HighlightGroup)> {
        self.syntax.as_ref()
            .map(|s| s.highlights_for_line(line_idx))
            .unwrap_or_default()
    }
}
```

### Integration with Renderer

```rust
// src/ui/renderer.rs
impl Renderer {
    pub fn render(&mut self, editor: &Editor) -> Result<()> {
        // ... existing code ...

        // For each visible line:
        let highlights = editor.buffer().highlights_for_line(line_idx);

        // Apply colors when rendering:
        for (range, group) in highlights {
            let color = theme.get_color(group);
            // Use ratatui/crossterm to set color for range
            spans.push(Span::styled(text, Style::default().fg(color)));
        }
    }
}
```

### Handling Edits

When a Change is applied, we need to update the syntax tree:

```rust
// In Change::apply() or after edits
fn convert_change_to_edit(change: &Change, rope: &Rope) -> InputEdit {
    let start_byte = rope.line_to_byte(change.start.line) + change.start.col;
    let old_end_byte = rope.line_to_byte(change.old_end.line) + change.old_end.col;
    let new_end_byte = start_byte + change.new_text.len();

    InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_position: Point::new(change.start.line, change.start.col),
        old_end_position: Point::new(change.old_end.line, change.old_end.col),
        new_end_position: Point::new(change.new_end.line, change.new_end.col),
    }
}
```

## Dependencies

```toml
[dependencies]
tree-sitter = "0.20"
tree-sitter-rust = "0.20"
tree-sitter-javascript = "0.20"
tree-sitter-typescript = "0.20"
tree-sitter-python = "0.20"
tree-sitter-markdown = "0.20"
```

## Implementation Phases

### Phase 1: Core Infrastructure ✅
- [x] Add tree-sitter dependencies
- [x] Create syntax module structure
- [x] Implement basic SyntaxHighlighter
- [x] Add language detection

### Phase 2: Single Language (Rust) ✅
- [x] Implement Rust syntax highlighting
- [x] Embed highlight queries
- [x] Create basic theme
- [x] Integrate with Buffer

### Phase 3: Rendering ✅
- [x] Modify renderer to use highlights
- [x] Apply colors via ratatui
- [x] Test with Rust files

### Phase 4: Incremental Updates ✅
- [x] Convert Changes to InputEdits
- [x] Update tree on edits
- [x] Verify highlighting updates correctly

### Phase 5: Multiple Languages 🔄
- [ ] Add JavaScript/TypeScript
- [ ] Add Python
- [ ] Add Markdown
- [ ] Test language detection

### Phase 6: Polish 🔜
- [ ] Better theme (support dark/light)
- [ ] Performance optimization for large files
- [ ] Async parsing for very large files
- [ ] Configuration (:set syntax=rust)

## Highlight Queries

Example for Rust (`queries/rust.scm`):

```scheme
; Keywords
["fn" "let" "mut" "const" "static" "pub" "use" "mod" "struct" "enum" "impl" "trait"] @keyword

; Functions
(function_item name: (identifier) @function)
(call_expression function: (identifier) @function)

; Types
(type_identifier) @type
(primitive_type) @type.builtin

; Strings
(string_literal) @string
(char_literal) @string

; Numbers
(integer_literal) @number
(float_literal) @number

; Comments
(line_comment) @comment
(block_comment) @comment

; Operators
["=" "+" "-" "*" "/" "%" "==" "!=" "<" ">" "<=" ">=" "&&" "||" "!"] @operator

; Macros
(macro_invocation macro: (identifier) @macro)
```

## Color Theme

Default theme mapping:

```rust
HighlightGroup::Keyword     -> Color::Magenta (bold)
HighlightGroup::Function    -> Color::Blue
HighlightGroup::Type        -> Color::Yellow
HighlightGroup::String      -> Color::Green
HighlightGroup::Number      -> Color::Cyan
HighlightGroup::Comment     -> Color::DarkGray (italic)
HighlightGroup::Operator    -> Color::White
HighlightGroup::Variable    -> Color::White
HighlightGroup::Macro       -> Color::Magenta
```

## Performance Considerations

1. **Incremental parsing** - Tree-sitter's strength
   - Only reparse changed regions
   - O(log n) update complexity

2. **Lazy highlighting** - Only highlight visible lines
   - Renderer only queries visible range
   - Don't compute highlights for off-screen text

3. **Caching** - Cache highlight results per line
   - Invalidate on edits
   - Reuse for unchanged lines

4. **Async parsing** - For very large files
   - Parse in background thread
   - Show partial highlights while parsing
   - Use tokio runtime (already have it)

## Testing Strategy

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_rust_highlighting() {
        let source = r#"
fn main() {
    let x = 42;
    println!("Hello");
}
"#;
        let mut hl = SyntaxHighlighter::new_rust();
        hl.parse(source);

        let highlights = hl.highlights_for_line(0);
        assert!(highlights.iter().any(|(_, g)| matches!(g, HighlightGroup::Keyword)));
    }

    #[test]
    fn test_incremental_update() {
        // Test that edits update tree correctly
    }
}
```

## API Testing via REST

Can test via REST API:

```bash
# Load Rust file
curl -X PUT http://localhost:$PORT/buffer \
  -d '{"content": "fn main() { let x = 42; }"}'

# Get snapshot (should include syntax info)
curl http://localhost:$PORT/snapshot | jq '.buffer.highlights'
```

## Future Enhancements

1. **More languages** - C/C++, Go, Java, etc.
2. **Custom themes** - Load from file
3. **Semantic highlighting** - Use LSP for even better highlighting
4. **Treesitter queries for other features**:
   - Text objects (function text object, class text object)
   - Code folding
   - Indentation detection
   - Code navigation (jump to definition via tree)

## Notes

- Start simple: Rust only, embedded queries, basic theme
- Prove it works end-to-end
- Then expand to more languages
- Tree-sitter is battle-tested (used in Neovim, Helix, Zed)
- Queries can be copied from existing editors
