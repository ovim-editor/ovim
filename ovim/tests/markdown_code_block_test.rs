//! Test syntax highlighting inside markdown code blocks

use ovim::buffer::Buffer;
use ovim::syntax::HighlightGroup;

#[test]
fn test_rust_code_block_has_rust_highlights() {
    let content = r#"# Test

```rust
fn main() {
    let x = 42;
}
```
"#;

    let mut buffer = Buffer::new_from_str(content);
    buffer.set_file_path("test.md".to_string());
    buffer.enable_syntax_highlighting();

    // Line 3: "fn main() {" - should have Rust highlights
    let highlights = buffer.highlights_for_line(3);
    let has_keyword = highlights
        .iter()
        .any(|(_, g)| *g == HighlightGroup::Keyword);
    let has_function = highlights
        .iter()
        .any(|(_, g)| *g == HighlightGroup::Function);
    assert!(
        has_keyword || has_function,
        "Line 3 should have Rust keyword or function highlights, got: {:?}",
        highlights
    );

    // Line 4: "    let x = 42;" - should have keyword and number/constant
    let highlights = buffer.highlights_for_line(4);
    let has_keyword = highlights
        .iter()
        .any(|(_, g)| *g == HighlightGroup::Keyword);
    // Rust tree-sitter uses Constant for integer literals, not Number
    let has_number_or_constant = highlights
        .iter()
        .any(|(_, g)| *g == HighlightGroup::Number || *g == HighlightGroup::Constant);
    assert!(
        has_keyword,
        "Line 4 should have 'let' keyword highlight, got: {:?}",
        highlights
    );
    assert!(
        has_number_or_constant,
        "Line 4 should have '42' number/constant highlight, got: {:?}",
        highlights
    );
}

#[test]
fn test_bash_code_block_has_bash_highlights() {
    let content = r#"# Script

```bash
echo "hello"
if [ -f test ]; then
    cat test
fi
```
"#;

    let mut buffer = Buffer::new_from_str(content);
    buffer.set_file_path("test.md".to_string());
    buffer.enable_syntax_highlighting();

    // Line 3: "echo \"hello\"" - should have string highlight
    let highlights = buffer.highlights_for_line(3);
    let has_string = highlights.iter().any(|(_, g)| *g == HighlightGroup::String);
    assert!(
        has_string,
        "Line 3 should have string highlight for 'hello', got: {:?}",
        highlights
    );
}

#[test]
fn test_python_code_block_has_python_highlights() {
    let content = r#"# Python

```python
def hello():
    return 42
```
"#;

    let mut buffer = Buffer::new_from_str(content);
    buffer.set_file_path("test.md".to_string());
    buffer.enable_syntax_highlighting();

    // Line 3: "def hello():" - should have keyword highlight
    let highlights = buffer.highlights_for_line(3);
    let has_keyword = highlights
        .iter()
        .any(|(_, g)| *g == HighlightGroup::Keyword);
    assert!(
        has_keyword,
        "Line 3 should have 'def' keyword highlight, got: {:?}",
        highlights
    );
}

#[test]
fn test_unknown_language_falls_back_to_markup_raw() {
    let content = r#"# Test

```unknownlang
some code here
```
"#;

    let mut buffer = Buffer::new_from_str(content);
    buffer.set_file_path("test.md".to_string());
    buffer.enable_syntax_highlighting();

    // Line 3: "some code here" - should have MarkupRaw (green) from base markdown
    let highlights = buffer.highlights_for_line(3);
    let has_markup_raw = highlights
        .iter()
        .any(|(_, g)| *g == HighlightGroup::MarkupRaw);
    assert!(
        has_markup_raw,
        "Unknown language should fall back to MarkupRaw highlight, got: {:?}",
        highlights
    );
}

#[test]
fn test_text_outside_code_block_has_markdown_highlights() {
    let content = r#"# Heading

Regular text.

```rust
fn main() {}
```

More text.
"#;

    let mut buffer = Buffer::new_from_str(content);
    buffer.set_file_path("test.md".to_string());
    buffer.enable_syntax_highlighting();

    // Line 0: "# Heading" - should have heading highlight
    let highlights = buffer.highlights_for_line(0);
    let has_heading = highlights
        .iter()
        .any(|(_, g)| *g == HighlightGroup::MarkupHeading);
    assert!(
        has_heading,
        "Line 0 should have heading highlight, got: {:?}",
        highlights
    );
}

#[test]
fn test_multiple_code_blocks_different_languages() {
    let content = r#"# Multiple Languages

```rust
let x = 1;
```

```python
x = 1
```
"#;

    let mut buffer = Buffer::new_from_str(content);
    buffer.set_file_path("test.md".to_string());
    buffer.enable_syntax_highlighting();

    // Line 3: "let x = 1;" - Rust keyword
    let rust_highlights = buffer.highlights_for_line(3);
    let has_rust_keyword = rust_highlights
        .iter()
        .any(|(_, g)| *g == HighlightGroup::Keyword);
    assert!(
        has_rust_keyword,
        "Rust block should have keyword highlight, got: {:?}",
        rust_highlights
    );

    // Line 7: "x = 1" - Python doesn't highlight 'x' as keyword
    // but should still have number highlight
    let python_highlights = buffer.highlights_for_line(7);
    let has_number = python_highlights
        .iter()
        .any(|(_, g)| *g == HighlightGroup::Number);
    assert!(
        has_number,
        "Python block should have number highlight for '1', got: {:?}",
        python_highlights
    );
}
