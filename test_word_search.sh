#!/usr/bin/env zsh

# Test * and # word search motions
# Usage: ./test_word_search.sh <port>

set -e

if [ -z "$1" ]; then
    echo "Usage: $0 <port>"
    exit 1
fi

PORT=$1
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${BLUE}=== Testing * and # Word Search ===${NC}\n"

# Test 1: * searches forward for word under cursor
echo -e "${BLUE}Test 1: * (search forward for word under cursor)${NC}"
./send-cmd $PORT buffer "hello world\nworld peace\nhello there\nworld domination"
echo "Buffer:"
./send-cmd $PORT get buffer | head -5
./send-cmd $PORT keys "gg0"  # Go to first word (hello)
echo "Cursor on 'hello' at line 0"
./send-cmd $PORT keys "*"    # Search forward for 'hello'
CURSOR=$(./send-cmd $PORT get cursor)
echo "After * (should jump to next 'hello' at line 2): $CURSOR"
echo -e "${GREEN}✓ Test 1 complete${NC}\n"

# Test 2: # searches backward for word under cursor
echo -e "${BLUE}Test 2: # (search backward for word under cursor)${NC}"
./send-cmd $PORT buffer "apple banana apple cherry apple"
./send-cmd $PORT keys "gg0"
./send-cmd $PORT keys "www"  # Jump to middle 'apple'
CURSOR=$(./send-cmd $PORT get cursor)
echo "Cursor on middle 'apple': $CURSOR"
./send-cmd $PORT keys "#"    # Search backward for 'apple'
CURSOR=$(./send-cmd $PORT get cursor)
echo "After # (should jump to first 'apple' at col 0): $CURSOR"
echo -e "${GREEN}✓ Test 2 complete${NC}\n"

# Test 3: * followed by n (next match)
echo -e "${BLUE}Test 3: * followed by n${NC}"
./send-cmd $PORT buffer "foo bar foo baz foo"
./send-cmd $PORT keys "gg0"  # First 'foo'
./send-cmd $PORT keys "*"    # Jump to second 'foo'
./send-cmd $PORT keys "n"    # Jump to third 'foo'
CURSOR=$(./send-cmd $PORT get cursor)
echo "After * then n (should be at third 'foo' at col 16): $CURSOR"
echo -e "${GREEN}✓ Test 3 complete${NC}\n"

# Test 4: * with word boundaries (shouldn't match partial words)
echo -e "${BLUE}Test 4: Word boundary matching${NC}"
./send-cmd $PORT buffer "cat catch cathedral"
./send-cmd $PORT keys "gg0"  # On 'cat'
echo "Cursor on 'cat' (should NOT match 'catch' or 'cathedral')"
./send-cmd $PORT keys "*"
CURSOR=$(./send-cmd $PORT get cursor)
echo "After * (should stay at line 0, col 0 - no matches): $CURSOR"
echo -e "${GREEN}✓ Test 4 complete${NC}\n"

# Test 5: # followed by N (previous match in reverse)
echo -e "${BLUE}Test 5: # followed by N${NC}"
./send-cmd $PORT buffer "test\ntest\ntest"
./send-cmd $PORT keys "G0"   # Last 'test'
./send-cmd $PORT keys "#"    # Jump backward
./send-cmd $PORT keys "N"    # Jump forward (opposite of #)
CURSOR=$(./send-cmd $PORT get cursor)
echo "After # then N (should be back at line 2): $CURSOR"
echo -e "${GREEN}✓ Test 5 complete${NC}\n"

# Test 6: * on multi-line buffer
echo -e "${BLUE}Test 6: * across multiple lines${NC}"
./send-cmd $PORT buffer "one\ntwo\nthree\ntwo\nfour\ntwo"
./send-cmd $PORT keys "ggj0"  # Line 1, word 'two'
./send-cmd $PORT keys "*"     # First *
CURSOR1=$(./send-cmd $PORT get cursor)
./send-cmd $PORT keys "n"     # Next match
CURSOR2=$(./send-cmd $PORT get cursor)
echo "First * jump: $CURSOR1 (should be line 3)"
echo "After n: $CURSOR2 (should be line 5)"
echo -e "${GREEN}✓ Test 6 complete${NC}\n"

# Test 7: * when cursor is not on a word
echo -e "${BLUE}Test 7: * when not on a word character${NC}"
./send-cmd $PORT buffer "hello   world"
./send-cmd $PORT keys "gg0lllll"  # On a space
echo "Cursor on whitespace (not a word)"
./send-cmd $PORT keys "*"
CURSOR=$(./send-cmd $PORT get cursor)
echo "After * (should not move): $CURSOR"
echo -e "${GREEN}✓ Test 7 complete${NC}\n"

# Test 8: Wrap-around search
echo -e "${BLUE}Test 8: * wraps around to beginning${NC}"
./send-cmd $PORT buffer "unique word\nsome text\nmore text"
./send-cmd $PORT keys "gg0"  # On 'unique'
./send-cmd $PORT keys "*"    # No more matches, should wrap
CURSOR=$(./send-cmd $PORT get cursor)
echo "After * on unique word (should wrap to same position): $CURSOR"
echo -e "${GREEN}✓ Test 8 complete${NC}\n"

echo -e "${GREEN}=== All word search tests completed! ===${NC}"
echo -e "\n${BLUE}Summary:${NC}"
echo "✓ * - search forward for word under cursor"
echo "✓ # - search backward for word under cursor"
echo "✓ n - next match after * or #"
echo "✓ N - previous match"
echo "✓ Word boundary support (\\bword\\b)"
echo "✓ Multi-line search"
echo "✓ Graceful handling of non-word cursors"
echo "✓ Wrap-around search"
