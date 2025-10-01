#!/bin/bash

# Automated test runner for ovim REST API
# This script starts ovim with the API, extracts the URL, and runs tests

set -e

GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

TEST_FILE="/tmp/ovim_test_$$.txt"
LOG_FILE="/tmp/ovim_server_$$.log"

echo -e "${BLUE}=== ovim REST API Automated Tests ===${NC}\n"

# Clean up on exit
cleanup() {
    echo -e "\n${YELLOW}Cleaning up...${NC}"
    if [ ! -z "$OVIM_PID" ]; then
        kill $OVIM_PID 2>/dev/null || true
        wait $OVIM_PID 2>/dev/null || true
    fi
    rm -f "$TEST_FILE" "$LOG_FILE"
}
trap cleanup EXIT

# Create test file
echo "Creating test file..."
cat > "$TEST_FILE" << 'EOF'
Welcome to ovim!
This is a test file.
Line 3 here.
EOF

# Start ovim in background and capture output
echo "Starting ovim with REST API..."
cargo run --quiet -- "$TEST_FILE" --expose-rest-api --dimension=80x24 > "$LOG_FILE" 2>&1 &
OVIM_PID=$!

# Wait for server to start and extract URL
echo "Waiting for server to start..."
API_URL=""
for i in {1..50}; do
    if grep -q "API URL:" "$LOG_FILE" 2>/dev/null; then
        API_URL=$(grep "API URL:" "$LOG_FILE" | tail -1 | awk '{print $NF}')
        break
    fi
    sleep 0.1
done

if [ -z "$API_URL" ]; then
    echo -e "${RED}Failed to start API server or extract URL${NC}"
    echo "Log contents:"
    cat "$LOG_FILE"
    exit 1
fi

echo -e "${GREEN}✓${NC} Server started at: $API_URL"
echo ""

# Helper functions
send_keys() {
    curl -s -X POST "$API_URL/keys" \
      -H "Content-Type: application/json" \
      -d "{\"keys\": \"$1\"}"
}

get_cursor() {
    curl -s "$API_URL/cursor"
}

get_mode() {
    curl -s "$API_URL/mode"
}

get_buffer() {
    curl -s "$API_URL/buffer"
}

set_buffer() {
    curl -s -X PUT "$API_URL/buffer" \
      -H "Content-Type: application/json" \
      -d "{\"content\": \"$1\"}"
}

get_snapshot() {
    curl -s "$API_URL/snapshot"
}

test_assert() {
    local name="$1"
    local expected="$2"
    local actual="$3"

    if echo "$actual" | grep -q "$expected"; then
        echo -e "${GREEN}✓${NC} $name"
        return 0
    else
        echo -e "${RED}✗${NC} $name"
        echo "  Expected to contain: $expected"
        echo "  Got: $actual"
        return 1
    fi
}

# Run tests
echo -e "${BLUE}=== Running Tests ===${NC}\n"

PASSED=0
FAILED=0

# Test 1: Get initial buffer
echo "Test 1: Get initial buffer content"
result=$(get_buffer)
if test_assert "  Buffer contains 'Welcome'" "Welcome to ovim" "$result"; then
    ((PASSED++))
else
    ((FAILED++))
fi

# Test 2: Check initial mode
echo "Test 2: Check initial mode"
result=$(get_mode)
if test_assert "  Mode is Normal" "Normal" "$result"; then
    ((PASSED++))
else
    ((FAILED++))
fi

# Test 3: Navigation (gg)
echo "Test 3: Navigate to top with gg"
send_keys "gg" > /dev/null
result=$(get_cursor)
if test_assert "  Cursor at line 0" '"line":0' "$result"; then
    ((PASSED++))
else
    ((FAILED++))
fi

# Test 4: Navigation (G)
echo "Test 4: Navigate to bottom with G"
send_keys "G" > /dev/null
result=$(get_cursor)
if test_assert "  Cursor at line 2" '"line":2' "$result"; then
    ((PASSED++))
else
    ((FAILED++))
fi

# Test 5: Set buffer content
echo "Test 5: Set new buffer content"
set_buffer "Line 1\nLine 2\nLine 3" > /dev/null
result=$(get_buffer)
if test_assert "  Buffer updated" "Line 1" "$result" && test_assert "  Has Line 3" "Line 3" "$result"; then
    ((PASSED++))
else
    ((FAILED++))
fi

# Test 6: Insert mode
echo "Test 6: Enter insert mode"
send_keys "ggi" > /dev/null
result=$(get_mode)
if test_assert "  Mode is Insert" "Insert" "$result"; then
    ((PASSED++))
else
    ((FAILED++))
fi

# Test 7: Type in insert mode
echo "Test 7: Type text in insert mode"
send_keys "INSERTED: " > /dev/null
send_keys "<Esc>" > /dev/null
result=$(get_buffer)
if test_assert "  Text inserted" "INSERTED:" "$result"; then
    ((PASSED++))
else
    ((FAILED++))
fi

# Test 8: Delete line
echo "Test 8: Delete line with dd"
send_keys "ggdd" > /dev/null
result=$(get_buffer)
if test_assert "  First line deleted" "Line 2" "$result"; then
    ((PASSED++))
else
    ((FAILED++))
fi

# Test 9: Undo
echo "Test 9: Undo with u"
send_keys "u" > /dev/null
result=$(get_buffer)
if test_assert "  Delete undone" "INSERTED" "$result"; then
    ((PASSED++))
else
    ((FAILED++))
fi

# Test 10: Visual mode
echo "Test 10: Enter visual mode"
send_keys "ggv" > /dev/null
result=$(get_mode)
if test_assert "  Mode is Visual" "Visual" "$result"; then
    ((PASSED++))
else
    ((FAILED++))
fi

# Test 11: Move in visual mode
echo "Test 11: Move in visual mode"
send_keys "llll<Esc>" > /dev/null
result=$(get_mode)
if test_assert "  Back to Normal mode" "Normal" "$result"; then
    ((PASSED++))
else
    ((FAILED++))
fi

# Test 12: Yank line
echo "Test 12: Yank line with yy"
set_buffer "Copy me\nOriginal" > /dev/null
send_keys "ggyy" > /dev/null
result=$(get_snapshot)
if test_assert "  Line yanked to register" "Copy me" "$result"; then
    ((PASSED++))
else
    ((FAILED++))
fi

# Test 13: Paste
echo "Test 13: Paste with p"
send_keys "jp" > /dev/null
result=$(get_buffer)
# Should have original line, pasted line, then original line again
if test_assert "  Line pasted" "Copy me" "$result"; then
    ((PASSED++))
else
    ((FAILED++))
fi

# Test 14: Search
echo "Test 14: Search with /"
set_buffer "First line\nSecond target\nThird line" > /dev/null
send_keys "gg" > /dev/null
send_keys "/target<CR>" > /dev/null
result=$(get_cursor)
if test_assert "  Cursor on search result" '"line":1' "$result"; then
    ((PASSED++))
else
    ((FAILED++))
fi

# Test 15: Count prefix
echo "Test 15: Count prefix (3j)"
set_buffer "1\n2\n3\n4\n5\n6\n7\n8" > /dev/null
send_keys "gg" > /dev/null
send_keys "3j" > /dev/null
result=$(get_cursor)
if test_assert "  Moved down 3 lines" '"line":3' "$result"; then
    ((PASSED++))
else
    ((FAILED++))
fi

# Summary
echo ""
echo -e "${BLUE}=== Test Summary ===${NC}"
echo -e "Passed: ${GREEN}$PASSED${NC}"
echo -e "Failed: ${RED}$FAILED${NC}"
echo -e "Total: $((PASSED + FAILED))"

if [ $FAILED -eq 0 ]; then
    echo -e "\n${GREEN}All tests passed!${NC}"
    exit 0
else
    echo -e "\n${RED}Some tests failed${NC}"
    exit 1
fi
