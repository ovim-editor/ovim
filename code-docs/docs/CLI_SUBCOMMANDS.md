# CLI Subcommands - AI-First IDE

ovim now includes built-in session control, eliminating the need for `ovim-ctl`. The same binary acts as both editor and client.

## Quick Start

```bash
# Start editor (default behavior)
ovim file.rs                                     # Edit in TUI
ovim --headless --session dev file.rs           # Headless mode

# Control sessions (new!)
ovim sessions                                    # List all sessions
ovim send dev "ggdd"                            # Send keys
ovim mcp dev tools/list                         # MCP request
ovim kill dev                                   # Kill session
```

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                   ovim binary                       │
├─────────────────────────────────────────────────────┤
│                                                     │
│  No subcommand        Subcommand given             │
│  ↓                    ↓                             │
│  Editor Mode          Client Mode                  │
│  (TUI/Headless)       (HTTP client)                │
│                                                     │
└─────────────────────────────────────────────────────┘
```

## Subcommands

### `ovim sessions`

List all running ovim sessions with status.

```bash
ovim sessions
```

**Output:**
```
Running ovim sessions:

SESSION         PID      PORT       LSP        FILE
────────────────────────────────────────────────────────────────────────────────
dev             12345    8080       ready      src/main.rs
test            12346    8081       pending    tests/test.rs
```

### `ovim send <session> <keys>`

Send Vim key sequence to a session.

```bash
ovim send dev "gg"                    # Go to top
ovim send dev "10j"                   # Move down 10 lines
ovim send dev "dd"                    # Delete line
ovim send dev "iHello<Esc>"           # Insert text
```

### `ovim exec <session> <command>`

Execute an ex command.

```bash
ovim exec dev "w"                     # Save file
ovim exec dev "q"                     # Quit
ovim exec dev "%s/foo/bar/g"          # Replace all
```

### `ovim snapshot <session> [--format=json|pretty]`

Get complete editor state snapshot.

```bash
ovim snapshot dev                     # JSON output
ovim snapshot dev --format pretty     # Human-readable
```

**Pretty output:**
```
Session: dev
Mode: NORMAL
Cursor: line 42, col 5
Buffer: 350 lines
File: /path/to/file.rs
Registers: 3
Marks: 5
```

### `ovim buffer <session>`

Get raw buffer content.

```bash
ovim buffer dev                       # Print buffer
ovim buffer dev > output.txt          # Save to file
```

### `ovim mcp <session> <method> [params] [--id=N]`

Send MCP JSON-RPC request.

```bash
# Initialize
ovim mcp dev initialize '{
  "protocolVersion":"2025-03-26",
  "capabilities":{},
  "clientInfo":{"name":"cli","version":"1.0"}
}'

# List tools
ovim mcp dev tools/list

# Call tool
ovim mcp dev tools/call '{
  "name":"send_keys",
  "arguments":{"keys":"gg"}
}'

# Read resource
ovim mcp dev resources/read '{"uri":"ovim://buffer"}'
```

### `ovim kill <session>`

Kill a running session.

```bash
ovim kill dev
```

Sends SIGTERM, waits 500ms, then SIGKILL if needed. Cleans up session file automatically.

### `ovim health <session>`

Check session health and LSP status.

```bash
ovim health dev
```

**Output:**
```
Session: dev
Status: healthy
Uptime: 1234 seconds
File: /path/to/file.rs
Ready: true

LSP Servers:
  rust: ready
  javascript: initializing
```

### `ovim lsp-status <session>`

Get detailed LSP server information.

```bash
ovim lsp-status dev
```

Shows language, command, state, pending requests, and capabilities for all LSP servers.

## AI-First IDE Workflows

### Multi-File Editing

AI agents can spawn multiple sessions and coordinate edits:

```bash
# Spawn sessions for each file
ovim --headless --session main src/main.rs &
ovim --headless --session lib src/lib.rs &
ovim --headless --session tests tests/test.rs &

# Wait for LSP
sleep 2

# Query state
ovim snapshot main | jq '.buffer.line_count'
ovim buffer lib | grep "pub fn"

# Make coordinated edits
ovim send main "gg/struct Config<CR>cwNewConfig<Esc>"
ovim send lib "gg/pub struct Config<CR>cwpub struct NewConfig<Esc>"

# Verify
ovim mcp main resources/read '{"uri":"ovim://buffer"}' | jq '.result.contents[0].text'

# Cleanup
ovim kill main lib tests
```

### Refactoring Workflow

```bash
# Start session
ovim --headless --session refactor src/service.rs &

# Get current implementation
CURRENT=$(ovim buffer refactor)

# AI generates new implementation
# ... AI processing ...

# Apply changes via MCP
ovim mcp refactor tools/call '{
  "name":"set_buffer",
  "arguments":{"content":"'"$NEW_IMPL"'"}
}'

# Run tests
ovim exec refactor "!cargo test"

# Verify with LSP
ovim lsp-status refactor | grep "ready"

# Commit if tests pass
if [ $? -eq 0 ]; then
  ovim exec refactor "w"
  git add src/service.rs
  git commit -m "Refactor service"
fi
```

### Code Review Workflow

```bash
# Start review session
ovim --headless --session review src/feature.rs &

# Get LSP diagnostics
ovim mcp review tools/call '{"name":"lsp_hover","arguments":{}}' \
  | jq '.result.content'

# Navigate and inspect
ovim send review "gg"
for line in $(seq 1 100); do
  ovim send review "j"
  SNAPSHOT=$(ovim snapshot review --format pretty)
  # AI analyzes each line
done

# Leave comments (in separate file)
ovim --headless --session comments review_comments.md &
ovim send comments "iLine $line: Issue found<Esc>"
```

### Parallel Linting

```bash
# Spawn session for each file
for file in src/*.rs; do
  session=$(basename "$file" .rs)
  ovim --headless --session "$session" "$file" &
done

# Wait for LSP
sleep 3

# Collect diagnostics from all sessions
ovim sessions | tail -n +3 | awk '{print $1}' | while read session; do
  echo "=== $session ===" ovim lsp-status "$session" | grep "ready" && echo "OK" || echo "ISSUES"
done

# Cleanup
ovim sessions | tail -n +3 | awk '{print $1}' | xargs ovim kill
```

### Live Collaboration (AI + Human)

```bash
# Human starts editing
ovim src/main.rs --headless --session collab &

# AI monitors changes
while true; do
  SNAPSHOT=$(ovim snapshot collab --format json)
  CURSOR=$(echo "$SNAPSHOT" | jq '.cursor.line')
  MODE=$(echo "$SNAPSHOT" | jq -r '.mode')

  # AI suggests improvements based on context
  if [ "$MODE" = "NORMAL" ]; then
    # AI can insert suggestions
    ovim send collab "o// AI suggestion: ..."
  fi

  sleep 1
done
```

## MCP Integration Examples

### Spawn, Edit, Verify Pattern

```python
import subprocess
import json

def ovim_session(session, file):
    """Start ovim session"""
    subprocess.Popen([
        "ovim", file, "--headless", "--session", session
    ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(2)  # Wait for startup

def ovim_mcp(session, method, params={}):
    """Send MCP request"""
    result = subprocess.run([
        "ovim", "mcp", session, method, json.dumps(params)
    ], capture_output=True, text=True)
    return json.loads(result.stdout)

# AI workflow
ovim_session("edit", "src/main.rs")

# Get current code
result = ovim_mcp("edit", "resources/read", {"uri": "ovim://buffer"})
code = result["result"]["contents"][0]["text"]

# AI generates changes
new_code = ai_transform(code)

# Apply changes
ovim_mcp("edit", "tools/call", {
    "name": "set_buffer",
    "arguments": {"content": new_code}
})

# Verify with LSP
lsp_status = ovim_mcp("edit", "tools/call", {"name": "lsp_hover"})

# Cleanup
subprocess.run(["ovim", "kill", "edit"])
```

## Session Discovery

Sessions are automatically discovered from `~/.cache/ovim/sessions/*.json`:

```bash
# List all session files
ls -1 ~/.cache/ovim/sessions/

# Read session info
cat ~/.cache/ovim/sessions/dev.json | jq '.'

# Output:
{
  "pid": 12345,
  "port": 8080,
  "file": "src/main.rs",
  "started_at": 1234567890,
  "session_name": "dev",
  "lsp_ready": true,
  "start_time": 1234567890
}
```

Sessions are automatically cleaned up when:
- Process exits normally
- SIGTERM/SIGINT received
- `ovim kill` executed
- PID no longer exists (stale file detection)

## Comparison with ovim-ctl

| Feature | ovim-ctl (old) | ovim subcommands (new) |
|---------|----------------|------------------------|
| Binary | Separate shell script | Integrated into ovim |
| Session discovery | Manual port lookup | Automatic |
| MCP support | None | Full JSON-RPC |
| Error handling | Basic | Comprehensive |
| Output format | Plain text | Pretty + JSON |
| Dependencies | jq, curl, bash | None (all built-in) |

## Implementation

- **CLI**: `src/cli.rs` - Clap subcommands
- **Client**: `src/client.rs` - HTTP/MCP client
- **Subcommands**: `src/subcommands.rs` - Command implementations
- **Session**: `src/session.rs` - Session management

All operations use the existing REST/MCP API, ensuring consistency between TUI and CLI interfaces.
