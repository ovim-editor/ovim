use ovim::syntax::{SyntaxHighlighter, Language};

#[test]
fn test_syntax_highlighter_creation() {
    let result = SyntaxHighlighter::new(Language::Rust);
    match &result {
        Ok(_) => println!("✓ SyntaxHighlighter created successfully for Rust"),
        Err(e) => println!("✗ Failed to create SyntaxHighlighter for Rust: {}", e),
    }
    assert!(result.is_ok(), "Should be able to create Rust syntax highlighter");
}

#[test]
fn test_syntax_highlighter_parse_and_highlight() {
    let mut highlighter = SyntaxHighlighter::new(Language::Rust).expect("Failed to create highlighter");

    let source = "fn main() {\n    let x = 42;\n}";
    highlighter.parse(source);

    let highlights = highlighter.highlights_for_line(0, source);
    println!("Highlights for 'fn main()': {:?}", highlights);

    assert!(!highlights.is_empty(), "Should have highlights for 'fn main()'");
}
