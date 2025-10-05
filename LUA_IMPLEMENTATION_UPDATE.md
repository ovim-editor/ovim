# Lua Support Implementation Update

## Current Status: Phase 2 Complete ✓

The EditorBridge for Lua-Editor communication has been successfully implemented and wired into the editor.

## Completed Work

### Phase 1: Foundation (✓ Complete)
- ✓ Added mlua dependency with luajit, async, and serialize features
- ✓ Created `src/lua/mod.rs` with LuaContext implementation
- ✓ Created `src/lua/api.rs` with vim namespace (placeholder implementations)
- ✓ Created `src/lua/util.rs` for type conversion
- ✓ Created `src/config/mod.rs` for configuration and plugin loading
- ✓ Made Lua support **optional** via `lua` feature flag
- ✓ Added conditional compilation with `#[cfg(feature = "lua")]`

### Phase 2: EditorBridge Implementation (✓ Complete)
- ✓ Created `src/lua/editor_bridge.rs` - Thread-safe bridge for Lua-Editor communication
- ✓ Updated `vim.api` functions to use EditorBridge:
  - `vim.api.nvim_command(cmd)` - Queue commands for execution
  - `vim.api.nvim_exec(src, output)` - Execute multiple commands
  - `vim.api.nvim_get_current_line()` - Get current line from bridge cache
- ✓ Updated `vim.fn` functions to use EditorBridge:
  - `vim.fn.line('.')` - Get current line number (1-indexed)
  - `vim.fn.line('$')` - Get last line number
  - `vim.fn.col('.')` - Get current column (1-indexed)
- ✓ Updated `vim.cmd(command)` to use EditorBridge
- ✓ Added EditorBridge field to Editor struct
- ✓ Updated `enable_lua()` to create and initialize EditorBridge
- ✓ Added `sync_lua_bridge()` to sync editor state to bridge cache
- ✓ Added `get_lua_commands()` to retrieve pending commands from Lua
- ✓ Added `update_lua_state()` to update bridge after state changes
- ✓ Updated `execute_lua()` to sync state before execution

## Architecture

### EditorBridge Design

```rust
pub struct EditorBridge {
    inner: Arc<Mutex<EditorBridgeInner>>,
}

struct EditorBridgeInner {
    pending_commands: Vec<String>,      // Commands from Lua to execute
    cursor_pos: Option<(usize, usize)>, // Cached cursor position
    buffer_content: Option<String>,      // Cached buffer content
    mode: Option<String>,                // Cached mode
}
```

The bridge provides:
- **Thread-safe access** via Arc<Mutex<>>
- **Command queuing** from Lua to editor
- **State caching** from editor to Lua
- **Clone-able** for use in Lua closures

### Data Flow

```
Editor State → sync_lua_bridge() → EditorBridge Cache
                                         ↓
                                    Lua Functions
                                         ↓
                                  execute_command()
                                         ↓
                                  pending_commands
                                         ↓
                                  get_lua_commands()
                                         ↓
                                    Main Event Loop
                                         ↓
                                   execute_command()
```

## Current API Support

### vim.api (Basic)
- ✓ `vim.api.nvim_command(cmd)` - Queues command for execution
- ✓ `vim.api.nvim_exec(src, output)` - Executes multiple lines
- ✓ `vim.api.nvim_get_current_line()` - Returns current line text

### vim.fn (Basic)
- ✓ `vim.fn.line('.')` - Current line number (1-indexed)
- ✓ `vim.fn.line('$')` - Last line number
- ✓ `vim.fn.col('.')` - Current column (1-indexed)

### vim.cmd (Basic)
- ✓ `vim.cmd(command)` - Execute ex command

### vim.g (Placeholder)
- Empty table for global variables (not yet functional)

### vim.opt (Placeholder)
- Empty table for options (not yet functional)

## Building with Lua

```bash
# Build without Lua (default)
cargo build

# Build with Lua support
cargo build --features lua

# Run with Lua support
cargo run --features lua -- myfile.txt
```

## Example Lua Configuration

```lua
-- ~/.config/ovim/init.lua
print("ovim loaded!")

-- Get current position
local line = vim.fn.line('.')
local col = vim.fn.col('.')
print("Cursor at line " .. line .. ", column " .. col)

-- Queue a command
vim.cmd("echo 'Hello from Lua!'")

-- Get current line
local current_line = vim.api.nvim_get_current_line()
print("Current line: " .. current_line)
```

## Next Steps

### Phase 3: Command Processing Integration
1. Integrate `get_lua_commands()` into main event loop
2. Process pending commands from Lua API calls
3. Test command execution flow

### Phase 4: Expand vim.api
- `nvim_buf_get_lines()` - Get range of lines
- `nvim_buf_set_lines()` - Set range of lines
- `nvim_win_get_cursor()` - Get cursor position
- `nvim_win_set_cursor()` - Set cursor position
- `nvim_get_mode()` - Get current mode

### Phase 5: Expand vim.fn
- `expand()` - Expand special variables
- `bufnr()` - Get buffer number
- `bufname()` - Get buffer name
- `getline()` - Get line(s) from buffer

### Phase 6: vim.keymap
- `vim.keymap.set(mode, lhs, rhs, opts)` - Set key mapping
- `vim.keymap.del(mode, lhs)` - Delete key mapping

### Phase 7: LSP Integration
- `vim.lsp.buf.definition()` - Go to definition
- `vim.lsp.buf.hover()` - Show hover information
- `vim.lsp.buf.format()` - Format buffer

### Phase 8: Autocommands
- Event system for buffer/mode changes
- `vim.api.nvim_create_autocmd()`
- `vim.api.nvim_create_augroup()`

## Technical Notes

### Feature Flag Strategy
All Lua-related code is gated behind `#[cfg(feature = "lua")]` to allow:
- Building without Lua dependencies (default)
- Opt-in Lua support when needed
- Cleaner compile errors in environments without Lua

### Bridge vs Direct Access
The EditorBridge provides indirect access to editor state:
- **Pros**: Thread-safe, works with Lua closures, simple API
- **Cons**: State may be stale, requires manual syncing
- **Solution**: Sync before Lua execution and command retrieval

### Performance Considerations
- State syncing (cursor, buffer, mode) happens on demand
- Buffer content is cloned into bridge cache (could be optimized)
- Commands are queued and batched for execution
- No blocking calls from Lua to editor

## Testing Strategy

1. **Unit Tests** - Test EditorBridge methods in isolation
2. **Integration Tests** - Test vim.api calls with mock editor
3. **End-to-End Tests** - Test full Lua config loading and execution
4. **Manual Testing** - Create sample init.lua files and plugins

## Known Limitations

1. **State Staleness**: Bridge cache is manually synced, may be outdated
2. **Command Execution**: Commands are queued, not executed immediately
3. **Error Handling**: Limited error propagation from Lua to editor
4. **API Coverage**: Only basic vim.api/vim.fn functions implemented
5. **No Async**: No support for async Lua operations yet

## Files Modified

### New Files
- `src/lua/editor_bridge.rs` - EditorBridge implementation
- `src/lua/mod.rs` - LuaContext and module exports
- `src/lua/api.rs` - vim namespace API
- `src/lua/util.rs` - Type conversion utilities
- `src/config/mod.rs` - Configuration and plugin loading

### Modified Files
- `Cargo.toml` - Added mlua dependency and lua feature
- `src/lib.rs` - Conditional lua module export
- `src/editor/mod.rs` - Added EditorBridge integration
- `src/main.rs` - Lua initialization and :lua/:luafile commands

## Conclusion

The foundation for Lua support is now in place with a working EditorBridge that safely connects Lua scripts to editor state and operations. The next priority is integrating command execution into the main event loop and expanding the vim API coverage.
