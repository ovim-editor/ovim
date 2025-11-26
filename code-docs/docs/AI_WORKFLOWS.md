# AI Workflows - ovim as an AI-First IDE

This document showcases practical AI workflows using ovim's MCP-native architecture and integrated CLI subcommands.

## Philosophy

ovim is designed for **AI agents as first-class users**:

- **Multi-session**: Spawn multiple editors, one per file
- **MCP native**: AI speaks JSON-RPC natively
- **Scriptable**: All operations via CLI
- **Stateless**: Query any session's state at any time
- **Parallel**: Coordinate edits across multiple files

## Workflow 1: AI-Assisted Refactoring

**Scenario**: Rename a struct across multiple files with LSP verification.

```bash
#!/bin/bash
# AI refactoring script

# 1. Spawn sessions for all affected files
for file in src/main.rs src/lib.rs tests/test.rs; do
  session=$(basename "$file" .rs)
  ovim --headless --session "$session" "$file" &
done

sleep 2  # Wait for LSP initialization

# 2. Find all usages with LSP
find_usages() {
  local session=$1
  ovim mcp "$session" tools/call '{
    "name":"send_keys",
    "arguments":{"keys":"ggn"}  # Search for struct
  }' > /dev/null

  ovim snapshot "$session" | jq '.cursor.line'
}

# 3. Apply rename in each file
for session in main lib test; do
  echo "Refactoring $session..."

  # Navigate to struct definition
  ovim send "$session" "gg/struct OldName<CR>"

  # Rename
  ovim send "$session" "cwNewName<Esc>"

  # Verify with LSP (check for errors)
  lsp_ready=$(ovim health "$session" | grep "LSP.*ready" | wc -l)

  if [ "$lsp_ready" -eq 1 ]; then
    echo "  ✓ LSP verified for $session"
    ovim exec "$session" "w"  # Save
  else
    echo "  ✗ LSP errors in $session"
    exit 1
  fi
done

# 4. Cleanup
ovim kill main lib test
echo "Refactoring complete!"
```

## Workflow 2: Intelligent Code Review

**Scenario**: AI reviews code with LSP hover information and diagnostics.

```python
#!/usr/bin/env python3
import subprocess
import json
import time

class OvimSession:
    def __init__(self, session_name, file_path):
        self.session = session_name
        subprocess.Popen([
            "ovim", file_path,
            "--headless",
            "--session", session_name
        ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(2)  # Wait for LSP

    def mcp(self, method, params={}):
        result = subprocess.run([
            "ovim", "mcp", self.session, method,
            json.dumps(params)
        ], capture_output=True, text=True)
        return json.loads(result.stdout)

    def send_keys(self, keys):
        subprocess.run(["ovim", "send", self.session, keys])

    def get_buffer(self):
        result = subprocess.run([
            "ovim", "buffer", self.session
        ], capture_output=True, text=True)
        return result.stdout

    def get_cursor(self):
        result = self.mcp("resources/read", {"uri": "ovim://snapshot"})
        snapshot = json.loads(result["result"]["contents"][0]["text"])
        return snapshot["cursor"]

    def kill(self):
        subprocess.run(["ovim", "kill", self.session])

# AI Code Review
def ai_code_review(file_path):
    session = OvimSession("review", file_path)

    # Get file content
    code = session.get_buffer()
    lines = code.split('\n')

    issues = []

    # Navigate through each line
    session.send_keys("gg")  # Go to top

    for line_num in range(len(lines)):
        # Move to line
        if line_num > 0:
            session.send_keys("j")

        # Get LSP hover info (if available)
        try:
            hover_result = session.mcp("tools/call", {
                "name": "lsp_hover",
                "arguments": {}
            })

            # AI analyzes hover info
            hover_text = hover_result.get("result", {}).get("content", [])
            if hover_text:
                ai_analysis = analyze_with_ai(lines[line_num], hover_text)
                if ai_analysis["has_issue"]:
                    issues.append({
                        "line": line_num + 1,
                        "code": lines[line_num],
                        "issue": ai_analysis["description"]
                    })
        except:
            pass  # No hover info available

    session.kill()
    return issues

def analyze_with_ai(code_line, hover_info):
    # AI processing here (GPT-4, Claude, etc.)
    # Returns: {"has_issue": bool, "description": str}
    pass

# Run review
if __name__ == "__main__":
    issues = ai_code_review("src/main.rs")
    for issue in issues:
        print(f"Line {issue['line']}: {issue['issue']}")
        print(f"  {issue['code']}")
```

## Workflow 3: Parallel Test Execution

**Scenario**: AI spawns sessions for each test file, runs them in parallel, collects results.

```bash
#!/bin/bash
# Parallel test runner

# Find all test files
test_files=$(find tests -name "*.rs")

# Spawn session for each
for file in $test_files; do
  session="test_$(basename "$file" .rs)"
  ovim --headless --session "$session" "$file" &
done

# Wait for LSP
sleep 3

# Run tests in parallel
pids=()
for file in $test_files; do
  session="test_$(basename "$file" .rs)"

  (
    # Execute tests
    ovim exec "$session" "!cargo test --test $(basename "$file" .rs)"

    # Capture exit code
    exit_code=$?

    # Get diagnostics if failed
    if [ $exit_code -ne 0 ]; then
      ovim lsp-status "$session" > "failures/$session.log"
    fi

    echo "$session: $exit_code"
  ) &

  pids+=($!)
done

# Wait for all
for pid in "${pids[@]}"; do
  wait $pid
done

# Collect results
echo "Test Results:"
ovim sessions | tail -n +3 | while read session pid port lsp file; do
  if [ -f "failures/$session.log" ]; then
    echo "  ✗ $session FAILED"
  else
    echo "  ✓ $session PASSED"
  fi
done

# Cleanup
ovim sessions | tail -n +3 | awk '{print $1}' | xargs -n1 ovim kill
```

## Workflow 4: Live Documentation Generation

**Scenario**: AI monitors code changes and updates documentation in real-time.

```python
#!/usr/bin/env python3
import subprocess
import json
import time
from watchdog.observers import Observer
from watchdog.events import FileSystemEventHandler

class DocumentationGenerator:
    def __init__(self):
        self.sessions = {}

    def start_session(self, file_path):
        session_name = f"doc_{file_path.replace('/', '_')}"

        subprocess.Popen([
            "ovim", file_path,
            "--headless",
            "--session", session_name
        ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

        time.sleep(2)
        self.sessions[file_path] = session_name
        return session_name

    def generate_docs(self, file_path):
        session = self.sessions.get(file_path)
        if not session:
            session = self.start_session(file_path)

        # Get current buffer
        result = subprocess.run([
            "ovim", "buffer", session
        ], capture_output=True, text=True)

        code = result.stdout

        # Extract functions with LSP
        functions = self.extract_functions_with_lsp(session)

        # Generate markdown docs
        docs = self.ai_generate_documentation(code, functions)

        # Write to docs file
        docs_file = file_path.replace("src/", "docs/").replace(".rs", ".md")
        with open(docs_file, 'w') as f:
            f.write(docs)

        print(f"Generated docs for {file_path}")

    def extract_functions_with_lsp(self, session):
        # Use LSP to find all public functions
        result = subprocess.run([
            "ovim", "mcp", session, "tools/call",
            json.dumps({
                "name": "send_keys",
                "arguments": {"keys": "gg"}
            })
        ], capture_output=True, text=True)

        # Navigate through and collect function signatures
        # (simplified - real implementation would use LSP symbols)
        functions = []

        # Get buffer and parse
        buffer_result = subprocess.run([
            "ovim", "buffer", session
        ], capture_output=True, text=True)

        for line in buffer_result.stdout.split('\n'):
            if 'pub fn ' in line:
                functions.append(line.strip())

        return functions

    def ai_generate_documentation(self, code, functions):
        # AI generates documentation (GPT-4, Claude, etc.)
        # Returns markdown string
        pass

    def cleanup(self):
        for session in self.sessions.values():
            subprocess.run(["ovim", "kill", session])

# Watch for changes and regenerate docs
class CodeChangeHandler(FileSystemEventHandler):
    def __init__(self, doc_gen):
        self.doc_gen = doc_gen

    def on_modified(self, event):
        if event.src_path.endswith('.rs'):
            self.doc_gen.generate_docs(event.src_path)

if __name__ == "__main__":
    doc_gen = DocumentationGenerator()

    observer = Observer()
    handler = CodeChangeHandler(doc_gen)
    observer.schedule(handler, "src/", recursive=True)
    observer.start()

    try:
        print("Watching for changes...")
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        observer.stop()
        doc_gen.cleanup()

    observer.join()
```

## Workflow 5: Autonomous Bug Fixing

**Scenario**: AI detects errors via LSP, proposes fixes, tests them, commits if successful.

```bash
#!/bin/bash
# Autonomous bug fixer

fix_bugs() {
  local file=$1
  local session="bugfix_$(basename "$file" .rs)"

  # Start session
  ovim --headless --session "$session" "$file" &
  sleep 2

  # Check for LSP diagnostics
  diagnostics=$(ovim lsp-status "$session" | grep -i "error" | wc -l)

  if [ "$diagnostics" -gt 0 ]; then
    echo "Found $diagnostics errors in $file"

    # Get buffer content
    current_code=$(ovim buffer "$session")

    # AI generates fix
    fixed_code=$(echo "$current_code" | ai_fix_errors)

    # Apply fix
    ovim mcp "$session" tools/call "{
      \"name\":\"set_buffer\",
      \"arguments\":{\"content\":\"$fixed_code\"}
    }"

    # Wait for LSP to process
    sleep 1

    # Check if errors are gone
    new_diagnostics=$(ovim lsp-status "$session" | grep -i "error" | wc -l)

    if [ "$new_diagnostics" -eq 0 ]; then
      echo "  ✓ Fix successful!"

      # Run tests
      ovim exec "$session" "!cargo test"

      if [ $? -eq 0 ]; then
        # Save and commit
        ovim exec "$session" "w"
        git add "$file"
        git commit -m "AI: Fix errors in $file"
        echo "  ✓ Committed fix"
      else
        echo "  ✗ Tests failed, reverting"
        git checkout "$file"
      fi
    else
      echo "  ✗ Fix did not resolve errors"
    fi
  fi

  ovim kill "$session"
}

# Process all Rust files
find src -name "*.rs" | while read file; do
  fix_bugs "$file"
done
```

## Workflow 6: Interactive Pair Programming

**Scenario**: Human and AI edit simultaneously with conflict resolution.

```python
#!/usr/bin/env python3
import subprocess
import json
import time
import threading

class PairProgrammingSession:
    def __init__(self, file_path):
        self.file = file_path
        self.session = "pair"
        self.last_known_state = None
        self.running = True

        # Start session
        subprocess.Popen([
            "ovim", file_path,
            "--headless",
            "--session", self.session
        ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(2)

        # Start monitoring thread
        self.monitor_thread = threading.Thread(target=self.monitor_changes)
        self.monitor_thread.start()

    def get_snapshot(self):
        result = subprocess.run([
            "ovim", "snapshot", self.session, "--format", "json"
        ], capture_output=True, text=True)
        return json.loads(result.stdout)

    def monitor_changes(self):
        """AI monitors what human is doing"""
        while self.running:
            snapshot = self.get_snapshot()

            if snapshot != self.last_known_state:
                # Detect what changed
                if self.last_known_state:
                    self.on_change(self.last_known_state, snapshot)

                self.last_known_state = snapshot

            time.sleep(0.5)

    def on_change(self, old, new):
        """React to human's changes"""
        cursor_moved = old["cursor"] != new["cursor"]
        buffer_changed = old["buffer"]["content"] != new["buffer"]["content"]

        if buffer_changed:
            # AI analyzes the change
            new_code = new["buffer"]["content"]
            suggestions = self.ai_suggest_improvements(new_code)

            if suggestions:
                print(f"AI: {suggestions}")
                # Could insert comment with suggestion

    def ai_suggest_improvements(self, code):
        # AI processing
        pass

    def ai_make_edit(self, edit_description):
        """AI makes an edit"""
        # Use MCP to apply AI's edit
        subprocess.run([
            "ovim", "mcp", self.session, "tools/call",
            json.dumps({
                "name": "send_keys",
                "arguments": {"keys": edit_description}
            })
        ])

    def stop(self):
        self.running = False
        self.monitor_thread.join()
        subprocess.run(["ovim", "kill", self.session])

# Usage
session = PairProgrammingSession("src/main.rs")

try:
    print("Pair programming session started. AI is watching...")
    while True:
        # AI can make suggestions or edits
        user_input = input("AI command: ")
        if user_input == "quit":
            break
        elif user_input.startswith("edit:"):
            session.ai_make_edit(user_input[5:])
except KeyboardInterrupt:
    pass

session.stop()
```

## Key Patterns

### 1. Spawn-Edit-Verify

```bash
ovim --headless --session NAME FILE &
sleep 2  # Wait for LSP
ovim send NAME "edits..."
ovim health NAME | grep "ready"
ovim kill NAME
```

### 2. MCP Query-Response

```bash
ovim mcp SESSION METHOD PARAMS | jq '.result'
```

### 3. Parallel Execution

```bash
for file in FILES; do
  ovim --headless --session $(basename $file) $file &
done
# ... operations ...
ovim sessions | awk '{print $1}' | xargs -n1 ovim kill
```

### 4. State Monitoring

```bash
while true; do
  SNAPSHOT=$(ovim snapshot SESSION)
  # Process snapshot
  sleep 1
done
```

## Best Practices

1. **Wait for LSP**: Always `sleep 2` after spawning sessions
2. **Check health**: Verify LSP is ready before critical operations
3. **Cleanup**: Always kill sessions when done
4. **Error handling**: Check exit codes and LSP diagnostics
5. **Idempotency**: Use MCP for state queries, not just commands
6. **Parallel**: Leverage multiple sessions for speed
7. **Atomic**: Use MCP `set_buffer` for large changes

## Integration with AI Frameworks

### LangChain

```python
from langchain.tools import tool

@tool
def ovim_edit_file(file_path: str, changes: str) -> str:
    """Edit a file using ovim with LSP verification"""
    session = f"langchain_{file_path.replace('/', '_')}"

    # Spawn session
    subprocess.Popen([...])

    # Apply changes via MCP
    result = subprocess.run([
        "ovim", "mcp", session, "tools/call",
        json.dumps({"name": "set_buffer", "arguments": {"content": changes}})
    ])

    # Verify with LSP
    health = subprocess.run(["ovim", "health", session], ...)

    # Cleanup
    subprocess.run(["ovim", "kill", session])

    return result.stdout
```

### AutoGPT

```python
class OvimCommand(Command):
    def execute(self, session, operation, *args):
        if operation == "edit":
            return subprocess.run([
                "ovim", "send", session, args[0]
            ]).stdout
        elif operation == "query":
            return subprocess.run([
                "ovim", "mcp", session, args[0], args[1]
            ]).stdout
```

## Conclusion

ovim's MCP-native architecture and integrated CLI make it the ideal editor for AI workflows:

- **No external dependencies**: Everything built-in
- **Scriptable**: All operations via CLI
- **Observable**: Query any state at any time
- **Parallel**: Multiple sessions, coordinated edits
- **Verified**: LSP integration for correctness

This enables AI agents to edit code with the same power and precision as human developers.
