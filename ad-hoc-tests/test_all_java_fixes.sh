#!/usr/bin/env bash
# Comprehensive test for all Java fixes

set -euo pipefail

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ovim-java-fixes.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "╔════════════════════════════════════════════════════════════════╗"
echo "║          Java LSP & Syntax Highlighting - Final Tests         ║"
echo "╚════════════════════════════════════════════════════════════════╝"
echo ""

# Test 1: Language Detection
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 1: Language Detection"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
if ./test_java_detection.sh > "$TMP_DIR/test1.log" 2>&1; then
    echo "✅ PASSED: Language detection working"
else
    echo "❌ FAILED: Language detection not working"
    cat "$TMP_DIR/test1.log"
    exit 1
fi
echo ""

# Test 2: Syntax Highlighting
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 2: Syntax Highlighting"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
if ./test_java_syntax.sh > "$TMP_DIR/test2.log" 2>&1; then
    echo "✅ PASSED: Syntax highlighting working"
else
    echo "❌ FAILED: Syntax highlighting not working"
    cat "$TMP_DIR/test2.log"
    exit 1
fi
echo ""

# Test 3: LSP Language ID
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 3: LSP Language ID Detection"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
if ./test_lsp_detection.sh > "$TMP_DIR/test3.log" 2>&1; then
    echo "✅ PASSED: LSP language ID detection working"
else
    echo "❌ FAILED: LSP language ID detection not working"
    cat "$TMP_DIR/test3.log"
    exit 1
fi
echo ""

# Test 4: Build
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 4: Build Verification"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cargo build --lib --quiet 2>&1 | grep -i "^error" > "$TMP_DIR/test4.log" || true
if [ ! -s "$TMP_DIR/test4.log" ]; then
    echo "✅ PASSED: Build successful (no errors)"
else
    echo "❌ FAILED: Build has errors"
    cat "$TMP_DIR/test4.log"
    exit 1
fi
echo ""

# Test 5: Test Files Exist
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 5: Test Files Created"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
if [ -f "TestJava.java" ]; then
    echo "✅ TestJava.java exists"
else
    echo "❌ TestJava.java missing"
    exit 1
fi

if [ -f "java_test_project/src/main/java/com/example/HelloWorld.java" ]; then
    echo "✅ HelloWorld.java exists"
else
    echo "❌ HelloWorld.java missing"
    exit 1
fi

if [ -f "java_test_project/pom.xml" ]; then
    echo "✅ pom.xml exists"
else
    echo "❌ pom.xml missing"
    exit 1
fi
echo ""

# Summary
echo "╔════════════════════════════════════════════════════════════════╗"
echo "║                     ALL TESTS PASSED! ✅                        ║"
echo "╚════════════════════════════════════════════════════════════════╝"
echo ""
echo "Summary:"
echo "  ✅ Language detection: Working"
echo "  ✅ Syntax highlighting: Working"
echo "  ✅ LSP language ID: Working"
echo "  ✅ Build: Successful"
echo "  ✅ Test files: Created"
echo ""
echo "Java support is fully functional!"
echo ""
echo "Next steps:"
echo "  1. Open a Java file: cargo run -- TestJava.java"
echo "  2. Or open project: cargo run -- java_test_project/src/main/java/com/example/HelloWorld.java"
echo "  3. Syntax highlighting will work immediately"
echo "  4. LSP operations will work once jdtls is installed"
echo ""
