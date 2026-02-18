#!/usr/bin/env bash

set -euo pipefail

# Comprehensive testing script for ovim
# Tests various Neovim features and identifies missing functionality

API_URL="http://127.0.0.1:35339"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test counter
TESTS_RUN=0
TESTS_PASSED=0
TESTS_FAILED=0

# Helper functions
send_keys() {
    local keys="$1"
    curl -s -X POST "$API_URL/v1/keys" \
        -H "Content-Type: application/json" \
        -d "{\"keys\": \"$keys\"}" > /dev/null
}

get_cursor() {
    curl -s "$API_URL/v1/cursor"
}

get_buffer() {
    curl -s "$API_URL/v1/buffer" | jq -r '.content'
}

get_mode() {
    curl -s "$API_URL/v1/mode" | jq -r '.mode'
}

reset_buffer() {
    local content="$1"
    curl -s -X PUT "$API_URL/v1/buffer" \
        -H "Content-Type: application/json" \
        -d "{\"content\": \"$content\"}" > /dev/null
    send_keys "gg0"  # Go to start
}

assert_cursor() {
    local expected_line="$1"
    local expected_col="$2"
    local actual=$(get_cursor)
    local actual_line=$(echo "$actual" | jq '.line')
    local actual_col=$(echo "$actual" | jq '.column')

    TESTS_RUN=$((TESTS_RUN + 1))

    if [ "$actual_line" == "$expected_line" ] && [ "$actual_col" == "$expected_col" ]; then
        echo -e "${GREEN}✓${NC} Cursor at ($expected_line, $expected_col)"
        TESTS_PASSED=$((TESTS_PASSED + 1))
        return 0
    else
        echo -e "${RED}✗${NC} Expected cursor at ($expected_line, $expected_col), got ($actual_line, $actual_col)"
        TESTS_FAILED=$((TESTS_FAILED + 1))
        return 1
    fi
}

assert_buffer_contains() {
    local expected="$1"
    local actual=$(get_buffer)

    TESTS_RUN=$((TESTS_RUN + 1))

    if echo "$actual" | grep -qF "$expected"; then
        echo -e "${GREEN}✓${NC} Buffer contains: $expected"
        TESTS_PASSED=$((TESTS_PASSED + 1))
        return 0
    else
        echo -e "${RED}✗${NC} Buffer should contain: $expected"
        echo -e "  Actual buffer: $actual"
        TESTS_FAILED=$((TESTS_FAILED + 1))
        return 1
    fi
}

assert_mode() {
    local expected="$1"
    local actual=$(get_mode)

    TESTS_RUN=$((TESTS_RUN + 1))

    if [ "$actual" == "$expected" ]; then
        echo -e "${GREEN}✓${NC} Mode is $expected"
        TESTS_PASSED=$((TESTS_PASSED + 1))
        return 0
    else
        echo -e "${RED}✗${NC} Expected mode $expected, got $actual"
        TESTS_FAILED=$((TESTS_FAILED + 1))
        return 1
    fi
}

print_section() {
    echo ""
    echo "============================================"
    echo "$1"
    echo "============================================"
}

# ============================================
# Test Suite
# ============================================

print_section "BASIC MOTIONS: h, j, k, l"

reset_buffer "line1\nline2\nline3\nline4"

echo "Test: j (down)"
send_keys "j"
assert_cursor 1 0

echo "Test: 2j (down 2 lines)"
send_keys "2j"
assert_cursor 3 0

echo "Test: k (up)"
send_keys "k"
assert_cursor 2 0

echo "Test: 2k (up 2 lines)"
send_keys "2k"
assert_cursor 0 0

echo "Test: l (right)"
send_keys "l"
assert_cursor 0 1

echo "Test: 3l (right 3)"
send_keys "3l"
assert_cursor 0 4

echo "Test: h (left)"
send_keys "h"
assert_cursor 0 3

echo "Test: 2h (left 2)"
send_keys "2h"
assert_cursor 0 1

print_section "WORD MOTIONS: w, b, e, W, B, E"

reset_buffer "the quick brown fox jumps"

echo "Test: w (next word)"
send_keys "w"
assert_cursor 0 4

echo "Test: 2w (next 2 words)"
send_keys "2w"
assert_cursor 0 16

echo "Test: b (back word)"
send_keys "b"
assert_cursor 0 10

echo "Test: e (end of word)"
reset_buffer "the quick brown"
send_keys "e"
assert_cursor 0 2

echo "Test: ge (back to end of previous word)"
send_keys "w"  # Move to 'quick'
send_keys "ge"
assert_cursor 0 2

print_section "LINE MOTIONS: 0, $, ^"

reset_buffer "  hello world  "

echo "Test: $ (end of line)"
send_keys "$"
assert_cursor 0 13  # Last char before newline

echo "Test: 0 (start of line)"
send_keys "0"
assert_cursor 0 0

echo "Test: ^ (first non-blank)"
send_keys "^"
assert_cursor 0 2

print_section "FILE MOTIONS: gg, G"

reset_buffer "line1\nline2\nline3\nline4\nline5"

echo "Test: G (end of file)"
send_keys "G"
assert_cursor 4 0

echo "Test: gg (start of file)"
send_keys "gg"
assert_cursor 0 0

echo "Test: 3G (goto line 3)"
send_keys "3G"
assert_cursor 2 0

print_section "INSERT MODE"

reset_buffer "hello"

echo "Test: i (insert before cursor)"
send_keys "i"
assert_mode "INSERT"
send_keys "x"
assert_buffer_contains "xhello"
send_keys "<Esc>"
assert_mode "NORMAL"

reset_buffer "hello"
echo "Test: a (append after cursor)"
send_keys "a"
assert_mode "INSERT"
send_keys "x"
assert_buffer_contains "hxello"
send_keys "<Esc>"

reset_buffer "hello"
echo "Test: I (insert at start of line)"
send_keys "I"
send_keys "x"
assert_buffer_contains "xhello"
send_keys "<Esc>"

reset_buffer "hello"
echo "Test: A (append at end of line)"
send_keys "A"
send_keys "x"
assert_buffer_contains "hellox"
send_keys "<Esc>"

reset_buffer "hello\nworld"
echo "Test: o (open line below)"
send_keys "o"
assert_mode "INSERT"
send_keys "new"
assert_buffer_contains "hello\nnew\nworld"
send_keys "<Esc>"

reset_buffer "hello\nworld"
echo "Test: O (open line above)"
send_keys "O"
send_keys "new"
assert_buffer_contains "new\nhello\nworld"
send_keys "<Esc>"

print_section "DELETE OPERATIONS"

reset_buffer "hello world"
echo "Test: x (delete char)"
send_keys "x"
assert_buffer_contains "ello world"

reset_buffer "hello world"
echo "Test: 3x (delete 3 chars)"
send_keys "3x"
assert_buffer_contains "lo world"

reset_buffer "hello world"
echo "Test: dd (delete line)"
send_keys "dd"
assert_buffer_contains ""

reset_buffer "line1\nline2\nline3"
echo "Test: dd on first line"
send_keys "dd"
assert_buffer_contains "line2\nline3"
assert_cursor 0 0

reset_buffer "hello world"
echo "Test: dw (delete word)"
send_keys "dw"
assert_buffer_contains "world"

reset_buffer "hello world"
echo "Test: d$ (delete to end of line)"
send_keys "d$"
assert_buffer_contains ""

reset_buffer "hello world"
echo "Test: d0 (delete to start of line)"
send_keys "$"  # Go to end
send_keys "d0"
assert_buffer_contains "d"  # Only last char remains

print_section "CHANGE OPERATIONS"

reset_buffer "hello world"
echo "Test: cw (change word)"
send_keys "cw"
assert_mode "INSERT"
send_keys "goodbye"
send_keys "<Esc>"
assert_buffer_contains "goodbye world"

reset_buffer "hello world"
echo "Test: cc (change line)"
send_keys "cc"
assert_mode "INSERT"
send_keys "new line"
send_keys "<Esc>"
assert_buffer_contains "new line"

print_section "YANK AND PASTE"

reset_buffer "hello world"
echo "Test: yw and p (yank word and paste)"
send_keys "yw"  # Yank word
send_keys "p"   # Paste after cursor
assert_buffer_contains "hhelloello world"

reset_buffer "line1\nline2\nline3"
echo "Test: yy and p (yank line and paste)"
send_keys "yy"  # Yank line
send_keys "p"   # Paste below
assert_buffer_contains "line1\nline1\nline2"

print_section "VISUAL MODE"

reset_buffer "hello world"
echo "Test: v (visual mode)"
send_keys "v"
assert_mode "VISUAL"
send_keys "<Esc>"
assert_mode "NORMAL"

reset_buffer "hello world"
echo "Test: visual selection and delete"
send_keys "v"
send_keys "4l"  # Select 5 chars
send_keys "d"
assert_mode "NORMAL"
assert_buffer_contains " world"

reset_buffer "line1\nline2\nline3"
echo "Test: V (visual line mode)"
send_keys "V"
assert_mode "VISUAL_LINE"
send_keys "d"
assert_buffer_contains "line2\nline3"

print_section "SEARCH"

reset_buffer "hello world hello again"
echo "Test: / (search forward)"
send_keys "/world<CR>"
assert_cursor 0 6

reset_buffer "hello world hello again"
echo "Test: n (next search match)"
send_keys "/hello<CR>"
assert_cursor 0 0
send_keys "n"
assert_cursor 0 12

reset_buffer "hello world hello again"
echo "Test: ? (search backward)"
send_keys "$"  # Go to end
send_keys "?hello<CR>"
assert_cursor 0 12

print_section "TEXT OBJECTS"

reset_buffer "hello world test"
echo "Test: diw (delete inner word)"
send_keys "w"  # Move to 'world'
send_keys "diw"
assert_buffer_contains "hello  test"

reset_buffer "hello world test"
echo "Test: daw (delete around word)"
send_keys "w"  # Move to 'world'
send_keys "daw"
assert_buffer_contains "hello test"

reset_buffer "\"hello world\""
echo "Test: di\" (delete inside quotes)"
send_keys "di\""
assert_buffer_contains "\"\""

reset_buffer "(hello world)"
echo "Test: di( (delete inside parens)"
send_keys "di("
assert_buffer_contains "()"

reset_buffer "{hello world}"
echo "Test: di{ (delete inside braces)"
send_keys "di{"
assert_buffer_contains "{}"

reset_buffer "[hello world]"
echo "Test: di[ (delete inside brackets)"
send_keys "di["
assert_buffer_contains "[]"

print_section "UNDO/REDO"

reset_buffer "hello"
echo "Test: u (undo)"
send_keys "Aworld<Esc>"
assert_buffer_contains "helloworld"
send_keys "u"
assert_buffer_contains "hello"

reset_buffer "hello"
echo "Test: Ctrl-r (redo)"
send_keys "Aworld<Esc>"
send_keys "u"
assert_buffer_contains "hello"
send_keys "<C-r>"
assert_buffer_contains "helloworld"

print_section "MACROS"

reset_buffer "test\ntest\ntest"
echo "Test: Record and replay macro"
send_keys "qa"    # Start recording to register 'a'
send_keys "Iline <Esc>j"  # Insert "line " at start and move down
send_keys "q"     # Stop recording
send_keys "gg"    # Go to start
send_keys "3@a"   # Replay 3 times
assert_buffer_contains "line "

print_section "FIND CHARACTER"

reset_buffer "hello world"
echo "Test: f (find character)"
send_keys "fw"
assert_cursor 0 6

reset_buffer "hello world"
echo "Test: F (find character backward)"
send_keys "$"
send_keys "Fe"
assert_cursor 0 1

reset_buffer "hello world"
echo "Test: t (till character)"
send_keys "tw"
assert_cursor 0 5

reset_buffer "hello world"
echo "Test: T (till character backward)"
send_keys "$"
send_keys "Te"
assert_cursor 0 2

reset_buffer "hello world hello"
echo "Test: ; (repeat find)"
send_keys "fh"
assert_cursor 0 0
send_keys ";"
assert_cursor 0 12

reset_buffer "hello world hello"
echo "Test: , (repeat find backward)"
send_keys "$"
send_keys "Fh"
assert_cursor 0 12
send_keys ","
assert_cursor 0 0

print_section "REPLACE"

reset_buffer "hello"
echo "Test: r (replace character)"
send_keys "rx"
assert_buffer_contains "xello"

reset_buffer "hello"
echo "Test: 3r (replace 3 characters)"
send_keys "3rx"
assert_buffer_contains "xxxlo"

print_section "MARKS"

reset_buffer "line1\nline2\nline3"
echo "Test: m (set mark) and ' (goto mark)"
send_keys "ma"   # Set mark 'a'
send_keys "G"    # Go to end
assert_cursor 2 0
send_keys "'a"   # Jump to mark 'a'
assert_cursor 0 0

print_section "PARAGRAPH MOTIONS"

reset_buffer "para1\nline2\n\npara2\nline4"
echo "Test: } (next paragraph)"
send_keys "}"
assert_cursor 3 0

reset_buffer "para1\nline2\n\npara2\nline4"
echo "Test: { (previous paragraph)"
send_keys "G"
send_keys "{"
assert_cursor 2 0

print_section "ADVANCED TEXT OBJECTS"

reset_buffer "first sentence. second sentence."
echo "Test: dis (delete inner sentence)"
send_keys "dis"
assert_buffer_contains " second sentence."

reset_buffer "para1\n\npara2"
echo "Test: dip (delete inner paragraph)"
send_keys "dip"
assert_buffer_contains "\npara2"

reset_buffer "para1\n\npara2"
echo "Test: dap (delete around paragraph)"
send_keys "dap"
assert_buffer_contains "para2"

print_section "JOIN LINES"

reset_buffer "line1\nline2\nline3"
echo "Test: J (join lines)"
send_keys "J"
assert_buffer_contains "line1 line2"

reset_buffer "line1\nline2\nline3\nline4"
echo "Test: 3J (join 3 lines)"
send_keys "3J"
assert_buffer_contains "line1 line2 line3"

print_section "REPEAT COMMAND"

reset_buffer "hello world test"
echo "Test: . (repeat last change)"
send_keys "dw"  # Delete word
assert_buffer_contains "world test"
send_keys "."   # Repeat
assert_buffer_contains "test"

print_section "CASE OPERATIONS"

reset_buffer "Hello World"
echo "Test: ~ (toggle case)"
send_keys "~"
assert_buffer_contains "hello World"

reset_buffer "hello"
echo "Test: gU (uppercase)"
send_keys "gUiw"
assert_buffer_contains "HELLO"

reset_buffer "HELLO"
echo "Test: gu (lowercase)"
send_keys "guiw"
assert_buffer_contains "hello"

print_section "INCREMENT/DECREMENT"

reset_buffer "count: 5"
echo "Test: Ctrl-a (increment number)"
send_keys "f5"
send_keys "<C-a>"
assert_buffer_contains "count: 6"

reset_buffer "count: 5"
echo "Test: Ctrl-x (decrement number)"
send_keys "f5"
send_keys "<C-x>"
assert_buffer_contains "count: 4"

print_section "INDENTATION"

reset_buffer "line1\nline2"
echo "Test: >> (indent)"
send_keys ">>"
# Check if line is indented

reset_buffer "    line1\n    line2"
echo "Test: << (dedent)"
send_keys "<<"
# Check if line is dedented

# ============================================
# Summary
# ============================================

print_section "TEST SUMMARY"
echo "Tests run: $TESTS_RUN"
echo -e "${GREEN}Tests passed: $TESTS_PASSED${NC}"
echo -e "${RED}Tests failed: $TESTS_FAILED${NC}"

if [ $TESTS_FAILED -eq 0 ]; then
    echo -e "\n${GREEN}All tests passed!${NC}"
    exit 0
else
    echo -e "\n${RED}Some tests failed.${NC}"
    exit 1
fi
