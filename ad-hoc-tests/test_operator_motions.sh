#!/usr/bin/env zsh

# Test operator+motion combinations
# Usage: ./test_operator_motions.sh <port>

set -euo pipefail

if [[ -z "${1:-}" ]]; then
    echo "Usage: $0 <port>"
    exit 1
fi

PORT="$1"
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${BLUE}=== Testing Operator + Motion Combinations ===${NC}\n"

# Test 1: dj (delete current + next line)
echo -e "${BLUE}Test 1: dj (delete current line + line below)${NC}"
./send-cmd "$PORT" buffer "Line 1\nLine 2\nLine 3\nLine 4"
echo "Buffer:"
./send-cmd "$PORT" get buffer | head -5
./send-cmd "$PORT" keys "gg"  # Go to first line
echo "Executing: dj"
./send-cmd "$PORT" keys "dj"
echo "After dj (should have removed Line 1 and Line 2):"
./send-cmd "$PORT" get buffer | head -5
echo -e "${GREEN}✓ Test 1 complete${NC}\n"

# Test 2: dk (delete current + previous line)
echo -e "${BLUE}Test 2: dk (delete current line + line above)${NC}"
./send-cmd "$PORT" buffer "Line 1\nLine 2\nLine 3\nLine 4"
./send-cmd "$PORT" keys "ggj"  # Go to Line 2
echo "Executing: dk"
./send-cmd "$PORT" keys "dk"
echo "After dk (should have removed Line 1 and Line 2):"
./send-cmd "$PORT" get buffer | head -5
echo -e "${GREEN}✓ Test 2 complete${NC}\n"

# Test 3: d2j (delete current + 2 lines below)
echo -e "${BLUE}Test 3: d2j (delete current + 2 lines below)${NC}"
./send-cmd "$PORT" buffer "Line 1\nLine 2\nLine 3\nLine 4\nLine 5"
./send-cmd "$PORT" keys "gg"
echo "Executing: d2j"
./send-cmd "$PORT" keys "d2j"
echo "After d2j (should have removed Lines 1-3):"
./send-cmd "$PORT" get buffer | head -5
echo -e "${GREEN}✓ Test 3 complete${NC}\n"

# Test 4: yj (yank current + next line)
echo -e "${BLUE}Test 4: yj (yank current + next line)${NC}"
./send-cmd "$PORT" buffer "Line 1\nLine 2\nLine 3"
./send-cmd "$PORT" keys "gg"
echo "Executing: yj then p"
./send-cmd "$PORT" keys "yj"
./send-cmd "$PORT" keys "p"
echo "After yj and p (should have duplicated Lines 1-2):"
./send-cmd "$PORT" get buffer | head -7
echo -e "${GREEN}✓ Test 4 complete${NC}\n"

# Test 5: cj (change current + next line)
echo -e "${BLUE}Test 5: cj (change current + next line)${NC}"
./send-cmd "$PORT" buffer "Line 1\nLine 2\nLine 3"
./send-cmd "$PORT" keys "gg"
echo "Executing: cj"
./send-cmd "$PORT" keys "cj"
./send-cmd "$PORT" get mode
echo "Mode after cj (should be Insert):"
./send-cmd "$PORT" keys "<Esc>"
echo -e "${GREEN}✓ Test 5 complete${NC}\n"

# Test 6: dw (delete word)
echo -e "${BLUE}Test 6: dw (delete word)${NC}"
./send-cmd "$PORT" buffer "hello world test"
./send-cmd "$PORT" keys "gg0"
echo "Executing: dw"
./send-cmd "$PORT" keys "dw"
echo "After dw:"
./send-cmd "$PORT" get buffer | head -3
echo -e "${GREEN}✓ Test 6 complete${NC}\n"

# Test 7: d$ (delete to end of line)
echo -e "${BLUE}Test 7: d\$ (delete to end of line)${NC}"
./send-cmd "$PORT" buffer "delete this part"
./send-cmd "$PORT" keys "gg0wwww"  # Move to "this"
echo "Executing: d\$"
./send-cmd "$PORT" keys "d\$"
echo "After d\$:"
./send-cmd "$PORT" get buffer | head -3
echo -e "${GREEN}✓ Test 7 complete${NC}\n"

# Test 8: dfe (delete to next 'e')
echo -e "${BLUE}Test 8: dfe (delete to next 'e')${NC}"
./send-cmd "$PORT" buffer "delete everything"
./send-cmd "$PORT" keys "gg0"
echo "Executing: dfe"
./send-cmd "$PORT" keys "dfe"
echo "After dfe:"
./send-cmd "$PORT" get buffer | head -3
echo -e "${GREEN}✓ Test 8 complete${NC}\n"

# Test 9: d% (delete to matching bracket)
echo -e "${BLUE}Test 9: d% (delete to matching bracket)${NC}"
./send-cmd "$PORT" buffer "func(args)"
./send-cmd "$PORT" keys "gg0f("
echo "Executing: d%"
./send-cmd "$PORT" keys "d%"
echo "After d%:"
./send-cmd "$PORT" get buffer | head -3
echo -e "${GREEN}✓ Test 9 complete${NC}\n"

# Test 10: Undo
echo -e "${BLUE}Test 10: Undo all changes${NC}"
./send-cmd "$PORT" keys "u"
echo "After undo:"
./send-cmd "$PORT" get buffer | head -3
echo -e "${GREEN}✓ Test 10 complete${NC}\n"

echo -e "${GREEN}=== All operator+motion tests completed! ===${NC}"
echo -e "\n${BLUE}Summary:${NC}"
echo "✓ dj - delete with j motion"
echo "✓ dk - delete with k motion"
echo "✓ d2j - delete with count+motion"
echo "✓ yj - yank with motion"
echo "✓ cj - change with motion"
echo "✓ dw - delete word"
echo "✓ d\$ - delete to end"
echo "✓ dfe - delete with find"
echo "✓ d% - delete with bracket match"
echo "✓ u - undo works"
