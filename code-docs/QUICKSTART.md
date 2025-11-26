# ovim REST API Quick Start

## 1. Start the Editor

```bash
cargo run -- test.txt --expose-rest-api
```

You'll see output like:
```
REST API server listening on http://127.0.0.1:56789
API URL: http://127.0.0.1:56789
```

**Note the port number!** (56789 in this example)

## 2. Test in Another Terminal

Replace `56789` with your actual port:

```bash
export API="http://127.0.0.1:56789"

# Check it's working
curl $API/mode
# Expected: {"mode":"Normal"}

# Set some content
curl -X PUT $API/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Line 1\nLine 2\nLine 3"}'

# Navigate to top
curl -X POST $API/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "gg"}'

# Check cursor (should be 0,0)
curl $API/cursor

# Enter insert mode
curl -X POST $API/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "i"}'

# Type something
curl -X POST $API/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "Hello! "}'

# Exit insert mode
curl -X POST $API/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "<Esc>"}'

# Check the buffer
curl $API/buffer | jq '.content'

# Get full snapshot
curl $API/snapshot | jq '.'
```

## 3. Run Automated Tests

```bash
# Quick manual tests
./manual_test.sh 56789

# Or comprehensive test suite
./test_api.sh
# (Update the API_URL in the script first)
```

## Common Operations

### Navigation
```bash
curl -X POST $API/keys -H "Content-Type: application/json" -d '{"keys": "gg"}'  # top
curl -X POST $API/keys -H "Content-Type: application/json" -d '{"keys": "G"}'   # bottom
curl -X POST $API/keys -H "Content-Type: application/json" -d '{"keys": "5j"}'  # down 5
curl -X POST $API/keys -H "Content-Type: application/json" -d '{"keys": "10l"}' # right 10
```

### Editing
```bash
# Delete line
curl -X POST $API/keys -H "Content-Type: application/json" -d '{"keys": "dd"}'

# Yank line
curl -X POST $API/keys -H "Content-Type: application/json" -d '{"keys": "yy"}'

# Paste
curl -X POST $API/keys -H "Content-Type: application/json" -d '{"keys": "p"}'

# Undo
curl -X POST $API/keys -H "Content-Type: application/json" -d '{"keys": "u"}'

# Redo
curl -X POST $API/keys -H "Content-Type: application/json" -d '{"keys": "<C-r>"}'
```

### Commands
```bash
# Save
curl -X POST $API/command -H "Content-Type: application/json" -d '{"command": "w"}'

# Quit (only if no changes)
curl -X POST $API/command -H "Content-Type: application/json" -d '{"command": "q"}'

# Force quit
curl -X POST $API/command -H "Content-Type: application/json" -d '{"command": "q!"}'

# Save and quit
curl -X POST $API/command -H "Content-Type: application/json" -d '{"command": "wq"}'
```

### Search
```bash
# Search for "target"
curl -X POST $API/keys -H "Content-Type: application/json" -d '{"keys": "/target<CR>"}'

# Next match
curl -X POST $API/keys -H "Content-Type: application/json" -d '{"keys": "n"}'

# Previous match
curl -X POST $API/keys -H "Content-Type: application/json" -d '{"keys": "N"}'
```

## Tips

1. **Use jq for pretty output**
   ```bash
   curl $API/snapshot | jq '.'
   ```

2. **Create a helper function**
   ```bash
   send() { curl -s -X POST $API/keys -H "Content-Type: application/json" -d "{\"keys\": \"$1\"}"; }
   send "ggdG"  # Clear buffer
   ```

3. **Watch for changes**
   ```bash
   watch -n 1 "curl -s $API/cursor | jq ."
   ```

4. **Test scripts**
   ```bash
   # All test scripts need the port
   ./manual_test.sh 56789
   ```

## Troubleshooting

**API not responding?**
- Check ovim is still running
- Verify the port number
- Try `curl -v` to see detailed errors

**Terminal issues?**
- ovim needs a terminal to run (can't background with `&`)
- Use separate terminal windows/tabs
- Or use tmux/screen

**Build issues?**
```bash
cargo clean
cargo build --release
```

## Next Steps

- Read `TESTING.md` for comprehensive test cases
- Read `README.md` for full documentation
- Read `IMPLEMENTATION_SUMMARY.md` for architecture details
- Try the automated tests: `./run_tests.sh`
- Experiment with complex workflows
- Build your own integrations!
