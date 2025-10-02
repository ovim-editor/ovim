use ovim::buffer::Buffer;

#[test]
fn test_rust_syntax_highlighting_enabled() {
    // Create a temporary file
    let temp_file = "/tmp/test_syntax.rs";
    std::fs::write(temp_file, "fn main() {\n    let x = 42;\n}\n").unwrap();

    // Load the file into a buffer
    let buffer = Buffer::load_file(temp_file).unwrap();

    // Check if syntax highlighting is enabled
    assert!(buffer.has_syntax_highlighting(), "Syntax highlighting should be enabled for .rs files");

    // Clean up
    std::fs::remove_file(temp_file).unwrap();
}

#[test]
fn test_rust_syntax_highlights_for_line() {
    // Create a temporary file with Rust code
    let temp_file = "/tmp/test_syntax2.rs";
    std::fs::write(temp_file, "fn main() {\n    let x = 42;\n    println!(\"Hello\");\n}\n").unwrap();

    // Load the file into a buffer
    let buffer = Buffer::load_file(temp_file).unwrap();

    // Get highlights for the first line (should have "fn" as a keyword)
    let highlights = buffer.highlights_for_line(0);

    println!("Highlights for line 0: {:?}", highlights);
    assert!(!highlights.is_empty(), "Line 0 should have syntax highlights (contains 'fn main')");

    // Get highlights for line 1 (should have "let" as a keyword and "42" as a number)
    let highlights = buffer.highlights_for_line(1);

    println!("Highlights for line 1: {:?}", highlights);
    assert!(!highlights.is_empty(), "Line 1 should have syntax highlights (contains 'let x = 42')");

    // Clean up
    std::fs::remove_file(temp_file).unwrap();
}
