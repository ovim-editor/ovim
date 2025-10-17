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

#[test]
fn test_highlights_for_all_lines_matches_per_line() {
    let mut highlighter = SyntaxHighlighter::new(Language::Rust).expect("Failed to create highlighter");

    let source = "fn main() {\n    let x = 42;\n    println!(\"test\");\n}";
    highlighter.parse(source);

    // Get highlights using the new efficient method
    let all_highlights = highlighter.highlights_for_all_lines(source);

    // Get highlights using the old per-line method
    let lines: Vec<&str> = source.lines().collect();
    let mut per_line_highlights = Vec::new();
    for line_idx in 0..lines.len() {
        per_line_highlights.push(highlighter.highlights_for_line(line_idx, source));
    }

    // Compare results
    assert_eq!(all_highlights.len(), per_line_highlights.len(),
        "Should have same number of lines");

    for (line_idx, (all_line, per_line)) in all_highlights.iter().zip(per_line_highlights.iter()).enumerate() {
        assert_eq!(all_line, per_line,
            "Highlights for line {} should match between methods", line_idx);
    }

    println!("✓ highlights_for_all_lines produces same results as per-line highlighting");
}

#[test]
fn test_highlights_for_all_lines_performance() {
    use std::time::Instant;

    let mut highlighter = SyntaxHighlighter::new(Language::Rust).expect("Failed to create highlighter");

    // Create a larger source file for performance testing
    let mut source = String::new();
    for i in 0..100 {
        source.push_str(&format!("fn function_{}() {{\n", i));
        source.push_str(&format!("    let x = {};\n", i));
        source.push_str(&format!("    let y = \"string_{}\";\n", i));
        source.push_str("    println!(\"test\");\n");
        source.push_str("}\n\n");
    }

    highlighter.parse(&source);

    // Measure new method
    let start = Instant::now();
    let all_highlights = highlighter.highlights_for_all_lines(&source);
    let all_lines_duration = start.elapsed();

    // Measure old method
    let start = Instant::now();
    let lines: Vec<&str> = source.lines().collect();
    let mut per_line_highlights = Vec::new();
    for line_idx in 0..lines.len() {
        per_line_highlights.push(highlighter.highlights_for_line(line_idx, &source));
    }
    let per_line_duration = start.elapsed();

    println!("Performance comparison (100 functions, ~600 lines):");
    println!("  highlights_for_all_lines: {:?}", all_lines_duration);
    println!("  per-line highlighting:    {:?}", per_line_duration);
    println!("  Speedup: {:.2}x", per_line_duration.as_secs_f64() / all_lines_duration.as_secs_f64());

    // Verify the new method is faster (should be significantly faster)
    assert!(all_lines_duration < per_line_duration,
        "highlights_for_all_lines should be faster than per-line");

    // Verify results still match
    assert_eq!(all_highlights.len(), per_line_highlights.len());
}
