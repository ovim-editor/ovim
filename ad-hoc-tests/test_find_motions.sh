#!/usr/bin/env zsh

# Test script for f/F/t/T find motions
# Usage: Start ovim manually, then run: ./test_find_motions.sh <port>

set -euo pipefail

if [[ -z "${1:-}" ]]; then
    echo "Usage: $0 <port>"
    echo ""
    echo "Steps:"
    echo "1. Start ovim: cargo run -- test_find.txt --expose-rest-api"
    echo "2. Note the port from 'API URL'"
    echo "3. Run: ./test_find_motions.sh <port>"
    exit 1
fi

PORT="$1"
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${BLUE}=== Testing f/F/t/T Find Character Motions ===${NC}\n"

# Setup test content
echo -e "${BLUE}Setting up test buffer...${NC}"
./send-cmd "$PORT" buffer "The quick brown fox jumps over the lazy dog"
./send-cmd "$PORT" get buffer | head -3
echo ""

# Test 1: f motion (find forward)
echo -e "${BLUE}Test 1: f motion - find next 'o'${NC}"
./send-cmd "$PORT" keys "gg0"  # Go to start
echo "Current cursor:"
./send-cmd "$PORT" get cursor
echo "Sending: fo"
./send-cmd "$PORT" keys "fo"
echo "After fo (should be at first 'o' in 'brown'):"
./send-cmd "$PORT" get cursor
echo -e "${GREEN}✓ Test 1 complete${NC}\n"

# Test 2: f with count
echo -e "${BLUE}Test 2: 2fo - find 2nd 'o'${NC}"
./send-cmd "$PORT" keys "gg0"  # Reset
echo "Sending: 2fo"
./send-cmd "$PORT" keys "2fo"
echo "After 2fo (should be at second 'o' in 'fox'):"
./send-cmd "$PORT" get cursor
echo -e "${GREEN}✓ Test 2 complete${NC}\n"

# Test 3: F motion (find backward)
echo -e "${BLUE}Test 3: F motion - find previous 'u'${NC}"
./send-cmd "$PORT" keys "gg$"  # Go to end
echo "Current cursor (at end):"
./send-cmd "$PORT" get cursor
echo "Sending: Fu"
./send-cmd "$PORT" keys "Fu"
echo "After Fu (should be at 'u' in 'jumps'):"
./send-cmd "$PORT" get cursor
echo -e "${GREEN}✓ Test 3 complete${NC}\n"

# Test 4: t motion (till forward)
echo -e "${BLUE}Test 4: t motion - till next 'x'${NC}"
./send-cmd "$PORT" keys "gg0"  # Reset
echo "Sending: tx"
./send-cmd "$PORT" keys "tx"
echo "After tx (should be ONE BEFORE 'x' in 'fox'):"
./send-cmd "$PORT" get cursor
echo -e "${GREEN}✓ Test 4 complete${NC}\n"

# Test 5: T motion (till backward)
echo -e "${BLUE}Test 5: T motion - till previous 'q'${NC}"
./send-cmd "$PORT" keys "gg\$"  # Go to end
echo "Sending: Tq"
./send-cmd "$PORT" keys "Tq"
echo "After Tq (should be ONE AFTER 'q' in 'quick'):"
./send-cmd "$PORT" get cursor
echo -e "${GREEN}✓ Test 5 complete${NC}\n"

# Test 6: ; to repeat
echo -e "${BLUE}Test 6: ; to repeat last find${NC}"
./send-cmd "$PORT" keys "gg0"  # Reset
./send-cmd "$PORT" keys "fe"  # Find first 'e'
echo "After fe (first 'e' in 'The'):"
./send-cmd "$PORT" get cursor
echo "Sending: ; (should find next 'e' in 'over')"
./send-cmd "$PORT" keys ";"
echo "After ; :"
./send-cmd "$PORT" get cursor
echo -e "${GREEN}✓ Test 6 complete${NC}\n"

# Test 7: , to repeat in opposite direction
echo -e "${BLUE}Test 7: , to repeat in opposite direction${NC}"
echo "Sending: , (should go back to previous 'e' in 'the')"
./send-cmd "$PORT" keys ","
echo "After , :"
./send-cmd "$PORT" get cursor
echo -e "${GREEN}✓ Test 7 complete${NC}\n"

# Test 8: f with operator (df)
echo -e "${BLUE}Test 8: df with delete operator${NC}"
./send-cmd "$PORT" keys "gg0"  # Reset
echo "Before dfo:"
./send-cmd "$PORT" get buffer | head -3
echo "Sending: dfo (delete up to and including first 'o')"
./send-cmd "$PORT" keys "dfo"
echo "After dfo:"
./send-cmd "$PORT" get buffer | head -3
echo -e "${GREEN}✓ Test 8 complete${NC}\n"

# Test 9: Undo the delete
echo -e "${BLUE}Test 9: Undo with u${NC}"
./send-cmd "$PORT" keys "u"
echo "After undo:"
./send-cmd "$PORT" get buffer | head -3
echo -e "${GREEN}✓ Test 9 complete${NC}\n"

echo -e "${GREEN}=== All find motion tests passed! ===${NC}"
