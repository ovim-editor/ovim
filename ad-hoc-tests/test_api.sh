#!/bin/bash

# REST API Test Script for ovim
# Start ovim with: cargo run -- test.txt --expose-rest-api
# Then run this script in another terminal: ./test_api.sh

API_URL="http://localhost:3000"
BOLD='\033[1m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${BOLD}=== Testing ovim REST API ===${NC}\n"

# Helper function to pretty print JSON
pretty_json() {
    if command -v jq &> /dev/null; then
        echo "$1" | jq '.'
    else
        echo "$1"
    fi
}

# Test 1: Get initial snapshot
echo -e "${BLUE}${BOLD}Test 1: Get initial snapshot${NC}"
response=$(curl -s $API_URL/snapshot)
echo "Response:"
pretty_json "$response"
echo ""

# Test 2: Get buffer content
echo -e "${BLUE}${BOLD}Test 2: Get buffer content${NC}"
response=$(curl -s $API_URL/buffer)
echo "Response:"
pretty_json "$response"
echo ""

# Test 3: Get cursor position
echo -e "${BLUE}${BOLD}Test 3: Get cursor position${NC}"
response=$(curl -s $API_URL/cursor)
echo "Response:"
pretty_json "$response"
echo ""

# Test 4: Get mode
echo -e "${BLUE}${BOLD}Test 4: Get current mode${NC}"
response=$(curl -s $API_URL/mode)
echo "Response:"
pretty_json "$response"
echo ""

# Test 5: Set buffer content
echo -e "${BLUE}${BOLD}Test 5: Set buffer content${NC}"
response=$(curl -s -X PUT $API_URL/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Hello, World!\nThis is line 2.\nThis is line 3.\nFinal line."}')
echo "Response:"
pretty_json "$response"
echo ""

# Test 6: Navigate with hjkl
echo -e "${BLUE}${BOLD}Test 6: Navigate down with 'j' (3 times)${NC}"
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "jjj"}')
echo "Response:"
pretty_json "$response"
response=$(curl -s $API_URL/cursor)
echo "Cursor after:"
pretty_json "$response"
echo ""

# Test 7: Navigate with gg (go to top)
echo -e "${BLUE}${BOLD}Test 7: Go to top with 'gg'${NC}"
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "gg"}')
response=$(curl -s $API_URL/cursor)
echo "Cursor after 'gg':"
pretty_json "$response"
echo ""

# Test 8: Navigate with G (go to bottom)
echo -e "${BLUE}${BOLD}Test 8: Go to bottom with 'G'${NC}"
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "G"}')
response=$(curl -s $API_URL/cursor)
echo "Cursor after 'G':"
pretty_json "$response"
echo ""

# Test 9: Navigate right with 'l'
echo -e "${BLUE}${BOLD}Test 9: Navigate right with 'l' (5 times)${NC}"
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "lllll"}')
response=$(curl -s $API_URL/cursor)
echo "Cursor after 'lllll':"
pretty_json "$response"
echo ""

# Test 10: Enter insert mode and type
echo -e "${BLUE}${BOLD}Test 10: Enter insert mode, type, exit${NC}"
curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "gg"}' > /dev/null
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "iINSERTED: "}')
response=$(curl -s $API_URL/mode)
echo "Mode after 'i':"
pretty_json "$response"
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "<Esc>"}')
response=$(curl -s $API_URL/mode)
echo "Mode after '<Esc>':"
pretty_json "$response"
response=$(curl -s $API_URL/buffer)
echo "Buffer content:"
pretty_json "$response"
echo ""

# Test 11: Delete line with dd
echo -e "${BLUE}${BOLD}Test 11: Delete current line with 'dd'${NC}"
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "dd"}')
response=$(curl -s $API_URL/buffer)
echo "Buffer after 'dd':"
pretty_json "$response"
echo ""

# Test 12: Undo with u
echo -e "${BLUE}${BOLD}Test 12: Undo with 'u'${NC}"
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "u"}')
response=$(curl -s $API_URL/buffer)
echo "Buffer after 'u':"
pretty_json "$response"
echo ""

# Test 13: Redo with Ctrl-R
echo -e "${BLUE}${BOLD}Test 13: Redo with '<C-r>'${NC}"
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "<C-r>"}')
response=$(curl -s $API_URL/buffer)
echo "Buffer after Ctrl-R:"
pretty_json "$response"
echo ""

# Test 14: Delete word with dw
echo -e "${BLUE}${BOLD}Test 14: Delete word with 'dw'${NC}"
curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "gg0"}' > /dev/null
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "dw"}')
response=$(curl -s $API_URL/buffer)
echo "Buffer after 'dw':"
pretty_json "$response"
echo ""

# Test 15: Yank line with yy
echo -e "${BLUE}${BOLD}Test 15: Yank line with 'yy'${NC}"
curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "gg"}' > /dev/null
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "yy"}')
echo "Yanked line (check registers in snapshot)"
echo ""

# Test 16: Paste with p
echo -e "${BLUE}${BOLD}Test 16: Paste with 'p'${NC}"
curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "j"}' > /dev/null
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "p"}')
response=$(curl -s $API_URL/buffer)
echo "Buffer after paste:"
pretty_json "$response"
echo ""

# Test 17: Visual mode
echo -e "${BLUE}${BOLD}Test 17: Visual mode with 'v' and movement${NC}"
curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "gg0"}' > /dev/null
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "vllll"}')
response=$(curl -s $API_URL/mode)
echo "Mode after 'vllll':"
pretty_json "$response"
response=$(curl -s $API_URL/snapshot)
echo "Snapshot (check visual_selection):"
pretty_json "$response" | head -30
echo ""

# Test 18: Delete in visual mode
echo -e "${BLUE}${BOLD}Test 18: Delete visual selection with 'd'${NC}"
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "d"}')
response=$(curl -s $API_URL/buffer)
echo "Buffer after visual delete:"
pretty_json "$response"
echo ""

# Test 19: Search forward
echo -e "${BLUE}${BOLD}Test 19: Search forward with '/line'${NC}"
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "/line<CR>"}')
response=$(curl -s $API_URL/cursor)
echo "Cursor after search:"
pretty_json "$response"
echo ""

# Test 20: Search next with n
echo -e "${BLUE}${BOLD}Test 20: Find next match with 'n'${NC}"
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "n"}')
response=$(curl -s $API_URL/cursor)
echo "Cursor after 'n':"
pretty_json "$response"
echo ""

# Test 21: Execute command :w
echo -e "${BLUE}${BOLD}Test 21: Execute :w command${NC}"
response=$(curl -s -X POST $API_URL/command \
  -H "Content-Type: application/json" \
  -d '{"command": "w"}')
echo "Response:"
pretty_json "$response"
echo ""

# Test 22: Count prefix (5j = move down 5 lines)
echo -e "${BLUE}${BOLD}Test 22: Count prefix - '5j' to move down 5 lines${NC}"
curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "gg"}' > /dev/null
before=$(curl -s $API_URL/cursor)
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "5j"}')
after=$(curl -s $API_URL/cursor)
echo "Cursor before '5j':"
pretty_json "$before"
echo "Cursor after '5j':"
pretty_json "$after"
echo ""

# Test 23: Change word with cw
echo -e "${BLUE}${BOLD}Test 23: Change word with 'cw'${NC}"
curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "gg0"}' > /dev/null
response=$(curl -s -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "cwNEWWORD<Esc>"}')
response=$(curl -s $API_URL/buffer)
echo "Buffer after 'cwNEWWORD<Esc>':"
pretty_json "$response"
echo ""

# Test 24: Final snapshot
echo -e "${BLUE}${BOLD}Test 24: Final full snapshot${NC}"
response=$(curl -s $API_URL/snapshot)
echo "Final snapshot:"
pretty_json "$response"
echo ""

echo -e "${GREEN}${BOLD}=== All tests completed ===${NC}"
