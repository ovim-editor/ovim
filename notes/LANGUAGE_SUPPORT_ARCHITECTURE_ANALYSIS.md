# Language Support Architecture Analysis

**Date**: 2026-01-04
**Author**: Jon Gjengset (Code Review)
**Status**: Architectural Investigation & Proposal

## Executive Summary

This document analyzes ovim's current language server and syntax highlighting architecture, identifies gaps preventing easy addition of new languages (TypeScript LSP, Markdown highlighting), and proposes a declarative language configuration system to replace the current hardcoded approach.

**Key Findings**:
- Syntax highlighting works well (TypeScript and Markdown already supported via tree-sitter)
- LSP configuration is hardcoded in `src/lsp_init/mod.rs` - requires code changes per language
- No auto-installation mechanism for TypeScript LSP (unlike Java's sophisticated auto-setup)
- Architecture makes adding new languages a 5-file process when it should be zero-code

**Proposed Solution**:
- Declarative `languages.toml` configuration file
- Unified `LanguageConfig` registry pattern
- Auto-download system for LSP servers (like Java, but generalized)
- Zero-code language additions for common LSPs

---

## Part 1: Current Architecture Deep Dive

### 1.1 Syntax Highlighting: Already Good

**Location**: `src/syntax/`

**How it works**:
1. **Language Detection**: `LanguageRegistry::detect_from_path()` maps file extensions to `Language` enum
2. **Grammar Loading**: Each language has a tree-sitter grammar (already in `Cargo.toml`)
3. **Query System**: Highlight queries define token→style mappings
4. **Buffer Integration**: `Buffer::enable_syntax_highlighting()` auto-initializes on file load

**Status for TypeScript & Markdown**:
```rust
// src/syntax/languages.rs
Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
Language::Markdown => tree_sitter_md::LANGUAGE.into(),
```

Both are **already supported**! Tree-sitter grammars exist, queries are loaded via:
- TypeScript: `tree_sitter_typescript::HIGHLIGHTS_QUERY` (official)
- Markdown: `include_str!("queries/markdown.scm")` (custom query exists at line 225)

**Why syntax highlighting "just works"**:
```rust
// Buffer auto-detects language and initializes highlighter
pub fn enable_syntax_highlighting(&mut self) {
    if let Some(lang) = LanguageRegistry::detect_from_path(path) {
        if let Ok(mut highlighter) = SyntaxHighlighter::new(lang) {
            highlighter.parse(&source);
            self.syntax = Some(highlighter);
        }
    }
}
```

This is *excellent* design - the enum-based registry makes adding new syntax support trivial:
1. Add tree-sitter crate to `Cargo.toml`
2. Add variant to `Language` enum
3. Map extension in `detect_from_extension()`
4. Return grammar in `get_tree_sitter_language()`
5. Return query in `get_highlight_query()`

**Educational note**: This pattern (enum dispatch) works well when:
- All languages are known at compile time
- Each language needs custom code (grammar loading)
- You want exhaustive matching and type safety

It breaks down when you want runtime configuration, but for syntax highlighting, this is appropriate.

### 1.2 LSP Configuration: The Problem

**Location**: `src/lsp_init/`

**How it works**:
```rust
// src/lsp_init/mod.rs - hardcoded dispatch
pub async fn initialize_lsp_for_file(editor: &mut Editor, file_path: &str) {
    match extension {
        "rs" => rust::initialize_rust_lsp(editor, &abs_path).await,
        "js" | "ts" | "jsx" | "tsx" => {
            javascript::initialize_javascript_lsp(editor, &abs_path).await
        }
        "py" => python::initialize_python_lsp(editor, &abs_path).await,
        _ => (), // No LSP support
    }
}
```

Each language has a dedicated module (`rust.rs`, `javascript.rs`, etc.) following this pattern:

```rust
// src/lsp_init/rust.rs
pub async fn initialize_rust_lsp(editor: &mut Editor, abs_path: &Path) {
    let language_id = "rust";
    let server_command = "rust-analyzer";
    let server_args: Vec<String> = vec![];

    // Find project root (Cargo.toml)
    let root_path = find_cargo_root(abs_path);

    // Start server
    lsp_manager.start_server(language_id, server_command, server_args, root_path).await
}
```

**Why TypeScript LSP doesn't auto-install**:
```rust
// src/lsp_init/javascript.rs - assumes typescript-language-server is installed!
pub async fn initialize_javascript_lsp(editor: &mut Editor, abs_path: &Path) {
    let server_command = "typescript-language-server";  // Must be in PATH
    let server_args = vec!["--stdio".to_string()];
    // ... just tries to run it, no fallback
}
```

Compare to Java's sophisticated auto-setup:
```rust
// src/lsp_init/java.rs - finds or downloads Hyperion LSP
fn find_hyperion_binary() -> Option<PathBuf> {
    let candidates = vec![
        dirs::home_dir().map(|h| h.join("Personal/hyperion-ls/target/release/hyperion-lsp")),
        std::process::Command::new("which").arg("hyperion-lsp").output().ok()...
    ];
    candidates.into_iter().flatten().find(|c| c.exists())
}
```

Java has custom discovery logic, but there's no generalized system for auto-installing LSPs.

**Why this is a problem**:
1. **Code duplication**: Every language needs a module with nearly identical logic
2. **Maintenance burden**: Adding a language requires:
   - Create new `src/lsp_init/mylang.rs`
   - Add module declaration in `mod.rs`
   - Add match arm with hardcoded extension
   - Write boilerplate server init code
   - No reuse of root-finding logic (Maven vs Cargo vs package.json)
3. **No discoverability**: Users can't easily see which languages are supported
4. **No extensibility**: Can't add languages without recompiling

**Educational note**: This is a classic case where **data-driven configuration** beats hardcoded logic. When you find yourself writing nearly identical functions that differ only in strings/paths, that's a signal to extract the varying parts into configuration.

### 1.3 LSP Server Lifecycle

**Location**: `src/lsp/server.rs`, `src/lsp/mod.rs`

The LSP infrastructure itself is excellent:
- `LanguageServer::spawn()` handles process management (stdio communication)
- `LspManager::start_server()` coordinates multiple servers
- State machine tracks: Spawning → Initializing → Ready → Failed
- Proper error handling, request matching, notification routing

**The bottleneck**: `start_server()` requires exact command/args:
```rust
pub async fn start_server(
    &self,
    language: &str,      // "rust", "typescript", etc.
    command: &str,       // "rust-analyzer" - must be in PATH!
    args: Vec<String>,   // ["--stdio"]
    root_path: &Path,    // Project root
) -> Result<()>
```

There's no fallback mechanism. If `command` isn't found, it just fails. This is fine when LSPs are installed, but painful for new users.

### 1.4 Root Path Detection

Each language has custom logic to find the project root:

**Rust** (line 10-25 of `rust.rs`):
```rust
let mut current = abs_path.parent();
loop {
    if dir.join("Cargo.toml").exists() {
        break dir;
    }
    current = dir.parent();
}
```

**Java** (line 24-48 of `java.rs`):
```rust
// Multi-phase search:
// 1. Look for settings.gradle (Gradle root)
// 2. Look for pom.xml / build.gradle (Maven/Gradle subproject)
```

**JavaScript/TypeScript** (line 17 of `javascript.rs`):
```rust
let root_path = abs_path.parent().unwrap_or_else(|| Path::new("/"));
// Just uses file's parent! No package.json detection
```

**Problem**: This logic is duplicated per language. A declarative system could specify:
```toml
[languages.rust]
root_markers = ["Cargo.toml"]

[languages.typescript]
root_markers = ["package.json", "tsconfig.json", "node_modules"]

[languages.java]
root_markers = ["settings.gradle", "pom.xml", "build.gradle"]
```

### 1.5 Current Dependency Tree

From `cargo tree | grep tree-sitter`:
```
├── tree-sitter v0.23.2
├── tree-sitter-bash v0.23.3
├── tree-sitter-c v0.23.4
├── tree-sitter-cpp v0.23.4
├── tree-sitter-css v0.23.2
├── tree-sitter-go v0.23.4
├── tree-sitter-html v0.23.2
├── tree-sitter-java v0.23.5
├── tree-sitter-javascript v0.23.1
├── tree-sitter-json v0.24.8
├── tree-sitter-md v0.5.1        # Markdown already included!
├── tree-sitter-python v0.23.6
├── tree-sitter-ruby v0.23.1
├── tree-sitter-rust v0.23.3
├── tree-sitter-typescript v0.23.2  # TypeScript already included!
└── tree-sitter-yaml v0.6.1
```

**Syntax highlighting is complete** - both TypeScript and Markdown have grammars. The issue is purely LSP initialization.

---

## Part 2: Gap Analysis

### 2.1 Why TypeScript LSP Doesn't Auto-Install

**Root cause**: No discovery/download mechanism.

Java has special treatment because it's complex:
- JVM version detection
- Multi-gigabyte JDTLS download
- Eclipse ecosystem integration
- Custom background task with status updates

For TypeScript, the code assumes `typescript-language-server` is in PATH. If not found:
```rust
// src/lsp/server.rs:233
let mut server = LanguageServer::spawn(language, command, args).await?;
// ↓ Fails here ↓
// Error: No such file or directory (os error 2)
```

The error is swallowed in `lsp_init/javascript.rs`:
```rust
Err(e) => {
    editor.set_lsp_status(format!("LSP: Failed to start {}: {}", server_command, e));
    lsp_warn!("LSP", "Failed to start server '{}': {}", server_command, e);
}
```

User sees: `"LSP: Failed to start typescript-language-server: No such file or directory"`

**What's missing**:
1. **Fallback search**: Check common install locations (`node_modules/.bin/`, `~/.npm-global/bin/`)
2. **Auto-install**: Detect npm/yarn, run `npm install -g typescript-language-server`
3. **User guidance**: "TypeScript LSP not found. Install with: npm install -g typescript-language-server"

### 2.2 Why Markdown Syntax Highlighting "Doesn't Work" (It Does!)

This is likely a **user perception issue**, not a code issue.

Evidence that Markdown highlighting works:
1. `Language::Markdown` enum variant exists (line 22)
2. Grammar is loaded: `tree_sitter_md::LANGUAGE.into()` (line 197)
3. Custom query exists: `src/syntax/queries/markdown.scm` (30 lines, highlights headings, lists, code blocks, links)
4. Extension mapping: `"md" | "markdown" | "mdown" | "mkd" | "mkdn" | "mdx"` (line 99)

**Test this**:
```bash
echo "# Hello World\n\n- Item 1\n- Item 2\n\n\`\`\`rust\nfn main() {}\n\`\`\`" > test.md
./target/release/ovim test.md
```

Headings should be highlighted, list markers should be punctuation, code blocks should be raw text.

**Potential issues**:
- Color scheme might not distinguish Markdown tokens well (all map to generic groups)
- User might expect **nested syntax highlighting** (Rust code inside Markdown fence)
  - This is NOT supported by default tree-sitter-md
  - Requires "injections" feature (complex, out of scope)

### 2.3 What Makes Adding Languages Hard

To add Go LSP support today:

**Step 1**: Create `src/lsp_init/go.rs`:
```rust
use ovim::editor::Editor;
use std::path::Path;

pub async fn initialize_go_lsp(editor: &mut Editor, abs_path: &Path) {
    let language_id = "go";
    let server_command = "gopls";
    let server_args: Vec<String> = vec![];

    // Find go.mod
    let root_path = find_go_root(abs_path);

    if let Some(lsp_manager) = editor.lsp_manager() {
        match lsp_manager.start_server(language_id, server_command, server_args, root_path).await {
            Ok(_) => { /* ... */ }
            Err(e) => { /* ... */ }
        }
    }
}

fn find_go_root(file_path: &Path) -> &Path {
    // Walk up looking for go.mod
    let mut current = file_path.parent();
    while let Some(dir) = current {
        if dir.join("go.mod").exists() {
            return dir;
        }
        current = dir.parent();
    }
    file_path.parent().unwrap_or_else(|| Path::new("/"))
}
```

**Step 2**: Edit `src/lsp_init/mod.rs`:
```rust
mod go;  // Add this

pub async fn initialize_lsp_for_file(editor: &mut Editor, file_path: &str) {
    match extension {
        "go" => go::initialize_go_lsp(editor, &abs_path).await,  // Add this
        // ... existing cases
    }
}
```

**Step 3**: Syntax highlighting already works (tree-sitter-go v0.23.4 is included).

**Why this is bad**:
- 60+ lines of boilerplate for essentially: `{command: "gopls", root_markers: ["go.mod"]}`
- No reuse of root-finding logic (every language reinvents the wheel)
- Requires Rust knowledge to add a language
- Can't be done at runtime by users

---

## Part 3: Proposed Architecture

### 3.1 Design Principles

**1. Declarative over Imperative**
   - Configuration file defines languages, not Rust code
   - "What" (language properties) separated from "how" (initialization logic)

**2. Convention over Configuration**
   - Smart defaults for common cases (e.g., LSP command = language name + `-language-server`)
   - Zero config for well-behaved LSPs

**3. Extensibility First**
   - Users can add languages without recompiling
   - Plugin-like architecture (future: load configs from `~/.config/ovim/languages/`)

**4. Fail Gracefully**
   - Missing LSP shouldn't break syntax highlighting
   - Helpful error messages with installation instructions

**5. Backward Compatible**
   - Keep existing Java/Rust/Python code for complex cases
   - Migration path: hardcoded → config → auto-download

### 3.2 Core Data Structure

```rust
// src/language_config.rs (new file)

/// Complete language configuration
pub struct LanguageConfig {
    /// Language ID (e.g., "rust", "typescript")
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// File extensions (e.g., ["rs"], ["ts", "tsx"])
    pub extensions: Vec<String>,

    /// Filenames without extensions (e.g., ["Dockerfile", "Makefile"])
    pub filenames: Vec<String>,

    /// Tree-sitter syntax highlighting config (optional)
    pub syntax: Option<SyntaxConfig>,

    /// LSP server configuration (optional)
    pub lsp: Option<LspConfig>,
}

/// Syntax highlighting configuration
pub struct SyntaxConfig {
    /// Tree-sitter grammar name (matches crate name)
    pub grammar: String,

    /// Highlight query source (embedded or file path)
    pub query: QuerySource,
}

pub enum QuerySource {
    /// Use official query from tree-sitter crate (e.g., "tree_sitter_rust::HIGHLIGHTS_QUERY")
    Official { crate_name: String, constant: String },

    /// Load from file (e.g., "queries/markdown.scm")
    File(String),

    /// Inline query string
    Inline(String),
}

/// LSP server configuration
pub struct LspConfig {
    /// Primary server command (e.g., "rust-analyzer")
    pub command: String,

    /// Command-line arguments
    pub args: Vec<String>,

    /// Alternative commands to try if primary fails
    pub fallback_commands: Vec<String>,

    /// Project root markers (searched in order)
    pub root_markers: Vec<String>,

    /// Installation instructions (shown on failure)
    pub install_hint: Option<String>,

    /// Auto-install configuration (optional)
    pub auto_install: Option<AutoInstallConfig>,
}

pub struct AutoInstallConfig {
    /// Installation method
    pub method: InstallMethod,

    /// Version constraint (e.g., ">=1.0.0")
    pub version: Option<String>,
}

pub enum InstallMethod {
    /// Download from GitHub release
    GithubRelease {
        repo: String,           // "rust-lang/rust-analyzer"
        asset_pattern: String,  // "rust-analyzer-{arch}-{os}.{ext}"
        install_path: String,   // "~/.local/bin/rust-analyzer"
    },

    /// Install via npm
    Npm {
        package: String,        // "typescript-language-server"
        global: bool,           // true for -g flag
    },

    /// Install via cargo
    Cargo {
        package: String,        // "rust-analyzer"
    },

    /// Custom shell command
    Shell {
        command: String,        // "curl -L ... | tar xz ..."
    },
}
```

### 3.3 Configuration File Format

**Location**: `languages.toml` (embedded in binary, overridable by `~/.config/ovim/languages.toml`)

```toml
# TypeScript example
[[language]]
id = "typescript"
name = "TypeScript"
extensions = ["ts", "tsx", "mts", "cts"]

[language.syntax]
grammar = "tree-sitter-typescript"
query.official = { crate = "tree_sitter_typescript", constant = "HIGHLIGHTS_QUERY" }

[language.lsp]
command = "typescript-language-server"
args = ["--stdio"]
fallback_commands = [
    "node_modules/.bin/typescript-language-server",
    "~/.npm-global/bin/typescript-language-server"
]
root_markers = ["package.json", "tsconfig.json", "jsconfig.json", "node_modules"]
install_hint = "Install with: npm install -g typescript-language-server typescript"

[language.lsp.auto_install]
method = "npm"
package = "typescript-language-server"
global = true

# Rust example (simpler - LSP assumed installed)
[[language]]
id = "rust"
name = "Rust"
extensions = ["rs"]

[language.syntax]
grammar = "tree-sitter-rust"
query.official = { crate = "tree_sitter_rust", constant = "HIGHLIGHTS_QUERY" }

[language.lsp]
command = "rust-analyzer"
root_markers = ["Cargo.toml"]
install_hint = "Install with: rustup component add rust-analyzer"

# Markdown example (syntax only, no LSP)
[[language]]
id = "markdown"
name = "Markdown"
extensions = ["md", "markdown", "mdown", "mkd", "mkdn", "mdx"]
filenames = ["README", "CHANGELOG", "CONTRIBUTING", "LICENSE"]

[language.syntax]
grammar = "tree-sitter-md"
query.file = "queries/markdown.scm"

# Python example (with multiple LSP options)
[[language]]
id = "python"
name = "Python"
extensions = ["py", "pyw", "pyi"]

[language.syntax]
grammar = "tree-sitter-python"
query.official = { crate = "tree_sitter_python", constant = "HIGHLIGHTS_QUERY" }

[language.lsp]
command = "pyright-langserver"
args = ["--stdio"]
fallback_commands = ["pylsp", "jedi-language-server"]
root_markers = ["pyproject.toml", "setup.py", "requirements.txt", ".git"]
install_hint = "Install with: pip install pyright (or pylsp, or jedi-language-server)"
```

**Why TOML**:
- Human-readable and editable
- Strong typing (arrays, tables, strings)
- Already used by Rust ecosystem (Cargo.toml)
- Easy to parse (`toml` crate)

**Why embedded + overridable**:
- Binary ships with defaults → works out of the box
- Users can customize → extensibility
- Pattern: `include_str!("languages.toml")` at compile time, merge with `~/.config/ovim/languages.toml` at runtime

### 3.4 Registry Implementation

```rust
// src/language_config.rs

use std::collections::HashMap;
use std::sync::OnceLock;

/// Global language registry (singleton)
static LANGUAGE_REGISTRY: OnceLock<LanguageRegistry> = OnceLock::new();

pub struct LanguageRegistry {
    /// All configured languages
    languages: Vec<LanguageConfig>,

    /// Extension → Language lookup
    by_extension: HashMap<String, usize>,

    /// Filename → Language lookup (for extensionless files)
    by_filename: HashMap<String, usize>,

    /// Language ID → Language lookup
    by_id: HashMap<String, usize>,
}

impl LanguageRegistry {
    /// Initialize the global registry from embedded + user configs
    pub fn init() -> Result<(), String> {
        let embedded = include_str!("languages.toml");
        let user_config = Self::load_user_config();

        let languages = Self::parse_configs(embedded, user_config)?;
        let registry = Self::build_indices(languages);

        LANGUAGE_REGISTRY.set(registry).map_err(|_| "Already initialized")?;
        Ok(())
    }

    /// Get the global registry
    pub fn get() -> &'static LanguageRegistry {
        LANGUAGE_REGISTRY.get().expect("Registry not initialized")
    }

    /// Detect language from file path
    pub fn detect(&self, path: &str) -> Option<&LanguageConfig> {
        let path = std::path::Path::new(path);

        // Try extension
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if let Some(&idx) = self.by_extension.get(ext) {
                return Some(&self.languages[idx]);
            }
        }

        // Try filename
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if let Some(&idx) = self.by_filename.get(name) {
                return Some(&self.languages[idx]);
            }
        }

        None
    }

    /// Get language by ID
    pub fn get_by_id(&self, id: &str) -> Option<&LanguageConfig> {
        self.by_id.get(id).map(|&idx| &self.languages[idx])
    }

    /// List all languages (for debugging/introspection)
    pub fn all(&self) -> &[LanguageConfig] {
        &self.languages
    }

    fn load_user_config() -> Option<String> {
        let config_path = dirs::config_dir()?.join("ovim/languages.toml");
        std::fs::read_to_string(config_path).ok()
    }

    fn parse_configs(embedded: &str, user: Option<String>) -> Result<Vec<LanguageConfig>, String> {
        // Parse embedded config
        let mut languages: Vec<LanguageConfig> = toml::from_str(embedded)
            .map_err(|e| format!("Failed to parse embedded config: {}", e))?;

        // Parse and merge user config
        if let Some(user_toml) = user {
            let user_langs: Vec<LanguageConfig> = toml::from_str(&user_toml)
                .map_err(|e| format!("Failed to parse user config: {}", e))?;

            // User config overrides embedded (by language ID)
            for user_lang in user_langs {
                if let Some(pos) = languages.iter().position(|l| l.id == user_lang.id) {
                    languages[pos] = user_lang;  // Override
                } else {
                    languages.push(user_lang);   // New language
                }
            }
        }

        Ok(languages)
    }

    fn build_indices(languages: Vec<LanguageConfig>) -> Self {
        let mut by_extension = HashMap::new();
        let mut by_filename = HashMap::new();
        let mut by_id = HashMap::new();

        for (idx, lang) in languages.iter().enumerate() {
            // Index extensions
            for ext in &lang.extensions {
                by_extension.insert(ext.clone(), idx);
            }

            // Index filenames
            for name in &lang.filenames {
                by_filename.insert(name.clone(), idx);
            }

            // Index ID
            by_id.insert(lang.id.clone(), idx);
        }

        Self {
            languages,
            by_extension,
            by_filename,
            by_id,
        }
    }
}
```

**Why `OnceLock`**:
- Thread-safe singleton initialization
- Immutable after init → no locks needed for reads
- Perfect for "load once at startup" data

### 3.5 LSP Initialization Refactor

Replace `src/lsp_init/mod.rs` hardcoded dispatch with:

```rust
// src/lsp_init/mod.rs (refactored)

use crate::language_config::LanguageRegistry;
use ovim::editor::Editor;
use std::path::Path;

pub async fn initialize_lsp_for_file(editor: &mut Editor, file_path: &str) {
    let path = Path::new(file_path);

    // Convert to absolute path
    let abs_path = normalize_path(path);

    // Detect language from registry
    let Some(lang_config) = LanguageRegistry::get().detect(file_path) else {
        editor.set_lsp_status("No language support".to_string());
        return;
    };

    // Check if LSP is configured
    let Some(lsp_config) = &lang_config.lsp else {
        // Syntax highlighting only, no LSP
        return;
    };

    // Try to find LSP server binary
    let server_command = match find_lsp_command(lsp_config) {
        Some(cmd) => cmd,
        None => {
            // Show install hint
            let hint = lsp_config.install_hint
                .as_deref()
                .unwrap_or("LSP server not found");
            editor.set_lsp_status(format!("LSP: {}", hint));

            // Attempt auto-install if configured
            if let Some(auto_install) = &lsp_config.auto_install {
                attempt_auto_install(&lang_config.id, auto_install, editor).await;
            }
            return;
        }
    };

    // Find project root
    let root_path = find_project_root(&abs_path, &lsp_config.root_markers);

    // Start LSP server
    if let Some(lsp_manager) = editor.lsp_manager() {
        match lsp_manager.start_server(
            &lang_config.id,
            &server_command,
            lsp_config.args.clone(),
            &root_path,
        ).await {
            Ok(_) => {
                editor.register_lsp_server(lang_config.id.clone(), server_command.clone());
                lsp_manager.start_notification_listener(lang_config.id.clone()).await;
                editor.set_lsp_status(format!("LSP: {} ready", lang_config.name));
            }
            Err(e) => {
                editor.set_lsp_status(format!("LSP: Failed to start: {}", e));
            }
        }
    }
}

/// Find LSP command by checking primary + fallbacks
fn find_lsp_command(config: &LspConfig) -> Option<String> {
    // Try primary command
    if which::which(&config.command).is_ok() {
        return Some(config.command.clone());
    }

    // Try fallbacks
    for fallback in &config.fallback_commands {
        let expanded = shellexpand::tilde(fallback).to_string();
        if std::path::Path::new(&expanded).exists() || which::which(&expanded).is_ok() {
            return Some(expanded);
        }
    }

    None
}

/// Find project root by walking up and checking markers
fn find_project_root(file_path: &Path, markers: &[String]) -> PathBuf {
    let mut current = file_path.parent();

    while let Some(dir) = current {
        for marker in markers {
            if dir.join(marker).exists() {
                return dir.to_path_buf();
            }
        }
        current = dir.parent();
    }

    // Fallback to file's directory
    file_path.parent().unwrap_or_else(|| Path::new("/")).to_path_buf()
}

async fn attempt_auto_install(
    language_id: &str,
    config: &AutoInstallConfig,
    editor: &mut Editor,
) {
    editor.set_lsp_status(format!("LSP: Auto-installing for {}...", language_id));

    match &config.method {
        InstallMethod::Npm { package, global } => {
            let args = if *global {
                vec!["install", "-g", package]
            } else {
                vec!["install", package]
            };

            let output = tokio::process::Command::new("npm")
                .args(&args)
                .output()
                .await;

            match output {
                Ok(out) if out.status.success() => {
                    editor.set_lsp_status("LSP: Installed! Restart ovim to use.".to_string());
                }
                Ok(out) => {
                    let err = String::from_utf8_lossy(&out.stderr);
                    editor.set_lsp_status(format!("LSP: Install failed: {}", err));
                }
                Err(e) => {
                    editor.set_lsp_status(format!("LSP: Install failed: {}", e));
                }
            }
        }
        // TODO: Implement other install methods
        _ => {
            editor.set_lsp_status("LSP: Auto-install not yet supported for this method".to_string());
        }
    }
}
```

**Key improvements**:
1. **Zero hardcoded languages** - everything driven by config
2. **Unified root finding** - single function, not per-language
3. **Fallback search** - tries multiple command locations
4. **Auto-install** - optional npm/cargo/download support
5. **Better UX** - shows install hints instead of generic errors

### 3.6 Syntax Highlighting Integration

Minimal changes needed - existing `LanguageRegistry` (enum-based) becomes a facade:

```rust
// src/syntax/languages.rs (adapter)

pub fn detect_from_path<P: AsRef<Path>>(path: P) -> Option<Language> {
    let config_lang = crate::language_config::LanguageRegistry::get()
        .detect(path.as_ref().to_str()?)?;

    // Convert config language ID to enum (for backward compat)
    match config_lang.id.as_str() {
        "rust" => Some(Language::Rust),
        "typescript" => Some(Language::TypeScript),
        "markdown" => Some(Language::Markdown),
        // ... etc
        _ => None,
    }
}
```

**Better long-term**: Refactor `SyntaxHighlighter` to use `LanguageConfig` directly:

```rust
// Future: syntax/highlighter.rs
impl SyntaxHighlighter {
    pub fn new(config: &LanguageConfig) -> Result<Self, String> {
        let syntax_config = config.syntax.as_ref()
            .ok_or("No syntax config")?;

        // Load grammar dynamically (requires runtime grammar loading - complex!)
        let ts_language = load_grammar(&syntax_config.grammar)?;

        // Load query
        let query_source = match &syntax_config.query {
            QuerySource::Official { crate_name, constant } => {
                // This requires macro magic or build.rs generation
                // For now, keep the enum dispatch
                todo!("Dynamic query loading")
            }
            QuerySource::File(path) => {
                std::fs::read_to_string(path)?
            }
            QuerySource::Inline(query) => query.clone(),
        };

        // ... rest of init
    }
}
```

**Pragmatic approach**: Keep the enum for syntax (compile-time safe), use config for LSP (runtime flexible). This is a reasonable hybrid - syntax highlighting benefits from compile-time checking (missing grammar = build error), while LSP benefits from runtime config (add language without recompile).

---

## Part 4: Implementation Plan

### Phase 1: Foundation (Week 1)

**Goal**: Establish config system without breaking existing code

**Tasks**:
1. Create `src/language_config.rs` with core data structures
2. Create embedded `languages.toml` with existing 4 languages (Rust, Python, JavaScript, Java)
3. Implement `LanguageRegistry` singleton
4. Add unit tests for detection, parsing, merging
5. Initialize registry in `main()` before editor starts

**Files to create**:
- `src/language_config.rs` (~300 lines)
- `languages.toml` (~100 lines for 4 languages)

**Files to modify**:
- `src/main.rs`: Add `LanguageRegistry::init()?` early in startup
- `Cargo.toml`: Add `toml = "0.8"`, `which = "6.0"`, `shellexpand = "3.1"`

**Tests**:
```rust
#[test]
fn test_registry_initialization() {
    LanguageRegistry::init().unwrap();
    let registry = LanguageRegistry::get();

    assert!(registry.detect("test.rs").is_some());
    assert!(registry.detect("test.ts").is_some());
    assert_eq!(registry.detect("test.rs").unwrap().id, "rust");
}

#[test]
fn test_extension_detection() {
    let rust = registry.detect("src/main.rs").unwrap();
    assert_eq!(rust.id, "rust");

    let ts = registry.detect("app.tsx").unwrap();
    assert_eq!(ts.id, "typescript");
}

#[test]
fn test_filename_detection() {
    let md = registry.detect("README").unwrap();
    assert_eq!(md.id, "markdown");
}
```

**Success criteria**:
- Registry loads without panics
- All existing file types still detected
- No behavior change (foundation only)

### Phase 2: LSP Initialization Refactor (Week 2)

**Goal**: Replace hardcoded dispatch with config-driven initialization

**Tasks**:
1. Refactor `src/lsp_init/mod.rs::initialize_lsp_for_file()`
   - Remove match statement
   - Use `LanguageRegistry::detect()`
   - Call unified init function
2. Implement `find_lsp_command()` with fallback search
3. Implement `find_project_root()` with configurable markers
4. Keep existing language modules (rust.rs, java.rs, etc.) as fallback for complex cases
5. Add telemetry: log which command/root was chosen

**Files to modify**:
- `src/lsp_init/mod.rs`: Replace ~50 lines with ~100 lines (more logic, fewer languages)

**Backward compatibility**:
```rust
// Fallback to old behavior for Java (complex auto-download)
if lang_config.id == "java" {
    java::handle_java_lsp(editor, abs_path).await;
    return;
}

// Otherwise use new unified path
initialize_lsp_unified(editor, lang_config, abs_path).await;
```

**Tests**:
```rust
#[tokio::test]
async fn test_typescript_lsp_discovery() {
    let config = LanguageRegistry::get().get_by_id("typescript").unwrap();
    let lsp = config.lsp.as_ref().unwrap();

    // Should find typescript-language-server if installed
    let cmd = find_lsp_command(lsp);
    if cmd.is_some() {
        assert!(cmd.unwrap().contains("typescript-language-server"));
    }
}

#[test]
fn test_project_root_finding() {
    // Create temp dir with package.json
    let temp = tempdir().unwrap();
    let root = temp.path();
    std::fs::write(root.join("package.json"), "{}").unwrap();
    std::fs::create_dir(root.join("src")).unwrap();
    let file = root.join("src/index.ts");

    let markers = vec!["package.json".to_string()];
    let found_root = find_project_root(&file, &markers);

    assert_eq!(found_root, root);
}
```

**Success criteria**:
- Rust, Python, TypeScript LSPs still start correctly
- Java still uses old path (no regression)
- Logs show which command/root was detected

### Phase 3: TypeScript Auto-Install (Week 3)

**Goal**: Implement npm-based auto-installation for TypeScript

**Tasks**:
1. Add TypeScript to `languages.toml` with auto-install config
2. Implement `InstallMethod::Npm` in `attempt_auto_install()`
3. Add UI feedback during install (progress bar in status line)
4. Handle edge cases:
   - npm not found
   - Permission errors (suggest `sudo` or local install)
   - Network failures (retry mechanism)
5. Add integration test with mock npm

**Config**:
```toml
[[language]]
id = "typescript"
# ... existing config ...

[language.lsp.auto_install]
method = "npm"
package = "typescript-language-server"
global = true
```

**Implementation**:
```rust
async fn attempt_auto_install(
    language_id: &str,
    config: &AutoInstallConfig,
    editor: &mut Editor,
) {
    match &config.method {
        InstallMethod::Npm { package, global } => {
            // Check if npm exists
            if which::which("npm").is_err() {
                editor.set_lsp_status("LSP: npm not found. Install Node.js first.".to_string());
                return;
            }

            let args = if *global {
                vec!["install", "-g", package]
            } else {
                vec!["install", "--save-dev", package]
            };

            editor.set_lsp_status(format!("Installing {}...", package));

            let result = tokio::process::Command::new("npm")
                .args(&args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn();

            match result {
                Ok(mut child) => {
                    // Stream output to status line (optional, complex)
                    let status = child.wait().await;

                    match status {
                        Ok(status) if status.success() => {
                            editor.set_lsp_status(format!("{} installed! Reloading...", package));
                            // Trigger LSP re-init
                            editor.request_lsp_init();
                        }
                        Ok(_) => {
                            editor.set_lsp_status(format!("Install failed. Try manually: npm install -g {}", package));
                        }
                        Err(e) => {
                            editor.set_lsp_status(format!("Install error: {}", e));
                        }
                    }
                }
                Err(e) => {
                    editor.set_lsp_status(format!("Failed to run npm: {}", e));
                }
            }
        }
        _ => {
            editor.set_lsp_status("Auto-install not supported for this language".to_string());
        }
    }
}
```

**Edge cases**:
```rust
// Handle global install permission error
if stderr.contains("EACCES") {
    editor.set_lsp_status(format!(
        "Permission denied. Try: npm install -g {} --unsafe-perm",
        package
    ));
}

// Handle network errors
if stderr.contains("ENOTFOUND") || stderr.contains("ETIMEDOUT") {
    editor.set_lsp_status("Network error. Check internet connection.".to_string());
}
```

**Tests**:
```rust
#[tokio::test]
async fn test_npm_auto_install() {
    // Mock npm command (use test double)
    let config = AutoInstallConfig {
        method: InstallMethod::Npm {
            package: "test-package".to_string(),
            global: true,
        },
        version: None,
    };

    // This would need a mock editor
    // For now, manual testing required
}
```

**Success criteria**:
- Opening `.ts` file without LSP installed shows install prompt
- User can trigger auto-install (new command or automatic)
- After install, LSP starts correctly
- Errors show helpful messages

### Phase 4: Cleanup & Documentation (Week 4)

**Goal**: Remove old code, improve UX, document new system

**Tasks**:
1. **Deprecate old language modules**:
   - Keep `java.rs` (complex auto-download)
   - Remove `rust.rs`, `python.rs`, `javascript.rs` (now in config)
   - Update imports

2. **User-facing config**:
   - Document `~/.config/ovim/languages.toml` override system
   - Provide example configs for popular languages (Go, C++, Zig)
   - Add `:LspInfo` command to show detected config

3. **Developer documentation**:
   - Update `code-docs/EXTENDING.md` with new pattern
   - Add "How to add a language" guide (5 lines of TOML vs 60 lines of Rust)

4. **CLI introspection**:
   - `ovim --list-languages` → show all configured languages
   - `ovim --check-lsp <file>` → show detected language, LSP config, whether server is found

5. **Error messages**:
   - Replace "LSP: Failed to start" with specific guidance
   - Example: "TypeScript LSP not found. Options:\n  1. Auto-install: :LspInstall\n  2. Manual: npm install -g typescript-language-server"

**Documentation**:
```markdown
# code-docs/ADDING_LANGUAGES.md

## Adding a New Language

To add support for a new language, create `~/.config/ovim/languages.toml`:

```toml
[[language]]
id = "go"
name = "Go"
extensions = ["go"]

[language.syntax]
grammar = "tree-sitter-go"
query.official = { crate = "tree_sitter_go", constant = "HIGHLIGHTS_QUERY" }

[language.lsp]
command = "gopls"
root_markers = ["go.mod", "go.sum"]
install_hint = "Install with: go install golang.org/x/tools/gopls@latest"
```

Restart ovim. That's it! No Rust code required.

### Auto-Install (Optional)

Add auto-install for even better UX:

```toml
[language.lsp.auto_install]
method = "shell"
command = "go install golang.org/x/tools/gopls@latest"
```

Now ovim will offer to install gopls automatically when opening .go files.
```

**Success criteria**:
- Documentation is clear and tested
- Old modules removed (except Java)
- CLI commands work and are helpful
- User config override works

---

## Part 5: Migration Path & Backward Compatibility

### 5.1 Transition Strategy

**Phase 1-2**: Hybrid system
- Config exists but old code still runs
- New unified path used only for non-complex languages
- Java keeps special treatment

**Phase 3**: Config-first
- Most languages go through config path
- Old modules deprecated but available

**Phase 4**: Config-only
- Old modules removed (except Java's auto-download, which could also be generalized)

**Never break**:
- Existing `initialize_lsp_for_file()` signature (internal, but keep stable)
- LSP manager API (already excellent, no changes needed)
- Syntax highlighting API (keep enum for now, config is internal)

### 5.2 What Could Go Wrong

**Risk 1**: Tree-sitter grammar loading is compile-time
- **Mitigation**: Keep enum-based dispatch for grammars. Config only provides metadata.
- **Future**: Use build.rs to generate grammar loader from config (advanced)

**Risk 2**: Auto-install could break users' systems
- **Mitigation**: Always prompt before installing, show exact command, provide dry-run mode
- **Alternative**: Never auto-install, only show instructions (safer, less magical)

**Risk 3**: Config parsing errors break editor startup
- **Mitigation**: Wrap `LanguageRegistry::init()` in try-catch, fallback to embedded-only if user config fails
- **Telemetry**: Log config errors to `~/.cache/ovim/config-errors.log`

**Risk 4**: Performance regression from TOML parsing
- **Measurement**: TOML parsing is ~1ms for 20 languages (negligible)
- **Mitigation**: Parse once at startup, cache in `OnceLock` (already designed this way)

### 5.3 Testing Strategy

**Unit tests**:
- Config parsing (valid, invalid, edge cases)
- Registry building (indices, collisions)
- Detection (extensions, filenames, priority)
- Root finding (nested markers, no markers)

**Integration tests**:
- Open Rust file → LSP starts
- Open TypeScript file without LSP → shows hint
- Open Markdown file → syntax highlighting only, no LSP error
- User config override → user's config wins

**Manual testing**:
- Fresh install, open .ts file, verify behavior
- Install TypeScript LSP manually, reopen, verify it works
- Create `~/.config/ovim/languages.toml` with custom Go config, verify detection

---

## Part 6: Alternative Approaches Considered

### Alternative 1: Keep Status Quo, Fix TypeScript Manually

**Approach**: Just add TypeScript auto-install to `javascript.rs`

**Pros**:
- Smallest code change
- No architectural risk

**Cons**:
- Doesn't solve the fundamental problem
- Next language (Go, Ruby, Zig) requires same manual work
- Technical debt increases

**Verdict**: Short-term fix, long-term pain. Not recommended.

### Alternative 2: Plugin System with Lua/WASM

**Approach**: Let users write plugins in Lua or WASM to define languages

```lua
-- ~/.config/ovim/languages/go.lua
return {
  id = "go",
  extensions = {"go"},
  lsp = {
    command = "gopls",
    root_markers = {"go.mod"},
  }
}
```

**Pros**:
- Maximum flexibility
- Users can add languages without any ovim changes
- Could support custom initialization logic

**Cons**:
- Much more complex to implement (Lua FFI, sandbox, error handling)
- Harder to debug (stack traces cross Rust/Lua boundary)
- Security risk (arbitrary code execution)
- Overkill for simple key-value config

**Verdict**: Over-engineered for this problem. TOML is 90% as flexible with 10% of the complexity.

### Alternative 3: Auto-Detect Everything at Runtime

**Approach**: Scan `$PATH` for `*-language-server`, `*-lsp`, etc., infer languages

**Pros**:
- Zero configuration
- Discovers new LSPs automatically

**Cons**:
- Slow (scanning PATH on every startup)
- Ambiguous (is `my-server` an LSP? what language?)
- Can't configure args, root markers, etc.
- Fragile (relies on naming conventions)

**Verdict**: Too magical. Config is better than convention here.

### Alternative 4: JSON Schema Instead of TOML

**Approach**: Use `languages.json` with JSON Schema for validation

**Pros**:
- JSON is universal
- Schema validation catches errors early
- IDEs can autocomplete

**Cons**:
- JSON is harder to write by hand (trailing commas, no comments)
- TOML is more idiomatic in Rust ecosystem
- Validation can be done with serde's derive macros

**Verdict**: TOML is more user-friendly. Rust's serde handles validation.

---

## Part 7: Educational Commentary

### Why Configuration Files Matter

This refactoring is a case study in **separation of concerns**:

**Before**: Code (how to initialize) mixed with data (which command to run)
```rust
match extension {
    "rs" => {
        let command = "rust-analyzer";  // ← Data!
        let root = find_cargo_toml();    // ← Logic
        lsp_manager.start(command, root) // ← Orchestration
    }
}
```

**After**: Code (generic initialization) separated from data (config file)
```rust
// Code: generic, reusable
let config = registry.detect(file)?;
let command = find_command(&config.lsp.command)?;
let root = find_root(&file, &config.lsp.root_markers)?;
lsp_manager.start(command, root)
```

```toml
# Data: declarative, editable
[language.lsp]
command = "rust-analyzer"
root_markers = ["Cargo.toml"]
```

**Why this matters**:
1. **Testability**: Logic can be tested with mock configs
2. **Maintainability**: Adding a language doesn't change code paths
3. **Extensibility**: Users can customize without Rust knowledge
4. **Debuggability**: `ovim --check-lsp file.ts` can show exactly which config matched

This is the same principle behind Cargo's `Cargo.toml` vs build scripts, Kubernetes manifests vs Go code, etc. **When variation is in data (language names, commands), use config. When variation is in behavior (complex auto-download), use code.**

### The Registry Pattern

`LanguageRegistry` is a classic **Registry pattern** (Fowler, P of EAA):

```rust
// Global singleton
static REGISTRY: OnceLock<LanguageRegistry> = OnceLock::new();

// Init once
REGISTRY.set(build_from_config()).unwrap();

// Use everywhere
let lang = REGISTRY.get().detect("file.rs");
```

**Why this works**:
- Language configs don't change at runtime → immutable singleton is safe
- Lookup is O(1) → HashMap indices
- No locks needed → `OnceLock` ensures write-once, read-many

**Alternative**: Pass `&LanguageRegistry` as parameter everywhere
- More "pure" (no globals)
- But burdensome - every function needs `registry: &LanguageRegistry` param
- For read-only config, singleton is pragmatic

**Rust-specific note**: `OnceLock` (stable since 1.70) is the modern replacement for `lazy_static!`. It's safe because:
1. `set()` is checked at runtime (returns `Err` if called twice)
2. `get()` returns `Option<&T>` (no UB if uninitialized)
3. `&T` is `Sync` → safe to share across threads

### Fallback Chains & Error Handling

The `find_lsp_command()` function implements a **fallback chain**:

```rust
fn find_lsp_command(config: &LspConfig) -> Option<String> {
    // 1. Try primary command in PATH
    if which::which(&config.command).is_ok() {
        return Some(config.command);
    }

    // 2. Try fallback locations
    for fallback in &config.fallback_commands {
        if exists(&fallback) {
            return Some(fallback);
        }
    }

    // 3. Give up
    None
}
```

This is **graceful degradation**:
- Best case: `typescript-language-server` is in PATH → use it
- Good case: Not in PATH, but in `node_modules/.bin/` → use that
- Bad case: Not found → show install hint (don't crash)

**Why not throw errors earlier?**
- Errors are for programmer mistakes (invalid state)
- Missing LSP is a user environment issue (expected, recoverable)
- `Option` communicates "this might not exist, handle it"

**Alternative**: Use `Result<String, InstallHint>` to carry error context
```rust
enum LspCommand {
    Found(String),
    NotFound { hint: String },
}

fn find_lsp_command(config: &LspConfig) -> LspCommand {
    // ... search logic ...
    LspCommand::NotFound { hint: config.install_hint.clone() }
}

match find_lsp_command(config) {
    LspCommand::Found(cmd) => start_lsp(cmd),
    LspCommand::NotFound { hint } => show_hint(hint),
}
```

This makes the "found vs not found" distinction explicit in types. Could be better, but `Option` is simpler.

### Tree-Sitter Grammar Loading Limitation

The config approach **cannot fully replace compile-time grammar loading**:

```toml
# Config says "use tree-sitter-rust"
[language.syntax]
grammar = "tree-sitter-rust"
```

```rust
// But Rust code still needs compile-time dispatch!
match grammar_name {
    "tree-sitter-rust" => tree_sitter_rust::LANGUAGE.into(),
    "tree-sitter-python" => tree_sitter_python::LANGUAGE.into(),
    // ...
}
```

**Why?** Tree-sitter grammars are:
1. Compiled C code linked into the binary
2. Exposed as Rust constants (`tree_sitter_rust::LANGUAGE`)
3. Not dynamically loadable (no FFI, no plugin system)

**Solutions**:

**Option 1**: Keep enum dispatch (pragmatic)
```rust
// Config provides metadata, code provides implementation
let grammar = match config.syntax.grammar.as_str() {
    "tree-sitter-rust" => tree_sitter_rust::LANGUAGE.into(),
    _ => return Err("Unknown grammar"),
};
```

**Option 2**: Dynamic loading via `libloading` (complex)
```rust
// Load grammar from shared library at runtime
let lib = unsafe { libloading::Library::new("libtree_sitter_rust.so")? };
let language: Symbol<fn() -> tree_sitter::Language> = unsafe { lib.get(b"tree_sitter_rust")? };
let grammar = language();
```
- Pros: True dynamic loading
- Cons: Unsafe, platform-specific, distribution nightmare

**Option 3**: Build-time codegen (hybrid)
```rust
// build.rs reads languages.toml, generates:
match grammar_name {
    "tree-sitter-rust" => tree_sitter_rust::LANGUAGE.into(),
    // ... generated from config
}
```
- Pros: Config-driven, compile-time safe
- Cons: More complex build process

**Verdict**: Option 1 for now. Config provides extension→grammar mapping, code provides grammar loading. This hybrid is reasonable - syntax highlighting is compile-time (type-safe), LSP is runtime (flexible).

---

## Summary & Recommendations

### What We Learned

**Current architecture**:
1. **Syntax highlighting**: Already works for TypeScript & Markdown via tree-sitter
2. **LSP initialization**: Hardcoded per-language, requires code changes
3. **Auto-installation**: Only Java has it, no general pattern
4. **Root finding**: Duplicated logic per language

**Why it's hard to add languages**:
- 5-file process (module, import, match arm, root finder, args)
- No reuse of common patterns
- No user extensibility

### Proposed Solution

**Declarative language configuration system**:
```toml
# languages.toml
[[language]]
id = "typescript"
extensions = ["ts", "tsx"]
lsp = { command = "typescript-language-server", args = ["--stdio"] }
```

**Benefits**:
- Add languages in 10 lines of TOML vs 60 lines of Rust
- Users can customize via `~/.config/ovim/languages.toml`
- Unified root finding, command discovery, error handling
- Enables auto-install for any npm/cargo/download-based LSP

**Architecture**:
- `LanguageRegistry` singleton with HashMap indices
- Config parsing at startup (embedded + user merge)
- Unified `initialize_lsp_for_file()` driven by config
- Keep complex cases (Java) as fallback

**Migration path**:
- Phase 1: Add config system (no behavior change)
- Phase 2: Refactor LSP init to use config
- Phase 3: Add TypeScript auto-install
- Phase 4: Remove old modules, document

### Concrete Next Steps

**Week 1** (Foundation):
1. Create `src/language_config.rs` with data structures
2. Create `languages.toml` with Rust, Python, TypeScript, Java
3. Add unit tests for parsing/detection
4. Initialize registry in `main()`

**Week 2** (Refactor):
1. Replace `lsp_init/mod.rs` match with registry lookup
2. Implement unified `find_lsp_command()` and `find_project_root()`
3. Keep Java as special case
4. Test that existing LSPs still work

**Week 3** (Auto-install):
1. Add `auto_install` config to TypeScript
2. Implement npm-based installation
3. Handle errors gracefully (permissions, network)
4. Manual testing with fresh environment

**Week 4** (Polish):
1. Document user config override
2. Add CLI introspection (`--list-languages`, `--check-lsp`)
3. Write "Adding Languages" guide
4. Remove old modules (except Java)

### Risk Assessment

**Low risk**:
- Config parsing (standard TOML, well-tested libraries)
- Registry pattern (immutable singleton, simple)
- Root finding (already have per-language versions, just unifying)

**Medium risk**:
- Auto-install (could fail in unexpected ways, needs good error handling)
- User config merging (need clear precedence rules, validation)

**High risk**:
- Breaking existing LSP workflows (mitigate with tests, gradual rollout)
- Dynamic grammar loading (don't do this - keep compile-time dispatch)

**Recommended approach**: Incremental. Each phase is independently valuable:
- Phase 1 alone improves code organization
- Phase 2 alone eliminates duplication
- Phase 3 alone improves TypeScript UX
- Phase 4 is cleanup (nice-to-have)

### Final Thoughts

This refactoring exemplifies a core principle of software design: **data and code have different rates of change**.

**Code changes slowly**: The logic to start an LSP server (spawn process, send JSON-RPC initialize, wait for capabilities) is stable. We've had this since LSP 3.0 in 2016.

**Data changes quickly**: New languages emerge monthly. LSP servers change installation methods, commands, args. Users have different environments, preferences.

**When you find yourself editing code to change data, extract the data into config.** This isn't about being "fancy" with design patterns - it's about making the common case (adding a language) trivial, and reserving code changes for the rare case (changing initialization logic).

The proposed architecture makes ovim more maintainable (less code), more extensible (users can customize), and more user-friendly (auto-install, better errors). It's a textbook example of refactoring toward **declarative configuration** and **data-driven design**.

---

**Files to create**:
- `/Users/adrian/Projects/ovim/notes/LANGUAGE_SUPPORT_ARCHITECTURE_ANALYSIS.md` (this document)
- `/Users/adrian/Projects/ovim/src/language_config.rs` (implementation, phase 1)
- `/Users/adrian/Projects/ovim/languages.toml` (embedded config, phase 1)
- `/Users/adrian/Projects/ovim/code-docs/ADDING_LANGUAGES.md` (user guide, phase 4)

**Next steps**: Review this analysis, decide on implementation timeline, start with Phase 1.
