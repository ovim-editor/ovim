# LSP Usage Guide

## Overview

ovim has full LSP (Language Server Protocol) support for Rust, JavaScript/TypeScript, and Python. LSP provides IDE-like features such as goto-definition, hover information, and diagnostics.

## Supported Language Servers

- **Rust**: `rust-analyzer`
- **JavaScript/TypeScript**: `typescript-language-server`
- **Python**: `pylsp` (Python Language Server)

## Prerequisites

Make sure the language server for your language is installed and available in your PATH:

```bash
# Rust
cargo install rust-analyzer

# JavaScript/TypeScript
npm install -g typescript-language-server

# Python
pip install python-lsp-server
```

## LSP Features

### Goto Definition (`gd`)

Jump to the definition of the symbol under the cursor.

**Important**: The cursor must be positioned **exactly on the identifier** for `gd` to work.

**Example**:
```rust
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    let result = add(3, 4);
    //           ^ cursor here
}
```

To use goto-definition:
1. Position cursor on the identifier (e.g., 'add')
   - Use `fa` to find and position on 'a' in 'add'
   - Or use `w` to jump to the word
2. Press `gd`
3. Cursor jumps to the function definition

**Troubleshooting**:
- If `gd` doesn't work, check cursor position with `:echo col('.')`
- The cursor must be on a letter of the identifier, not on whitespace
- Use `f` + letter to precisely position on identifiers

### Hover Information (`K`)

Show type information and documentation for the symbol under the cursor.

1. Position cursor on identifier
2. Press `K` (Shift+k)
3. Hover information appears (if available)

### Diagnostics

LSP servers provide real-time diagnostics (errors, warnings, hints) as you edit.

- Diagnostics are automatically fetched from the language server
- View diagnostic counts with `:LspInfo`
- Diagnostics update after file changes and saves

### LSP Info (`:LspInfo`)

Check LSP status and active servers:

```vim
:LspInfo
```

Shows:
- Active language servers
- Diagnostic counts (errors, warnings, info, hints)
- Current file path
- LSP status

## Usage Tips

### Positioning for `gd`

The most common issue is cursor positioning. Here are reliable ways to position on an identifier:

1. **Find character**: `f` + first letter
   ```
   let result = add(3, 4);
   Press: fa    (finds 'a' in 'add')
   ```

2. **Word motions**: Use `w` repeatedly or with count
   ```
   let result = add(3, 4);
   From line start: 3w  (jumps to 'add')
   ```

3. **Search**: `/add<Enter>` to search and position

### Verifying Cursor Position

Use the REST API to check exact cursor position:

```bash
# Start ovim in headless mode
ovim myfile.rs --headless

# Check cursor position
curl http://localhost:<PORT>/cursor
```

## API Testing

Test LSP features via the REST API:

```bash
# Start headless mode
ovim test.rs --headless
# Note the port from output

export API_URL="http://127.0.0.1:<PORT>"

# Position cursor (line 5, find 'a' in identifier)
curl -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "5jfa"}'

# Verify position
curl $API_URL/cursor

# Execute goto-definition
curl -X POST $API_URL/keys \
  -H "Content-Type: application/json" \
  -d '{"keys": "gd"}'

# Check new cursor position
curl $API_URL/cursor
```

## Limitations

1. **Same-file only**: Currently only supports goto-definition within the same file
2. **Exact positioning required**: Cursor must be precisely on the identifier
3. **File must be saved**: Some LSP features require the file to be saved to disk

## Troubleshooting

### LSP not working

1. **Check server is installed**:
   ```bash
   which rust-analyzer  # or other server
   ```

2. **Check LSP status**:
   ```vim
   :LspInfo
   ```

3. **Verify file extension**: LSP only activates for .rs, .js, .ts, .py files

### `gd` does nothing

1. **Cursor not on identifier**: Most common issue
   - Try `fa` to find the first letter
   - Verify position with API if in headless mode

2. **LSP not initialized**:
   - Wait a moment after opening file
   - Check `:LspInfo` for active servers

3. **File not in project**:
   - Rust files should be in a Cargo project (Cargo.toml in parent dir)
   - Other languages should have proper project structure

### Server crashes

Check ovim's stderr output for LSP errors:
```bash
ovim myfile.rs 2> lsp_errors.log
```

## Advanced: didChange and didSave Notifications

The LSP implementation sends proper notifications to language servers:

- **didChange**: Sent when buffer is modified (after exiting insert mode)
- **didSave**: Sent when file is saved (`:w`, `:wq`, etc.)
- **Full document sync**: Sends complete file content (not incremental diffs)

These notifications keep the language server in sync with your edits, enabling accurate diagnostics and IDE features.
