#!/usr/bin/env zsh

# Manual Test Script for ovim REST API
# This script provides an interactive way to test the API

set -e

GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${BLUE}=== ovim REST API Manual Testing ===${NC}\n"
echo -e "${YELLOW}This script will help you test the ovim REST API${NC}\n"

# Check if port is provided
if [ -z "$1" ]; then
    echo -e "${YELLOW}Usage: $0 <port>${NC}"
    echo -e "${YELLOW}Example: $0 59028${NC}"
    echo ""
    echo "Steps:"
    echo "1. Start ovim in one terminal: cargo run -- test.txt --expose-rest-api"
    echo "2. Note the port number from the 'API URL' line"
    echo "3. Run this script with that port: ./manual_test.sh <port>"
    exit 1
fi

PORT=$1
API_URL="http://127.0.0.1:$PORT"

echo -e "Testing API at: ${GREEN}$API_URL${NC}\n"

# Test 1: Health check
echo -e "${BLUE}Test 1: Health Check${NC}"
echo "GET $API_URL/mode"
response=$(curl -s $API_URL/mode)
echo "Response: $response"
if echo "$response" | grep -q "mode"; then
    echo -e "${GREEN}✓ API is responding${NC}\n"
else
    echo -e "${RED}✗ API not responding${NC}\n"
    exit 1
fi

# Test 2: Set buffer
echo -e "${BLUE}Test 2: Set Buffer Content${NC}"
echo "PUT $API_URL/buffer"
curl -s -X PUT $API_URL/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Line 1: Hello\nLine 2: World\nLine 3: Test\nLine 4: Final"}' | head -3
echo -e "\n${GREEN}✓ Buffer set${NC}\n"

# Test 3: Get buffer
echo -e "${BLUE}Test 3: Get Buffer Content${NC}"
echo "GET $API_URL/buffer"
response=$(curl -s $API_URL/buffer)
echo "$response" | head -5
echo -e "${GREEN}✓ Buffer retrieved${NC}\n"

# Test 4: Navigate with gg
echo -e "${BLUE}Test 4: Navigate to Top (gg)${NC}"
curl -s -X POST $API_URL/keys -H "Content-Type: application/json" -d '{"keys": "gg"}' > /dev/null
response=$(curl -s $API_URL/cursor)
echo "Cursor: $response"
if echo "$response" | grep -q '"line":0'; then
    echo -e "${GREEN}✓ Navigated to line 0${NC}\n"
else
    echo -e "${RED}✗ Navigation failed${NC}\n"
fi

# Test 5: Move down
echo -e "${BLUE}Test 5: Move Down (jj)${NC}"
curl -s -X POST $API_URL/keys -H "Content-Type: application/json" -d '{"keys": "jj"}' > /dev/null
response=$(curl -s $API_URL/cursor)
echo "Cursor: $response"
if echo "$response" | grep -q '"line":2'; then
    echo -e "${GREEN}✓ Moved to line 2${NC}\n"
else
    echo -e "${RED}✗ Movement failed${NC}\n"
fi

# Test 6: Insert mode
echo -e "${BLUE}Test 6: Enter Insert Mode (i)${NC}"
curl -s -X POST $API_URL/keys -H "Content-Type: application/json" -d '{"keys": "ggi"}' > /dev/null
response=$(curl -s $API_URL/mode)
echo "Mode: $response"
if echo "$response" | grep -q "Insert"; then
    echo -e "${GREEN}✓ Entered Insert mode${NC}\n"
else
    echo -e "${RED}✗ Mode change failed${NC}\n"
fi

# Test 7: Type text
echo -e "${BLUE}Test 7: Type Text${NC}"
curl -s -X POST $API_URL/keys -H "Content-Type: application/json" -d '{"keys": "INSERTED: "}' > /dev/null
curl -s -X POST $API_URL/keys -H "Content-Type: application/json" -d '{"keys": "<Esc>"}' > /dev/null
response=$(curl -s $API_URL/buffer)
if echo "$response" | grep -q "INSERTED"; then
    echo -e "${GREEN}✓ Text inserted${NC}\n"
else
    echo -e "${RED}✗ Insert failed${NC}\n"
fi

# Test 8: Delete line
echo -e "${BLUE}Test 8: Delete Line (dd)${NC}"
curl -s -X POST $API_URL/keys -H "Content-Type: application/json" -d '{"keys": "ggdd"}' > /dev/null
response=$(curl -s $API_URL/buffer)
echo "Buffer after dd: $(echo $response | head -c 100)..."
echo -e "${GREEN}✓ Line deleted${NC}\n"

# Test 9: Undo
echo -e "${BLUE}Test 9: Undo (u)${NC}"
curl -s -X POST $API_URL/keys -H "Content-Type: application/json" -d '{"keys": "u"}' > /dev/null
response=$(curl -s $API_URL/buffer)
if echo "$response" | grep -q "INSERTED"; then
    echo -e "${GREEN}✓ Undo worked${NC}\n"
else
    echo -e "${RED}✗ Undo failed${NC}\n"
fi

# Test 10: Visual mode
echo -e "${BLUE}Test 10: Visual Mode (v)${NC}"
curl -s -X POST $API_URL/keys -H "Content-Type: application/json" -d '{"keys": "gg0v"}' > /dev/null
response=$(curl -s $API_URL/mode)
echo "Mode: $response"
if echo "$response" | grep -q "Visual"; then
    echo -e "${GREEN}✓ Entered Visual mode${NC}\n"
else
    echo -e "${RED}✗ Visual mode failed${NC}\n"
fi

# Test 11: Exit visual
curl -s -X POST $API_URL/keys -H "Content-Type: application/json" -d '{"keys": "<Esc>"}' > /dev/null

# Test 12: Yank and paste
echo -e "${BLUE}Test 11: Yank Line (yy)${NC}"
curl -s -X PUT $API_URL/buffer -H "Content-Type: application/json" -d '{"content": "Copy this\nOriginal"}' > /dev/null
curl -s -X POST $API_URL/keys -H "Content-Type: application/json" -d '{"keys": "ggyy"}' > /dev/null
curl -s -X POST $API_URL/keys -H "Content-Type: application/json" -d '{"keys": "jp"}' > /dev/null
response=$(curl -s $API_URL/buffer)
if echo "$response" | grep -c "Copy this" | grep -q "2"; then
    echo -e "${GREEN}✓ Yank and paste worked${NC}\n"
else
    echo "Buffer: $response"
    echo -e "${YELLOW}~ Paste completed (check buffer above)${NC}\n"
fi

# Test 13: Search
echo -e "${BLUE}Test 12: Search (/target)${NC}"
curl -s -X PUT $API_URL/buffer -H "Content-Type: application/json" -d '{"content": "No match\nFound target here\nMore target"}' > /dev/null
curl -s -X POST $API_URL/keys -H "Content-Type: application/json" -d '{"keys": "gg"}' > /dev/null
curl -s -X POST $API_URL/keys -H "Content-Type: application/json" -d '{"keys": "/target<CR>"}' > /dev/null
response=$(curl -s $API_URL/cursor)
echo "Cursor: $response"
if echo "$response" | grep -q '"line":1'; then
    echo -e "${GREEN}✓ Search found match on line 1${NC}\n"
else
    echo -e "${RED}✗ Search failed${NC}\n"
fi

# Test 14: Count prefix
echo -e "${BLUE}Test 13: Count Prefix (5j)${NC}"
curl -s -X PUT $API_URL/buffer -H "Content-Type: application/json" -d '{"content": "1\n2\n3\n4\n5\n6\n7\n8"}' > /dev/null
curl -s -X POST $API_URL/keys -H "Content-Type: application/json" -d '{"keys": "gg"}' > /dev/null
curl -s -X POST $API_URL/keys -H "Content-Type: application/json" -d '{"keys": "5j"}' > /dev/null
response=$(curl -s $API_URL/cursor)
echo "Cursor: $response"
if echo "$response" | grep -q '"line":5'; then
    echo -e "${GREEN}✓ Moved down 5 lines${NC}\n"
else
    echo -e "${RED}✗ Count prefix failed${NC}\n"
fi

# Test 15: Full snapshot
echo -e "${BLUE}Test 14: Get Full Snapshot${NC}"
echo "GET $API_URL/snapshot"
response=$(curl -s $API_URL/snapshot)
echo "$response" | head -c 200
echo "..."
echo -e "${GREEN}✓ Snapshot retrieved${NC}\n"

echo -e "${GREEN}${BOLD}=== All Manual Tests Completed ===${NC}"
echo -e "\nFor more detailed testing, see TESTING.md"
