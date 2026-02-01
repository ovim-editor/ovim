use ovim::syntax::{Language, SyntaxHighlighter};

#[test]
fn test_javascript_highlighter() {
    match SyntaxHighlighter::new(Language::JavaScript) {
        Ok(mut highlighter) => {
            println!("✓ JavaScript SyntaxHighlighter created successfully");
            let source = "function test() {\n  const x = 42;\n}";
            highlighter.parse(source);
            let highlights = highlighter.highlights_for_line(0, source);
            println!("JavaScript highlights for line 0: {:?}", highlights);
            assert!(!highlights.is_empty(), "Should have highlights");
        }
        Err(e) => {
            println!("✗ Failed to create JavaScript highlighter: {}", e);
            panic!("Failed: {}", e);
        }
    }
}

#[test]
fn test_python_highlighter() {
    match SyntaxHighlighter::new(Language::Python) {
        Ok(mut highlighter) => {
            println!("✓ Python SyntaxHighlighter created successfully");
            let source = "def test():\n    x = 42\n";
            highlighter.parse(source);
            let highlights = highlighter.highlights_for_line(0, source);
            println!("Python highlights for line 0: {:?}", highlights);
            assert!(!highlights.is_empty(), "Should have highlights");
        }
        Err(e) => {
            println!("✗ Failed to create Python highlighter: {}", e);
            panic!("Failed: {}", e);
        }
    }
}
