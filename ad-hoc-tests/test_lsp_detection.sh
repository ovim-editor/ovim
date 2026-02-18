#!/usr/bin/env bash
# Test LSP language detection for Java files

set -euo pipefail

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ovim-lsp-detect.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT
SRC_FILE="$TMP_DIR/test_lsp_detect.rs"
BIN_FILE="$TMP_DIR/test_lsp_detect"

echo "=== Testing LSP Language Detection ==="
echo ""

# Create a Rust test program
cat > "$SRC_FILE" <<'EOF'
use ovim::syntax::LanguageRegistry;

fn main() {
    // Test various Java file paths
    let test_files = vec![
        "TestJava.java",
        "java_test_project/src/main/java/com/example/HelloWorld.java",
        "java_test_project/src/main/java/com/example/HelloWorld.java",
    ];

    println!("Testing LSP language detection for Java files:");
    println!("");

    let mut all_passed = true;

    for file_path in test_files {
        print!("Testing {}: ", file_path);

        match LanguageRegistry::get_lsp_language_id(file_path) {
            Some("java") => {
                println!("✓ Detected as 'java'");
            },
            Some(other) => {
                println!("✗ Detected as '{}' (expected 'java')", other);
                all_passed = false;
            },
            None => {
                println!("✗ NOT detected (expected 'java')");
                all_passed = false;
            }
        }
    }

    println!("");

    if all_passed {
        println!("=== All LSP detection tests passed! ===");
    } else {
        println!("=== Some tests FAILED ===");
        std::process::exit(1);
    }
}
EOF

echo "Compiling test program..."
rustc --edition 2021 -L target/debug/deps --extern ovim=target/debug/libovim.rlib "$SRC_FILE" -o "$BIN_FILE"

echo "Running LSP detection tests..."
echo ""
"$BIN_FILE"

echo ""
echo "=== LSP detection working correctly! ==="
