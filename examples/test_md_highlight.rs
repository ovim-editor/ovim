// Test markdown syntax highlighting
// Run with: cargo run --example test_md_highlight

use ovim::syntax::{Language, LanguageRegistry, SyntaxHighlighter};

fn main() {
    println!("Testing markdown syntax highlighting...\n");

    // Test 1: Language detection
    let lang = LanguageRegistry::detect_from_path("README.md");
    println!("1. Language detection for README.md: {:?}", lang);

    if lang != Some(Language::Markdown) {
        println!("   ERROR: Expected Markdown, got {:?}", lang);
        return;
    }
    println!("   OK: Detected as Markdown");

    // Test 2: Create highlighter
    println!("\n2. Creating SyntaxHighlighter...");
    let highlighter_result = SyntaxHighlighter::new(Language::Markdown);

    match highlighter_result {
        Ok(mut highlighter) => {
            println!("   OK: Highlighter created successfully");

            // Test 3: Parse markdown content
            let test_content = r#"# Heading 1

This is a paragraph with **bold** and *italic* text.

## Heading 2

```rust
fn main() {
    println!("Hello");
}
```

- List item 1
- List item 2
"#;
            println!("\n3. Parsing markdown content...");
            highlighter.parse(test_content);

            // Test 4: Get highlights
            println!("\n4. Getting highlights for all lines...");
            let highlights = highlighter.highlights_for_all_lines(test_content);

            println!("   Total lines: {}", highlights.len());

            let mut total_highlights = 0;
            for (line_idx, line_highlights) in highlights.iter().enumerate() {
                if !line_highlights.is_empty() {
                    let line_text = test_content.lines().nth(line_idx).unwrap_or("");
                    println!("   Line {}: {:?}", line_idx, line_highlights);
                    println!("      Text: {:?}", line_text);
                    total_highlights += line_highlights.len();
                }
            }

            println!("\n   Total highlights found: {}", total_highlights);

            if total_highlights == 0 {
                println!("\n   WARNING: No highlights found!");
                println!("   This likely means the query patterns don't match the grammar nodes.");

                // Debug: show the query
                println!("\n   Query being used:");
                let query_source = LanguageRegistry::get_highlight_query(Language::Markdown);
                for (i, line) in query_source.lines().enumerate() {
                    println!("      {}: {}", i + 1, line);
                }
            }
        }
        Err(e) => {
            println!("   ERROR: Failed to create highlighter: {}", e);
            println!("\n   This is the root cause - the query is likely invalid for this grammar.");
        }
    }
}
