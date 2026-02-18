#!/usr/bin/env bash
# Test Java syntax highlighting initialization

set -euo pipefail

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ovim-java-syntax.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT
SRC_FILE="$TMP_DIR/test_syntax.rs"
BIN_FILE="$TMP_DIR/test_syntax"

echo "=== Testing Java Syntax Highlighting ==="
echo ""

# Create a simple Rust test program to verify syntax highlighting
cat > "$SRC_FILE" <<'EOF'
use ovim::syntax::{LanguageRegistry, SyntaxHighlighter};

fn main() {
    let test_file = "TestJava.java";

    println!("Testing syntax highlighting for: {}", test_file);
    println!("");

    // Detect language
    let lang = match LanguageRegistry::detect_from_path(test_file) {
        Some(l) => {
            println!("✓ Detected language: {:?}", l);
            l
        },
        None => {
            println!("✗ Failed to detect language");
            std::process::exit(1);
        }
    };

    // Try to create syntax highlighter
    match SyntaxHighlighter::new(lang) {
        Ok(_highlighter) => {
            println!("✓ Syntax highlighter initialized successfully");
            println!("✓ Tree-sitter parser created");
            println!("✓ Highlight queries loaded");
        },
        Err(e) => {
            println!("✗ Failed to create syntax highlighter: {}", e);
            std::process::exit(1);
        }
    }

    println!("");
    println!("=== Syntax highlighting test passed! ===");
}
EOF

echo "Compiling test program..."
rustc --edition 2021 -L target/debug/deps --extern ovim=target/debug/libovim.rlib "$SRC_FILE" -o "$BIN_FILE"

echo "Running syntax highlighting test..."
echo ""
"$BIN_FILE"

echo ""
echo "=== Java syntax highlighting working correctly! ==="
