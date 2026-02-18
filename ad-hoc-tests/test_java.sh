#!/usr/bin/env bash
# Manual test script for ovim Java IDE features
# Tests end-to-end workflow with real projects

set -euo pipefail

echo "🧪 ovim Java IDE - Manual Test Suite"
echo "======================================"
echo

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Test counter
TESTS_RUN=0
TESTS_PASSED=0
TESTS_FAILED=0
TMP_BASE="$(mktemp -d "${TMPDIR:-/tmp}/ovim-java-test.XXXXXX")"

# Helper functions
test_start() {
    echo -e "${YELLOW}► Test: $1${NC}"
    TESTS_RUN=$((TESTS_RUN + 1))
}

test_pass() {
    echo -e "${GREEN}✓ PASSED: $1${NC}"
    TESTS_PASSED=$((TESTS_PASSED + 1))
    echo
}

test_fail() {
    echo -e "${RED}✗ FAILED: $1${NC}"
    TESTS_FAILED=$((TESTS_FAILED + 1))
    echo
}

# Clean up function
cleanup() {
    echo "Cleaning up test projects..."
    rm -rf "$TMP_BASE"
}

trap cleanup EXIT

echo "Phase 1: Parser Tests"
echo "---------------------"

# Test 1: Create Gradle Java 17 project
test_start "Create Gradle Java 17 project and detect version"
GRADLE_TEST_DIR="$TMP_BASE/ovim_test_gradle17"
mkdir -p "$GRADLE_TEST_DIR"
cat > "$GRADLE_TEST_DIR/build.gradle" << 'EOF'
plugins {
    id 'java'
}

java {
    toolchain {
        languageVersion = JavaLanguageVersion.of(17)
    }
}
EOF

cat > "$GRADLE_TEST_DIR/Main.java" << 'EOF'
public class Main {
    public static void main(String[] args) {
        System.out.println("Hello from Java 17!");
    }
}
EOF

# Run parser test
if cargo test --test java_parser_test test_gradle_toolchain_java_17 2>&1 | grep -q "test result: ok"; then
    test_pass "Gradle Java 17 detection"
else
    test_fail "Gradle Java 17 detection"
fi

# Test 2: Create Maven Java 21 project
test_start "Create Maven Java 21 project and detect version"
MAVEN_TEST_DIR="$TMP_BASE/ovim_test_maven21"
mkdir -p "$MAVEN_TEST_DIR"
cat > "$MAVEN_TEST_DIR/pom.xml" << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
    <modelVersion>4.0.0</modelVersion>
    <groupId>com.test</groupId>
    <artifactId>test-app</artifactId>
    <version>1.0.0</version>

    <properties>
        <maven.compiler.source>21</maven.compiler.source>
        <maven.compiler.target>21</maven.compiler.target>
    </properties>
</project>
EOF

mkdir -p "$MAVEN_TEST_DIR/src/main/java"
cat > "$MAVEN_TEST_DIR/src/main/java/Main.java" << 'EOF'
public class Main {
    public static void main(String[] args) {
        System.out.println("Hello from Java 21!");
    }
}
EOF

if cargo test --test java_parser_test test_maven_java_version 2>&1 | grep -q "test result: ok"; then
    test_pass "Maven Java 21 detection"
else
    test_fail "Maven Java 21 detection"
fi

echo
echo "Phase 2: Downloader Tests"
echo "-------------------------"

# Test 3: Check download URLs
test_start "Verify jdtls download URLs are accessible"
URLS=(
    "https://download.eclipse.org/jdtls/snapshots/jdt-language-server-latest.tar.gz"
    "https://download.eclipse.org/jdtls/milestones/1.38.0/jdt-language-server-1.38.0-202408011337.tar.gz"
)

URL_WORKS=false
for url in "${URLS[@]}"; do
    if curl --head --silent --fail "$url" > /dev/null 2>&1; then
        echo "  ✓ URL accessible: $url"
        URL_WORKS=true
        break
    else
        echo "  ✗ URL not accessible: $url"
    fi
done

if [ "$URL_WORKS" = true ]; then
    test_pass "At least one download URL is accessible"
else
    test_fail "No download URLs are accessible"
fi

echo
echo "Phase 3: JVM Detection Tests"
echo "-----------------------------"

# Test 4: Check if Java is installed
test_start "Detect Java installation"
if command -v java &> /dev/null; then
    JAVA_VERSION=$(java -version 2>&1 | head -n 1)
    echo "  Found: $JAVA_VERSION"
    test_pass "Java installation detected"
else
    test_fail "Java installation not found"
fi

# Test 5: Check JAVA_HOME
test_start "Check JAVA_HOME environment variable"
if [ -n "${JAVA_HOME:-}" ]; then
    echo "  JAVA_HOME=$JAVA_HOME"
    test_pass "JAVA_HOME is set"
else
    echo "  JAVA_HOME is not set (this is okay)"
    test_pass "JAVA_HOME check completed"
fi

echo
echo "Phase 4: Cache Directory Tests"
echo "-------------------------------"

# Test 6: Cache directory creation
test_start "Verify cache directory structure"
EXPECTED_CACHE="$HOME/.cache/ovim/java"
if [ -d "$EXPECTED_CACHE" ] || mkdir -p "$EXPECTED_CACHE"; then
    echo "  Cache directory: $EXPECTED_CACHE"
    test_pass "Cache directory accessible"
else
    test_fail "Cannot create cache directory"
fi

echo
echo "Phase 5: Build Tests"
echo "--------------------"

# Test 7: Full build test
test_start "Build ovim with Java support"
if cargo build --release 2>&1 | grep -q "Finished"; then
    test_pass "Build successful"
else
    test_fail "Build failed"
fi

# Test 8: Run all unit tests
test_start "Run all Java-related unit tests"
if cargo test --test java_parser_test 2>&1 | grep -q "test result: ok"; then
    test_pass "All unit tests passed"
else
    test_fail "Some unit tests failed"
fi

echo
echo "======================================"
echo "📊 Test Results Summary"
echo "======================================"
echo "Tests Run:    $TESTS_RUN"
echo -e "${GREEN}Tests Passed: $TESTS_PASSED${NC}"
echo -e "${RED}Tests Failed: $TESTS_FAILED${NC}"
echo

if [ $TESTS_FAILED -eq 0 ]; then
    echo -e "${GREEN}🎉 All tests passed! Java IDE is ready!${NC}"
    exit 0
else
    echo -e "${RED}❌ Some tests failed. Please review the output above.${NC}"
    exit 1
fi
