#!/bin/bash
# Test LSP language detection for Java files

set -e

echo "=== Testing LSP Language Detection ==="
echo ""

# Create a Rust test program
cat > /tmp/test_lsp_detect.rs <<'EOF'
use ovim::syntax::LanguageRegistry;

fn main() {
    // Test various Java file paths
    let test_files = vec![
        "TestJava.java",
        "java_test_project/src/main/java/com/example/HelloWorld.java",
        "/workspace/java_test_project/src/main/java/com/example/HelloWorld.java",
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
rustc --edition 2021 -L target/debug/deps --extern ovim=target/debug/libovim.rlib /tmp/test_lsp_detect.rs -o /tmp/test_lsp_detect

echo "Running LSP detection tests..."
echo ""
/tmp/test_lsp_detect

echo ""
echo "=== LSP detection working correctly! ==="
