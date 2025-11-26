# Manual Testing Guide for ovim REST API

TODO: When you see this, immediately refactor this to use ovim-ctl.

## Setup

1. **Start ovim with REST API enabled:**
   ```bash
   cargo run -- test.txt --expose-rest-api
   ```

2. **Note the API URL** printed to stderr (e.g., `http://127.0.0.1:56789`)

3. **In a separate terminal**, run the curl commands below (replace `$API_URL` with the actual URL)

## Test Cases

### 1. Basic Health Check

```bash
# Get current mode
curl http://127.0.0.1:XXXX/mode

# Expected: {"mode":"Normal"}
```

### 2. Get Initial State

```bash
# Get full snapshot
curl http://127.0.0.1:XXXX/snapshot | jq '.'

# Get buffer content
curl http://127.0.0.1:XXXX/buffer | jq '.'

# Get cursor position
curl http://127.0.0.1:XXXX/cursor | jq '.'
```

### 3. Set Buffer Content

```bash
curl -X PUT http://127.0.0.1:XXXX/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Line 1: Hello World\nLine 2: Testing ovim\nLine 3: REST API\nLine 4: Final line"}'

# Verify
curl http://127.0.0.1:XXXX/buffer | jq '.content'
```

### 4. Navigation - Basic Movement

```bash
# Move to top with gg
curl -X POST http://127.0.0.1:XXXX/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "gg"}'

# Check cursor (should be at line 0)
curl http://127.0.0.1:XXXX/cursor
# Expected: {"line":0,"column":0}

# Move down 2 lines with jj
curl -X POST http://127.0.0.1:XXXX/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "jj"}'

# Check cursor (should be at line 2)
curl http://127.0.0.1:XXXX/cursor
# Expected: {"line":2,"column":0}

# Move to bottom with G
curl -X POST http://127.0.0.1:XXXX/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "G"}'

# Check cursor (should be at last line)
curl http://127.0.0.1:XXXX/cursor
```

### 5. Navigation - Horizontal Movement

```bash
# Go to top
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "gg"}'

# Move right 5 characters
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "lllll"}'

# Check cursor
curl http://127.0.0.1:XXXX/cursor
# Expected: {"line":0,"column":5}

# Move to start of line with 0
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "0"}'

# Check cursor
curl http://127.0.0.1:XXXX/cursor
# Expected: {"line":0,"column":0}
```

### 6. Insert Mode and Editing

```bash
# Go to top
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "gg"}'

# Enter insert mode
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "i"}'

# Check mode
curl http://127.0.0.1:XXXX/mode
# Expected: {"mode":"Insert"}

# Type some text
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "PREFIX: "}'

# Exit insert mode
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "<Esc>"}'

# Check mode
curl http://127.0.0.1:XXXX/mode
# Expected: {"mode":"Normal"}

# Check buffer
curl http://127.0.0.1:XXXX/buffer | jq '.content'
# Should see "PREFIX: Line 1: Hello World"
```

### 7. Delete Operations

```bash
# Go to top
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "gg"}'

# Delete current line with dd
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "dd"}'

# Check buffer (first line should be gone)
curl http://127.0.0.1:XXXX/buffer | jq '.content'
```

### 8. Undo and Redo

```bash
# Undo the delete
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "u"}'

# Check buffer (line should be back)
curl http://127.0.0.1:XXXX/buffer | jq '.content'

# Redo
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "<C-r>"}'

# Check buffer (line should be gone again)
curl http://127.0.0.1:XXXX/buffer | jq '.content'
```

### 9. Visual Mode

```bash
# Reset buffer
curl -X PUT http://127.0.0.1:XXXX/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Select this text please"}'

# Go to start
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "gg0"}'

# Enter visual mode and select
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "vllllll"}'

# Check mode
curl http://127.0.0.1:XXXX/mode
# Expected: {"mode":"Visual"}

# Check snapshot for visual selection
curl http://127.0.0.1:XXXX/snapshot | jq '.visual_selection'

# Delete selection
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "d"}'

# Check buffer
curl http://127.0.0.1:XXXX/buffer | jq '.content'
```

### 10. Yank and Paste

```bash
# Reset buffer
curl -X PUT http://127.0.0.1:XXXX/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Copy this line\nOriginal line"}'

# Go to top
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "gg"}'

# Yank line
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "yy"}'

# Move down
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "j"}'

# Paste
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "p"}'

# Check buffer (should have duplicated line)
curl http://127.0.0.1:XXXX/buffer | jq '.content'
```

### 11. Search

```bash
# Reset buffer
curl -X PUT http://127.0.0.1:XXXX/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "No match here\nFound the target word\nAnother target here"}'

# Go to top
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "gg"}'

# Search for "target"
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "/target<CR>"}'

# Check cursor (should be on line 1)
curl http://127.0.0.1:XXXX/cursor
# Expected: {"line":1,...}

# Find next match
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "n"}'

# Check cursor (should be on line 2)
curl http://127.0.0.1:XXXX/cursor
# Expected: {"line":2,...}
```

### 12. Count Prefixes

```bash
# Reset buffer
curl -X PUT http://127.0.0.1:XXXX/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\nLine 6\nLine 7\nLine 8"}'

# Go to top
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "gg"}'

# Move down 5 lines with 5j
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "5j"}'

# Check cursor
curl http://127.0.0.1:XXXX/cursor
# Expected: {"line":5,...}

# Delete 3 lines with 3dd
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "gg3dd"}'

# Check buffer (first 3 lines should be gone)
curl http://127.0.0.1:XXXX/buffer | jq '.content'
```

### 13. Ex Commands

```bash
# Execute :w (write) command
curl -X POST http://127.0.0.1:XXXX/command \
  -H "Content-Type: application/json" \
  -d '{"command": "w"}'

# Expected: success response with file info
```

### 14. Complex Workflow - Realistic Editing

```bash
# Setup a Python function
curl -X PUT http://127.0.0.1:XXXX/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "def greet(name):\n    print(\"Hello, {}\".format(name))\n    return"}'

# Navigate to "name" parameter and change it to "user"
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "gg"}'
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "/name<CR>"}'
curl -X POST http://127.0.0.1:XXXX/keys -H "Content-Type: application/json" -d '{"keys": "cwuser<Esc>"}'

# Check result
curl http://127.0.0.1:XXXX/buffer | jq '.content'
# Should show "def greet(user):"
```

### 15. Full Snapshot Check

```bash
# Get complete editor state
curl http://127.0.0.1:XXXX/snapshot | jq '.' > snapshot.json

# Inspect:
# - buffer content
# - cursor position
# - current mode
# - visual selection (if any)
# - registers content
# - marks
```

## Expected Vim-like Behavior

All operations should behave like Neovim:
- ✓ Modal editing (Normal, Insert, Visual modes)
- ✓ Navigation (hjkl, gg, G, 0, $, w, b)
- ✓ Operators + Motions (d, c, y with motions)
- ✓ Count prefixes (5j, 3dd, 2yy)
- ✓ Visual selection and operations
- ✓ Undo/Redo (u, Ctrl-R)
- ✓ Yank/Paste (yy, dd, p, P)
- ✓ Search (/, ?, n, N)
- ✓ Insert/Append (i, I, a, A, o, O)
- ✓ Ex commands (:w, :q, :wq)
