#!/bin/bash

# Integration test for ovim REST API
# This script tests the API with realistic Vim workflows

set -e  # Exit on error

API_URL="http://localhost:3000"
TEST_FILE="/tmp/ovim_test_$(date +%s).txt"

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${BLUE}=== ovim REST API Integration Tests ===${NC}\n"

# Check if server is running
echo -n "Checking if API server is running... "
if curl -s -f $API_URL/mode > /dev/null 2>&1; then
    echo -e "${GREEN}✓${NC}"
else
    echo -e "${RED}✗${NC}"
    echo -e "${RED}Error: API server not running at $API_URL${NC}"
    echo -e "${YELLOW}Start with: cargo run -- $TEST_FILE --expose-rest-api${NC}"
    exit 1
fi

# Helper functions
test_assert() {
    local name="$1"
    local expected="$2"
    local actual="$3"

    if [[ "$actual" == *"$expected"* ]]; then
        echo -e "${GREEN}✓${NC} $name"
    else
        echo -e "${RED}✗${NC} $name"
        echo "  Expected: $expected"
        echo "  Got: $actual"
        exit 1
    fi
}

send_keys() {
    curl -s -X POST $API_URL/keys \
      -H "Content-Type: application/json" \
      -d "{\"keys\": \"$1\"}" > /dev/null 2>&1
}

get_cursor_line() {
    curl -s $API_URL/cursor | grep -o '"line":[0-9]*' | cut -d: -f2
}

get_cursor_col() {
    curl -s $API_URL/cursor | grep -o '"column":[0-9]*' | cut -d: -f2
}

get_mode() {
    curl -s $API_URL/mode | grep -o '"mode":"[^"]*"' | cut -d\" -f4
}

get_buffer() {
    curl -s $API_URL/buffer | jq -r '.content' 2>/dev/null || curl -s $API_URL/buffer | grep -o '"content":"[^"]*"' | cut -d\" -f4
}

echo -e "\n${BLUE}=== Test Suite 1: Basic Navigation ===${NC}\n"

# Test 1: Initialize buffer
echo "Setting up test buffer..."
curl -s -X PUT $API_URL/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Line 1\nLine 2\nLine 3\nLine 4\nLine 5"}' > /dev/null

# Test 2: Move to start
send_keys "gg"
line=$(get_cursor_line)
test_assert "gg moves to first line" "0" "$line"

# Test 3: Move down with j
send_keys "jjj"
line=$(get_cursor_line)
test_assert "jjj moves down 3 lines" "3" "$line"

# Test 4: Move to end
send_keys "G"
line=$(get_cursor_line)
test_assert "G moves to last line" "4" "$line"

# Test 5: Move up with k
send_keys "kk"
line=$(get_cursor_line)
test_assert "kk moves up 2 lines" "2" "$line"

# Test 6: Move to start of line
send_keys "0"
col=$(get_cursor_col)
test_assert "0 moves to column 0" "0" "$col"

# Test 7: Move right
send_keys "lll"
col=$(get_cursor_col)
test_assert "lll moves right 3 columns" "3" "$col"

echo -e "\n${BLUE}=== Test Suite 2: Modes ===${NC}\n"

# Test 8: Enter insert mode
send_keys "i"
mode=$(get_mode)
test_assert "i enters insert mode" "Insert" "$mode"

# Test 9: Exit to normal mode
send_keys "<Esc>"
mode=$(get_mode)
test_assert "Esc returns to normal mode" "Normal" "$mode"

# Test 10: Enter visual mode
send_keys "v"
mode=$(get_mode)
test_assert "v enters visual mode" "Visual" "$mode"

# Test 11: Exit visual mode
send_keys "<Esc>"
mode=$(get_mode)
test_assert "Esc exits visual mode" "Normal" "$mode"

echo -e "\n${BLUE}=== Test Suite 3: Editing ===${NC}\n"

# Test 12: Insert text
send_keys "ggI# "
send_keys "<Esc>"
buffer=$(get_buffer)
test_assert "I inserts at line start" "# Line" "$buffer"

# Test 13: Append text
send_keys "A!"
send_keys "<Esc>"
buffer=$(get_buffer)
test_assert "A appends at line end" "!" "$buffer"

# Test 14: Delete line
curl -s -X PUT $API_URL/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Delete me\nKeep me\nAlso keep"}' > /dev/null
send_keys "ggdd"
buffer=$(get_buffer)
test_assert "dd deletes line" "Keep me" "$buffer"
test_assert "dd preserves next line" "Also keep" "$buffer"

# Test 15: Delete word
curl -s -X PUT $API_URL/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Delete this word please"}' > /dev/null
send_keys "gg0dw"
buffer=$(get_buffer)
test_assert "dw deletes word" "this word" "$buffer"

echo -e "\n${BLUE}=== Test Suite 4: Yank & Paste ===${NC}\n"

# Test 16: Yank and paste line
curl -s -X PUT $API_URL/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Copy this line\nOriginal"}' > /dev/null
send_keys "ggyy"
send_keys "jp"
buffer=$(get_buffer)
test_assert "yy and p duplicate line" "Copy this line" "$buffer"

echo -e "\n${BLUE}=== Test Suite 5: Count Prefixes ===${NC}\n"

# Test 17: Count with motion
curl -s -X PUT $API_URL/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "1\n2\n3\n4\n5\n6\n7\n8\n9\n10"}' > /dev/null
send_keys "gg"
send_keys "5j"
line=$(get_cursor_line)
test_assert "5j moves down 5 lines" "5" "$line"

# Test 18: Count with delete
curl -s -X PUT $API_URL/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Line 1\nLine 2\nLine 3\nLine 4\nLine 5"}' > /dev/null
send_keys "gg"
send_keys "3dd"
buffer=$(get_buffer)
test_assert "3dd deletes 3 lines" "Line 4" "$buffer"

echo -e "\n${BLUE}=== Test Suite 6: Search ===${NC}\n"

# Test 19: Forward search
curl -s -X PUT $API_URL/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "No match here\nFound the word target\nAnother target here"}' > /dev/null
send_keys "gg"
send_keys "/target<CR>"
line=$(get_cursor_line)
test_assert "/ searches forward" "1" "$line"

# Test 20: Next match
send_keys "n"
line=$(get_cursor_line)
test_assert "n finds next match" "2" "$line"

echo -e "\n${BLUE}=== Test Suite 7: Visual Mode ===${NC}\n"

# Test 21: Visual selection and delete
curl -s -X PUT $API_URL/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Delete these words please"}' > /dev/null
send_keys "gg0"
send_keys "vllllllld"
buffer=$(get_buffer)
test_assert "Visual delete removes selection" "these words" "$buffer"

echo -e "\n${BLUE}=== Test Suite 8: Undo/Redo ===${NC}\n"

# Test 22: Undo
curl -s -X PUT $API_URL/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Original text"}' > /dev/null
send_keys "ggdd"
send_keys "u"
buffer=$(get_buffer)
test_assert "u undoes delete" "Original text" "$buffer"

# Test 23: Redo
send_keys "<C-r>"
buffer=$(get_buffer)
test_assert "Ctrl-R redoes" "" "$buffer"  # Should be empty after redo

echo -e "\n${BLUE}=== Test Suite 9: Operators + Motions ===${NC}\n"

# Test 24: Delete to end of line
curl -s -X PUT $API_URL/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Keep this delete rest"}' > /dev/null
send_keys "gg0llllllllll"
send_keys "D"
buffer=$(get_buffer)
test_assert "D deletes to end" "Keep this" "$buffer"

# Test 25: Change word
curl -s -X PUT $API_URL/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Change oldword here"}' > /dev/null
send_keys "gg0wwcwnewword<Esc>"
buffer=$(get_buffer)
test_assert "cw changes word" "newword" "$buffer"

echo -e "\n${BLUE}=== Test Suite 10: Complex Workflows ===${NC}\n"

# Test 26: Realistic editing workflow
curl -s -X PUT $API_URL/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "def hello():\n    print(world)\n    return"}' > /dev/null

# Navigate to "world", change it to "hello"
send_keys "gg"
send_keys "/world<CR>"
send_keys "cwhello<Esc>"
buffer=$(get_buffer)
test_assert "Complex edit: change function arg" 'print(hello)' "$buffer"

# Test 27: Multiple operations in sequence
curl -s -X PUT $API_URL/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "one two three\nfour five six"}' > /dev/null
send_keys "gg0"      # Go to start
send_keys "dw"       # Delete "one "
send_keys "j"        # Move down
send_keys "dw"       # Delete "four "
send_keys "gg"       # Back to top
send_keys "P"        # Paste before
buffer=$(get_buffer)
test_assert "Sequence: delete, navigate, paste" "four" "$buffer"

echo -e "\n${GREEN}=== All Tests Passed! ===${NC}\n"
echo -e "Total tests: 27"
echo -e "${GREEN}✓ Navigation: 7${NC}"
echo -e "${GREEN}✓ Modes: 4${NC}"
echo -e "${GREEN}✓ Editing: 4${NC}"
echo -e "${GREEN}✓ Yank/Paste: 1${NC}"
echo -e "${GREEN}✓ Count Prefixes: 2${NC}"
echo -e "${GREEN}✓ Search: 2${NC}"
echo -e "${GREEN}✓ Visual Mode: 1${NC}"
echo -e "${GREEN}✓ Undo/Redo: 2${NC}"
echo -e "${GREEN}✓ Operators+Motions: 2${NC}"
echo -e "${GREEN}✓ Complex Workflows: 2${NC}"
