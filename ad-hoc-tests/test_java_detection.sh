#!/usr/bin/env bash
# Test Java language detection

set -euo pipefail

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ovim-java-detect.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT
SRC_FILE="$TMP_DIR/test_lang_detect.rs"
BIN_FILE="$TMP_DIR/test_lang_detect"

echo "=== Testing Java Language Detection ==="
echo ""

# Create a simple Rust test program to verify language detection
cat > "$SRC_FILE" <<'EOF'
use std::path::Path;

// Import from ovim crate
use ovim::syntax::LanguageRegistry;

fn main() {
    let test_file = "TestJava.java";

    println!("Testing file: {}", test_file);
    println!("");

    // Test 1: Detect language from path
    match LanguageRegistry::detect_from_path(test_file) {
        Some(lang) => println!("✓ Language detected: {:?}", lang),
        None => {
            println!("✗ Language NOT detected");
            std::process::exit(1);
        }
    }

    // Test 2: Get LSP language ID
    match LanguageRegistry::get_lsp_language_id(test_file) {
        Some(id) => println!("✓ LSP language ID: {}", id),
        None => {
            println!("✗ LSP language ID NOT found");
            std::process::exit(1);
        }
    }

    // Test 3: Check LSP support
    if LanguageRegistry::has_lsp_support(test_file) {
        println!("✓ LSP support confirmed");
    } else {
        println!("✗ LSP support NOT available");
        std::process::exit(1);
    }

    println!("");
    println!("=== All tests passed! ===");
}
EOF

echo "Compiling test program..."
rustc --edition 2021 -L target/debug/deps --extern ovim=target/debug/libovim.rlib "$SRC_FILE" -o "$BIN_FILE"

echo "Running tests..."
echo ""
"$BIN_FILE"

echo ""
echo "=== Java detection working correctly! ==="
