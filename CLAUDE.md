# ovim

A Neovim clone written in Rust, featuring a terminal-based UI and comprehensive Vim emulation.

## Project Overview

ovim is a text editor that reimplements core Vim/Neovim functionality in Rust. It provides a familiar modal editing experience with support for:

- **Modal Editing**: Normal, Insert, Visual, and Command modes
- **Text Operations**: Full operator-motion model (d, c, y with motions)
- **Registers**: Named registers for yanking and pasting
- **Marks**: Buffer marks and jump lists for navigation
- **Undo/Redo**: Full undo tree with change tracking
- **Macros**: Record and replay keystroke sequences
- **Search**: Forward and backward search with regex support
- **Text Objects**: Support for w, W, p, sentence, and other text objects
- **Visual Mode**: Character and line-based visual selection

## Architecture

The codebase is organized into several modules:

- **buffer**: Text buffer implementation using the ropey rope data structure
- **editor**: Core editor logic, input handling, and state management
- **ui**: Terminal UI rendering using ratatui and crossterm
- **mode**: Editor mode definitions (Normal, Insert, Visual, Command)
- **api**: REST API server for remote control and introspection (optional)

## Building and Running

```bash
# Build the project
cargo build --release

# Run with a file
cargo run -- myfile.txt

# Run in headless mode with REST API (no TUI, uses random available port)
cargo run -- myfile.txt --headless
# Output will show: API URL: http://127.0.0.1:<PORT>

# Run with custom viewport dimensions
cargo run -- myfile.txt --dimension=80x24

# Run tests
cargo test
```

## Configuration

ovim supports Lua-based configuration similar to Neovim. Configuration files are loaded from:

1. `$OVIM_CONFIG/init.lua`
2. `$XDG_CONFIG_HOME/ovim/init.lua`
3. `~/.config/ovim/init.lua`
4. `~/.ovim/init.lua`

### Example Configuration

```lua
-- Set options using vim.opt (Neovim-style)
vim.opt.number = true              -- Show line numbers
vim.opt.relativenumber = true      -- Show relative line numbers
vim.opt.expandtab = true           -- Use spaces instead of tabs
vim.opt.tabstop = 4                -- Tab width
vim.opt.shiftwidth = 4             -- Indent width
vim.opt.scroll = 10                -- Half-page scroll amount

-- Or use vim.cmd to execute ex commands
vim.cmd('set number')
vim.cmd('colorscheme tokyonight')

-- Use vim.api functions
vim.api.nvim_command('set tabstop=4')

-- Print messages
print("Configuration loaded!")
```

### Available Options

- `number` / `nu` - Show line numbers
- `relativenumber` / `rnu` - Show relative line numbers
- `expandtab` / `et` - Use spaces instead of tabs
- `tabstop` / `ts` - Tab width (1-16)
- `shiftwidth` / `sw` - Indent width (1-16)
- `scroll` - Half-page scroll amount

### Reloading Configuration

To reload your configuration without restarting ovim:
```
:ConfigReload
```

or

```
:reload
```

See `example_init.lua` for a complete example.

## Java Development (Zero Config)

ovim has **supersmooth Java support** with zero configuration:

```bash
# Just open any Java file - that's it!
ovim MyJavaFile.java

# ovim automatically:
# ✓ Downloads jdtls (one-time)
# ✓ Detects Java version from build.gradle/pom.xml
# ✓ Finds correct JVM (17, 21, etc.)
# ✓ Gives you full IDE features in seconds
```

**Features:**
- Auto-installs Eclipse JDT.LS
- Detects Java 8, 11, 17, 21, 24 from build files
- Works with Maven (pom.xml) and Gradle (build.gradle/build.gradle.kts)
- Full LSP: completion, go-to-definition, hover, diagnostics, refactoring
- Fully async and non-blocking

**See [ZERO_CONFIG_JAVA.md](ZERO_CONFIG_JAVA.md) for details.**

## Testing the REST API

1. **Start ovim in headless mode:**
   ```bash
   cargo run -- test.txt --headless
   ```
   Note the port number from the output (e.g., `http://127.0.0.1:56789`)

2. **Run manual tests in another terminal:**
   ```bash
   ./manual_test.sh 56789  # Replace with your actual port
   ```

3. **Or test individual endpoints:**
   ```bash
   export API_URL="http://127.0.0.1:56789"

   # Get current state
   curl $API_URL/snapshot | jq '.'

   # Send keys
   curl -X POST $API_URL/keys \
     -H "Content-Type: application/json" \
     -d '{"keys": "gg"}'

   # Check cursor
   curl $API_URL/cursor
   ```

### Command-Line Options

- **`--headless`**: Run in headless mode with REST API enabled (no TUI). The API server runs on a dynamically allocated port (output shows: `API URL: http://127.0.0.1:<PORT>`)
- **`--dimension=WIDTHxHEIGHT`**: Set the viewport dimensions (e.g., `80x24` for 80 columns by 24 rows). Useful for:
  - Automated testing with consistent dimensions
  - Taking screenshots at specific sizes
  - Debugging rendering issues

## REST API

When started with the `--headless` flag, ovim runs in headless mode without a TUI and exposes a REST API on a dynamically allocated port that allows external tools to control and introspect the editor remotely.

### Use Cases

- Automated testing and scripting
- Integration with external tools and plugins
- Remote debugging and inspection
- Building alternative frontends or bridges

### Endpoints

#### `GET /snapshot`

Get a complete snapshot of the editor state.

**Response:**
```json
{
  "buffer": {
    "content": "file contents...",
    "line_count": 42,
    "file_path": "/path/to/file.txt"
  },
  "cursor": {
    "line": 0,
    "column": 0
  },
  "mode": "Normal",
  "visual_selection": null,
  "registers": {
    "\"": "last yanked text",
    "0": "last yanked text"
  },
  "marks": {
    "a": {"line": 5, "column": 10}
  }
}
```

#### `POST /keys`

Send key events to the editor as if they were typed.

**Request:**
```json
{
  "keys": "dd"
}
```

**Response:**
```json
{
  "success": true
}
```

#### `GET /buffer`

Get the current buffer content.

**Response:**
```json
{
  "content": "file contents...",
  "line_count": 42,
  "file_path": "/path/to/file.txt"
}
```

#### `PUT /buffer`

Replace the entire buffer content.

**Request:**
```json
{
  "content": "new file contents..."
}
```

**Response:**
```json
{
  "success": true,
  "line_count": 3
}
```

#### `GET /cursor`

Get the current cursor position.

**Response:**
```json
{
  "line": 0,
  "column": 0
}
```

#### `GET /mode`

Get the current editor mode.

**Response:**
```json
{
  "mode": "Normal"
}
```

#### `POST /command`

Execute an ex command (e.g., `:w`, `:q`).

**Request:**
```json
{
  "command": "w"
}
```

**Response:**
```json
{
  "success": true,
  "output": "\"myfile.txt\" 42L, 1024C written"
}
```

### API Architecture

The REST API runs in a separate thread alongside the main editor event loop. Communication between the API server and the editor is handled through thread-safe channels:

- API requests are queued and processed synchronously with editor state updates
- The editor processes API requests between normal key events
- Responses include the updated state after the operation completes

This design ensures thread safety and consistency without requiring complex locking mechanisms.

### Example Usage

Start ovim in headless mode:
```bash
cargo run -- myfile.txt --headless
# Note the port from output: API URL: http://127.0.0.1:56789
```

In a separate terminal, test the API (replace 56789 with your actual port):

```bash
# Get editor snapshot
curl http://127.0.0.1:56789/snapshot

# Send keys to delete a line
curl -X POST http://127.0.0.1:56789/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "dd"}'

# Get buffer content
curl http://127.0.0.1:56789/buffer

# Set buffer content
curl -X PUT http://127.0.0.1:56789/buffer \
  -H "Content-Type: application/json" \
  -d '{"content": "Hello, World!\nThis is a test."}'

# Get cursor position
curl http://127.0.0.1:56789/cursor

# Get current mode
curl http://127.0.0.1:56789/mode

# Execute a save command
curl -X POST http://127.0.0.1:56789/command \
  -H "Content-Type: application/json" \
  -d '{"command": "w"}'

# Complex example: navigate and edit
curl -X POST http://127.0.0.1:56789/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "gg"}' # Go to first line

curl -X POST http://127.0.0.1:56789/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "iHello "}' # Insert "Hello "

curl -X POST http://127.0.0.1:56789/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "<Esc>"}' # Exit insert mode
```

Prefer using the CLI instead!

```
❯ ./send-cmd
Usage:
  ./send-cmd <port> keys <keys>
  ./send-cmd <port> buffer <content>
  ./send-cmd <port> get <endpoint>
  ./send-cmd <port> command <cmd>

Examples:
  ./send-cmd 56789 keys "gg"
  ./send-cmd 56789 keys "iHello<Esc>"
  ./send-cmd 56789 buffer "Line 1
Line 2"
  ./send-cmd 56789 get buffer
  ./send-cmd 56789 get cursor
  ./send-cmd 56789 get mode
  ./send-cmd 56789 get snapshot
  ./send-cmd 56789 command "w"
```

## Development

### Dependencies

- **ropey**: Rope data structure for efficient text editing
- **ratatui**: Terminal UI framework
- **crossterm**: Cross-platform terminal manipulation
- **regex**: Regular expression support
- **axum**: Web framework for REST API
- **tokio**: Async runtime
- **serde**: Serialization/deserialization
- **clap**: Command-line argument parsing

### Testing

```bash
# Run unit tests
cargo test

# Run with verbose output
cargo test -- --nocapture

# Run benchmarks
cargo bench
```

## License

[License information to be added]
