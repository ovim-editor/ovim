# Language Support in ovim

**ovim** provides syntax highlighting and Language Server Protocol (LSP) support for multiple programming languages through a declarative configuration system.

## Supported Languages

### Out-of-the-Box Support

The following languages have both syntax highlighting and LSP configured:

| Language | Extensions | LSP Server | Auto-Install |
|----------|------------|------------|--------------|
| Rust | `.rs` | rust-analyzer | ✗ (install via rustup) |
| TypeScript | `.ts`, `.tsx`, `.mts`, `.cts` | typescript-language-server | ✓ (npm) |
| JavaScript | `.js`, `.jsx`, `.mjs`, `.cjs` | typescript-language-server | ✗ |
| Python | `.py`, `.pyw`, `.pyi` | pyright-langserver | ✗ |
| Java | `.java` | hyperion-lsp | ✓ (auto-download) |

### Syntax Highlighting Only

These languages have syntax highlighting but no LSP configured by default:

- Markdown (`.md`, `.markdown`)
- JSON (`.json`, `.jsonc`)
- YAML (`.yaml`, `.yml`)
- HTML (`.html`, `.htm`)
- CSS (`.css`, `.scss`, `.sass`)
- Go (`.go`)
- C (`.c`, `.h`)
- C++ (`.cpp`, `.hpp`)
- Ruby (`.rb`)
- Bash (`.sh`, `.bash`)

## Installing LSP Servers

### TypeScript/JavaScript

TypeScript LSP supports **auto-installation**! When you open a `.ts` or `.tsx` file:

1. If `typescript-language-server` is not found, ovim will offer to install it
2. The installation happens via `npm install -g typescript-language-server typescript`
3. Once installed, LSP features work immediately

**Manual installation** (if you prefer):
```bash
npm install -g typescript-language-server typescript
```

### Rust

```bash
rustup component add rust-analyzer
```

### Python

```bash
# Option 1: pyright (recommended)
pip install pyright

# Option 2: python-lsp-server
pip install python-lsp-server
```

### Java

Java has automatic download of the Hyperion LSP server. No manual installation required!

## Checking LSP Status

### List All Languages

```bash
ovim lsp languages
```

Output:
```
ID              Name                 LSP
--------------------------------------------------
rust            Rust                 Configured
typescript      TypeScript           Configured
javascript      JavaScript           Configured
...
```

Add `--verbose` for detailed configuration.

### Check Specific File

```bash
ovim lsp check src/main.rs
```

Output:
```
✓ Language Detected: Rust (rust)
✓ Syntax Highlighting: tree-sitter-rust grammar
✓ LSP Configuration:
  Primary Command: rust-analyzer
✓ LSP Server Found: rust-analyzer
✓ Project Root: /path/to/project
```

This command shows:
- Which language was detected
- Whether syntax highlighting is available
- Whether LSP is configured
- Whether the LSP server is actually installed
- What the project root is (for LSP initialization)

## Customizing Language Support

You can override or extend language support by creating:

```
~/.config/ovim/languages.toml
```

### Adding LSP to an Existing Language

Example: Add LSP support for Go:

```toml
[[language]]
id = "go"
name = "Go"
extensions = ["go"]

[language.syntax]
grammar = "tree-sitter-go"
official = { crate = "tree_sitter_go", constant = "HIGHLIGHTS_QUERY" }

[language.lsp]
command = "gopls"
root_markers = ["go.mod", "go.sum", ".git"]
install_hint = "Install with: go install golang.org/x/tools/gopls@latest"
```

### Adding a New Language

Example: Add Zig support:

```toml
[[language]]
id = "zig"
name = "Zig"
extensions = ["zig"]

[language.lsp]
command = "zls"
root_markers = ["build.zig", ".git"]
install_hint = "Install from: https://install.zigtools.org/"
```

### Configuration Options

#### Language Definition

- `id` - Unique identifier (lowercase, no spaces)
- `name` - Human-readable name
- `extensions` - File extensions (without the dot)
- `filenames` - Exact filenames (e.g., "Makefile", "Dockerfile")

#### Syntax Highlighting

```toml
[language.syntax]
grammar = "tree-sitter-rust"  # Tree-sitter grammar crate name
official = { crate = "tree_sitter_rust", constant = "HIGHLIGHTS_QUERY" }
```

Or for custom queries:
```toml
[language.syntax]
grammar = "tree-sitter-markdown"
file = { path = "src/syntax/queries/markdown.scm" }
```

#### LSP Configuration

```toml
[language.lsp]
# Primary command to try (searched in PATH)
command = "rust-analyzer"

# Command-line arguments
args = ["--stdio"]

# Fallback locations if command not in PATH
fallback_commands = [
    "node_modules/.bin/typescript-language-server",
    "~/.npm-global/bin/typescript-language-server"
]

# Files/directories that mark project root (searched upward from file)
root_markers = ["Cargo.toml", "Cargo.lock", ".git"]

# Help message when LSP not found
install_hint = "Install with: rustup component add rust-analyzer"
```

#### Auto-Install (Advanced)

For npm-based LSPs:
```toml
[language.lsp.auto_install]
method = { type = "npm", package = "typescript-language-server", global = true }
```

## Troubleshooting

### LSP Not Working

1. **Check if language is detected:**
   ```bash
   ovim check-lsp yourfile.ext
   ```

2. **Check if LSP server is installed:**
   The `check-lsp` command will tell you if the server is found.

3. **Verify server is in PATH:**
   ```bash
   which rust-analyzer
   which typescript-language-server
   ```

4. **Check ovim's detection:**
   ```bash
   ovim list-languages --verbose
   ```

### LSP Starts But No Features

**Possible causes:**

1. **Wrong project root** - LSP needs to be initialized in the correct directory
   - Check: `ovim check-lsp yourfile.ext` shows project root
   - Fix: Add proper `root_markers` to your language config

2. **LSP initializing** - Some servers take time to index the project
   - Wait 5-10 seconds and try again
   - Check status bar in ovim

3. **LSP server error** - Check logs
   - Logs are in stderr when running `ovim --headless`
   - Look for `[LSP-REQUEST]` and `[LSP-RESPONSE]` messages

### Syntax Highlighting Not Working

**Verify grammar is loaded:**
```bash
ovim check-lsp yourfile.ext
```

Look for: `✓ Syntax Highlighting: tree-sitter-<language> grammar`

If missing:
1. Check that the tree-sitter crate is in `Cargo.toml`
2. Add syntax configuration to your `languages.toml`

### File Extension Not Recognized

**Add extension to your config:**
```toml
[[language]]
id = "rust"
extensions = ["rs", "rust"]  # Add your custom extension
```

## Ex Commands

While editing, you can check LSP status with:

```vim
:LspInfo     " Show LSP configuration for current buffer
:LspRestart  " Restart LSP server
```

*(Note: These commands will be added in Phase 4 - currently in development)*

## Examples

### Example 1: Open TypeScript File Without LSP

```bash
$ ovim app.tsx
# Status bar shows: "LSP: typescript-language-server not found"
# Auto-install prompt appears
# After installation, LSP features work
```

### Example 2: Check Rust Project

```bash
$ ovim check-lsp src/main.rs
File: /path/to/project/src/main.rs

✓ Language Detected: Rust (rust)
✓ Syntax Highlighting: tree-sitter-rust grammar
✓ LSP Configuration:
  Primary Command: rust-analyzer
✓ LSP Server Found: rust-analyzer
✓ Project Root: /path/to/project
```

### Example 3: Add Go LSP Support

Create `~/.config/ovim/languages.toml`:
```toml
[[language]]
id = "go"
extensions = ["go"]

[language.lsp]
command = "gopls"
root_markers = ["go.mod"]
install_hint = "go install golang.org/x/tools/gopls@latest"
```

Test it:
```bash
$ ovim check-lsp main.go
✓ Language Detected: Go (go)
✓ LSP Configuration:
  Primary Command: gopls
✓ LSP Server Found: gopls
✓ Project Root: /path/to/go-project
```

## Architecture Notes

### Why Declarative Configuration?

The language support system uses a declarative configuration file (`languages.toml`) rather than hardcoded Rust modules. This design:

1. **Separates data from code** - Language properties (command names, extensions) are data, not logic
2. **Enables user customization** - No need to recompile ovim to add a language
3. **Reduces code duplication** - All languages use the same initialization logic
4. **Makes debugging easier** - `ovim check-lsp` can show exactly what config was matched

### How Language Detection Works

1. **File extension** - Match file's extension against all configured languages
2. **Filename** - For extensionless files (e.g., "Dockerfile"), match exact filename
3. **Priority** - User config (`.config/ovim/languages.toml`) overrides embedded config

### How LSP Initialization Works

1. **Detect language** from file path
2. **Find LSP command** - Try primary command, then fallbacks
3. **Find project root** - Walk up directories looking for root markers
4. **Start server** - Spawn LSP process with proper root directory
5. **Initialize** - Send LSP initialize request with project capabilities

## Related Documentation

- [Architecture Analysis](/Users/adrian/Projects/ovim/notes/LANGUAGE_SUPPORT_ARCHITECTURE_ANALYSIS.md) - Design decisions and implementation details
- [CLAUDE.md](/Users/adrian/Projects/ovim/CLAUDE.md) - Quick reference for developers
- [CLI Reference](/Users/adrian/Projects/ovim/code-docs/CLI_SUBCOMMANDS.md) - All CLI commands

## Need Help?

**Check what languages are supported:**
```bash
ovim list-languages
```

**Debug why LSP isn't working:**
```bash
ovim check-lsp yourfile.ext --verbose
```

**Can't find what you need?** The configuration system is designed to be hackable. Check the examples above and the embedded `languages.toml` for patterns to follow.
