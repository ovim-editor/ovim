# ovim - A Neovim Clone in Rust

A terminal-based text editor that reimplements core Vim/Neovim functionality in Rust with an optional REST API for remote control and automation.

## Features

### Editor Features
- ✅ **Modal editing** (Normal, Insert, Visual, Command modes)
- ✅ **Navigation** (hjkl, gg, G, 0, $, w, b, e, ^)
- ✅ **Find character motions** (f, F, t, T, ;, , for quick line navigation)
- ✅ **Bracket matching** (% to jump to matching bracket/paren/brace)
- ✅ **Operators + Motions** (d, c, y combined with motions)
- ✅ **Count prefixes** (5j, 3dd, 2fo, etc.)
- ✅ **Visual selection** (character and line modes)
- ✅ **Undo/Redo** (u, Ctrl-R)
- ✅ **Repeat command** (. to repeat last change)
- ✅ **Yank/Paste** (yy, dd, p, P with register support)
- ✅ **Search** (/, ?, n, N with regex support)
- ✅ **Insert modes** (i, I, a, A, o, O)
- ✅ **Text objects** (w, W, p, sentence)
- ✅ **Marks** (ma, 'a, `a with jump list)
- ✅ **Macros** (qa, @a for recording and playback)
- ✅ **Ex commands** (:w, :q, :wq, etc.)

### REST API Features
- ✅ **Remote control** via HTTP endpoints
- ✅ **Full state introspection** (buffer, cursor, mode, registers, marks)
- ✅ **Key injection** (send vim commands programmatically)
- ✅ **Command execution** (execute ex commands)
- ✅ **Automated testing** support
- ✅ **Dynamic port allocation** (no conflicts)

## Installation

```bash
git clone <repo-url>
cd ovim
cargo build --release
```

## Usage

### Basic Editor

```bash
# Open a file
cargo run -- myfile.txt

# Start with empty buffer
cargo run
```

### With REST API

```bash
# Open with API enabled (random port)
cargo run -- myfile.txt --expose-rest-api

# The API URL will be printed:
# REST API server listening on http://127.0.0.1:56789
# API URL: http://127.0.0.1:56789
```

### With Custom Dimensions

```bash
# Useful for testing, screenshots, or headless environments
cargo run -- myfile.txt --dimension=80x24

# Combine options
cargo run -- myfile.txt --expose-rest-api --dimension=120x40
```

## REST API

### Available Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/snapshot` | GET | Get complete editor state |
| `/buffer` | GET | Get buffer content |
| `/buffer` | PUT | Replace buffer content |
| `/cursor` | GET | Get cursor position |
| `/mode` | GET | Get current mode |
| `/keys` | POST | Send keystrokes |
| `/command` | POST | Execute ex command |

### Example Usage

Start the editor with API:
```bash
cargo run -- test.txt --expose-rest-api
# Note the port number from output
```

In another terminal:
```bash
export API_URL="http://127.0.0.1:PORT"  # Replace PORT

# Get editor state
curl $API_URL/snapshot | jq '.'

# Set buffer content
curl -X PUT $API_URL/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Hello, World!\nLine 2\nLine 3"}'

# Navigate to top
curl -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "gg"}'

# Enter insert mode and type
curl -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "iPREFIX: <Esc>"}'

# Delete a line
curl -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "dd"}'

# Save file
curl -X POST $API_URL/command \
  -H "Content-Type: application/json" \
  -d '{"command": "w"}'
```

## Testing

### Manual Testing

1. Start ovim with API:
   ```bash
   cargo run -- test.txt --expose-rest-api
   ```

2. Note the port number, then run the test script:
   ```bash
   ./manual_test.sh 56789  # Replace with your port
   ```

### Automated Testing

See `TESTING.md` for comprehensive test scenarios.

### Integration Tests

```bash
# Start ovim in one terminal
cargo run -- test.txt --expose-rest-api

# In another terminal
cargo test --test api_test -- --ignored
```

## Architecture

```
ovim/
├── src/
│   ├── api/           # REST API server
│   │   ├── mod.rs     # Server initialization
│   │   ├── routes.rs  # Route definitions
│   │   ├── handlers.rs # Request handlers
│   │   └── state.rs   # API types and state
│   ├── buffer/        # Text buffer (rope-based)
│   ├── editor/        # Core editor logic
│   │   ├── input.rs   # Key event handling
│   │   ├── operators.rs # Vim operators (d, c, y)
│   │   ├── motions.rs  # Cursor motions
│   │   ├── change.rs   # Undo/redo system
│   │   ├── register.rs # Yank/paste registers
│   │   ├── search.rs   # Search functionality
│   │   ├── marks.rs    # Marks and jump list
│   │   └── macros.rs   # Macro recording
│   ├── ui/            # Terminal UI (ratatui)
│   ├── mode/          # Editor modes
│   └── cli.rs         # CLI argument parsing
```

### Communication Flow

```
┌─────────────┐     HTTP      ┌──────────────┐
│   REST API  │◄──────────────┤  API Client  │
│   (Axum)    │               └──────────────┘
└──────┬──────┘
       │ mpsc channel (tokio)
       │
       ▼
┌─────────────┐               ┌──────────────┐
│ Main Event  │◄──────────────┤  Terminal    │
│    Loop     │   Key Events  │   Input      │
└──────┬──────┘               └──────────────┘
       │
       ▼
┌─────────────┐               ┌──────────────┐
│   Editor    │◄─────────────►│    Buffer    │
│    State    │               │    (Rope)    │
└─────────────┘               └──────────────┘
```

- API requests are sent via tokio channels
- Main event loop processes both API requests and keyboard input
- All state mutations happen on the main thread (thread-safe)
- API responses include updated state after operations

## Dependencies

- **ropey** - Efficient rope data structure for text editing
- **ratatui** - Terminal UI framework
- **crossterm** - Cross-platform terminal manipulation
- **axum** - Web framework for REST API
- **tokio** - Async runtime
- **serde** - Serialization/deserialization
- **clap** - Command-line argument parsing
- **regex** - Regular expression support

## Use Cases

### Automated Testing
```bash
# Test script can control editor programmatically
./run_tests.sh
```

### IDE Integration
Build tools or plugins that interact with ovim via HTTP instead of complex IPC.

### Remote Editing
Control an ovim instance from another process or machine.

### Debugging
Inspect editor state without interrupting the editing session.

### CI/CD
Automate editor operations in headless environments with `--dimension` flag.

## Vim Compatibility

ovim implements a subset of Vim commands. Notable differences:
- Limited ex command support (`:w`, `:q`, `:wq` primarily)
- Simplified visual block mode
- No plugin system (yet)
- No split windows (yet)

## Development

### Running Tests
```bash
cargo test
```

### Building Release
```bash
cargo build --release
./target/release/ovim myfile.txt
```

### Code Style
```bash
cargo fmt
cargo clippy
```

## Documentation

- `CLAUDE.md` - Project overview and architecture
- `TESTING.md` - Comprehensive testing guide
- `manual_test.sh` - Quick manual API test script
- `test_api.sh` - Detailed API test scenarios
- `run_tests.sh` - Automated test runner

## License

[Add license information]

## Contributing

[Add contribution guidelines]

## Roadmap

- [ ] Visual block mode
- [ ] Split windows
- [ ] Plugin system
- [ ] More ex commands
- [ ] Configuration file
- [ ] Syntax highlighting
- [ ] LSP integration
- [ ] Extended API endpoints (tabs, windows)
