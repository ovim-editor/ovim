# Language Support: Before & After Comparison

**Context**: This document shows the concrete difference between the current hardcoded approach and the proposed declarative configuration system.

---

## Adding TypeScript LSP Support

### Current Approach (Hardcoded)

**Files to modify**: 2
**Lines of code**: ~60
**Requires**: Rust knowledge, recompilation

#### Step 1: Create `src/lsp_init/typescript.rs`

```rust
use ovim::editor::Editor;
use std::path::Path;

/// Initialize TypeScript LSP (typescript-language-server)
pub async fn initialize_typescript_lsp(editor: &mut Editor, abs_path: &Path) {
    let language_id = "typescript";
    let server_command = "typescript-language-server";
    let server_args = vec!["--stdio".to_string()];

    // Find project root (package.json)
    let root_path = find_typescript_root(abs_path);

    // Start the language server
    if let Some(lsp_manager) = editor.lsp_manager() {
        match lsp_manager
            .start_server(language_id, server_command, server_args, root_path)
            .await
        {
            Ok(_) => {
                editor.register_lsp_server(
                    language_id.to_string(),
                    server_command.to_string(),
                );

                lsp_manager
                    .start_notification_listener(language_id.to_string())
                    .await;

                editor.set_lsp_status(format!("LSP: {} ready", server_command));
            }
            Err(e) => {
                editor.set_lsp_status(format!(
                    "LSP: Failed to start {}: {}",
                    server_command, e
                ));
                ovim::lsp_warn!(
                    "LSP",
                    "Failed to start server '{}': {}",
                    server_command,
                    e
                );
            }
        }
    }
}

fn find_typescript_root(file_path: &Path) -> &Path {
    let mut current = file_path.parent();

    while let Some(dir) = current {
        if dir.join("package.json").exists()
            || dir.join("tsconfig.json").exists()
            || dir.join("node_modules").exists()
        {
            return dir;
        }
        current = dir.parent();
    }

    file_path.parent().unwrap_or_else(|| Path::new("/"))
}
```

#### Step 2: Edit `src/lsp_init/mod.rs`

```rust
mod typescript;  // Add this line

pub async fn initialize_lsp_for_file(editor: &mut Editor, file_path: &str) {
    let extension = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    match extension {
        "rs" => rust::initialize_rust_lsp(editor, &abs_path).await,
        "js" => javascript::initialize_javascript_lsp(editor, &abs_path).await,
        "ts" | "tsx" => typescript::initialize_typescript_lsp(editor, &abs_path).await,  // Add this
        "py" => python::initialize_python_lsp(editor, &abs_path).await,
        _ => (),
    }
}
```

**Problems**:
- 60 lines of boilerplate for what is essentially configuration
- Root-finding logic duplicated (every language does the same thing)
- No fallback search (assumes `typescript-language-server` is in PATH)
- No install guidance when LSP not found
- Can't add language without recompiling

---

### Proposed Approach (Declarative)

**Files to modify**: 1 (config file)
**Lines of code**: ~15
**Requires**: TOML knowledge, no recompilation

#### Edit `~/.config/ovim/languages.toml`

```toml
[[language]]
id = "typescript"
name = "TypeScript"
extensions = ["ts", "tsx", "mts", "cts"]

[language.syntax]
grammar = "tree-sitter-typescript"
official = { crate = "tree_sitter_typescript", constant = "HIGHLIGHTS_QUERY" }

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
method = { type = "npm", package = "typescript-language-server", global = true }
```

**Done!** No code changes needed. The unified LSP initialization logic handles:
- Extension detection
- Fallback command search
- Root finding via markers
- Install hints on failure
- Optional auto-install

**Benefits**:
- 15 lines vs 60 lines (75% less)
- Declarative (what, not how)
- Reusable (root finding is shared)
- Extensible (users can add languages)
- Better UX (fallbacks + install hints)

---

## Adding Markdown Syntax Highlighting

### Current Approach

**Already works!** Markdown syntax highlighting is fully functional:

```rust
// src/syntax/languages.rs (line 22, 99, 197, 225)
Language::Markdown => tree_sitter_md::LANGUAGE.into(),
```

**Extension mapping**:
```rust
"md" | "markdown" | "mdown" | "mkd" | "mkdn" | "mdx" => Some(Language::Markdown),
```

**Query**:
```rust
Language::Markdown => include_str!("queries/markdown.scm"),
```

**Verdict**: No changes needed. The issue was likely user perception or color scheme not distinguishing Markdown tokens well.

### Proposed Approach

**Same - already works**, but config makes it more visible:

```toml
[[language]]
id = "markdown"
name = "Markdown"
extensions = ["md", "markdown", "mdown", "mkd", "mkdn", "mdx"]
filenames = ["readme", "changelog", "contributing"]

[language.syntax]
grammar = "tree-sitter-md"
file = { path = "src/syntax/queries/markdown.scm" }

# No LSP - syntax highlighting only
```

Users can now:
- See which extensions are supported: `ovim --list-languages | grep markdown`
- Override query if needed: custom `~/.config/ovim/languages.toml`
- Check detection: `ovim --check-lsp README.md` → "Language: Markdown (syntax only)"

---

## Comparison: Adding a New Language (Go)

### Current Approach

1. Create `src/lsp_init/go.rs`:
   - 60 lines of boilerplate
   - Custom `find_go_root()` function
   - Error handling
   - LSP manager integration

2. Edit `src/lsp_init/mod.rs`:
   - Add `mod go;`
   - Add match arm: `"go" => go::initialize_go_lsp(...)`

3. Recompile

**Time**: 30-60 minutes (write code, test, debug)

### Proposed Approach

1. Edit `languages.toml`:

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
root_markers = ["go.mod", "go.sum"]
install_hint = "Install with: go install golang.org/x/tools/gopls@latest"
```

**Time**: 5 minutes (copy template, fill in values)

---

## Error Handling Comparison

### Current: Generic Errors

```
LSP: Failed to start typescript-language-server: No such file or directory (os error 2)
```

**Problems**:
- User doesn't know what to do
- No guidance on installation
- Error code is cryptic

### Proposed: Helpful Guidance

```
TypeScript LSP not found. Options:
  1. Install with: npm install -g typescript-language-server typescript
  2. Auto-install: Press <Leader>li
  3. Manual: Add to PATH or configure fallback_commands

Searched:
  ✗ typescript-language-server (not in PATH)
  ✗ node_modules/.bin/typescript-language-server (not found)
  ✗ ~/.npm-global/bin/typescript-language-server (not found)
```

**Benefits**:
- Clear next steps
- Shows what was searched
- Offers auto-install option
- Educational (teaches where LSPs live)

---

## Code Organization Comparison

### Current Structure

```
src/lsp_init/
├── mod.rs                 # Hardcoded dispatch (50 lines)
├── rust.rs                # Rust LSP init (54 lines)
├── python.rs              # Python LSP init (40 lines)
├── javascript.rs          # JS/TS LSP init (46 lines)
└── java.rs                # Java LSP init (180 lines, complex)

Total: ~370 lines across 5 files
```

**Problems**:
- High duplication (root finding, error handling)
- No shared logic
- Every language needs a file

### Proposed Structure

```
src/
├── language_config.rs     # Registry + core logic (300 lines)
└── lsp_init/
    ├── mod.rs             # Unified init using config (150 lines)
    └── java.rs            # Java special case (180 lines, kept for complex auto-download)

languages.toml             # All language configs (150 lines for 15 languages)

Total: ~630 lines (but supports unlimited languages via config)
```

**Benefits**:
- Centralized logic (one place to fix bugs)
- Shared root finding, command search, error handling
- Easy to add languages (config only)
- Better tested (fewer code paths)

---

## API Comparison: LSP Initialization

### Current (Internal)

```rust
// Hardcoded per language
match extension {
    "rs" => rust::initialize_rust_lsp(editor, &abs_path).await,
    "ts" => typescript::initialize_typescript_lsp(editor, &abs_path).await,
    // ... 10 more languages
}
```

### Proposed (Unified)

```rust
// Config-driven
let config = LanguageRegistry::get().detect(file_path)?;

if let Some(lsp_config) = &config.lsp {
    let command = find_lsp_command(lsp_config)?;
    let root = find_project_root(&abs_path, &lsp_config.root_markers);

    lsp_manager.start_server(&config.id, &command, lsp_config.args.clone(), &root).await?;
}
```

**Benefits**:
- Single code path (easier to reason about)
- Testable (inject mock config)
- Traceable (log which config matched)
- Extensible (add fields to config without changing code)

---

## User Experience Comparison

### Scenario: User Opens `app.tsx` Without TypeScript LSP Installed

#### Current Behavior

1. Open file: `ovim app.tsx`
2. Editor shows: `"LSP: Failed to start typescript-language-server: No such file or directory"`
3. User Googles error
4. Finds: "Install with `npm install -g typescript-language-server`"
5. Installs manually
6. Restarts ovim
7. Works

**Steps**: 7
**Time**: 5-10 minutes (if user knows npm)

#### Proposed Behavior

1. Open file: `ovim app.tsx`
2. Editor shows:
   ```
   TypeScript LSP not found.
   Install with: npm install -g typescript-language-server typescript
   Press <Leader>li to auto-install
   ```
3. User presses `<Leader>li`
4. Editor shows: `"Installing typescript-language-server..."`
5. Editor shows: `"Installed! LSP ready."`
6. Works

**Steps**: 3
**Time**: 30 seconds (automated)

---

## Maintainability Comparison

### Scenario: Bug in Root Finding Logic

#### Current Approach

**Bug**: Root finder doesn't check parent directories correctly

**Fix required**:
1. Fix in `rust.rs::find_cargo_root()` (10 lines)
2. Fix in `python.rs::find_python_root()` (10 lines)
3. Fix in `javascript.rs` (doesn't have root finder - uses parent, needs implementation)
4. Fix in `java.rs::find_jvm_project_root()` (20 lines, more complex)

**Files changed**: 4
**Lines changed**: ~40
**Test coverage**: Need tests for each language

#### Proposed Approach

**Fix required**:
1. Fix in `language_config.rs::find_project_root()` (single function)

**Files changed**: 1
**Lines changed**: ~10
**Test coverage**: Single test suite covers all languages

---

## Performance Comparison

### Startup Time

#### Current
- No config parsing
- Instant

#### Proposed
- Parse embedded TOML (~50 languages): ~1ms
- Parse user TOML (if exists): ~0.5ms
- Build indices (HashMap): ~0.1ms

**Total overhead**: ~1.6ms (negligible)

### Runtime Detection

#### Current
```rust
// Extension matching via match statement: O(1) - compiler optimized
match extension {
    "rs" => Language::Rust,
    // ...
}
```

#### Proposed
```rust
// HashMap lookup: O(1) - same complexity
registry.by_extension.get("rs")  // → index
registry.languages[index]         // → config
```

**Performance**: Identical (both O(1) lookup)

---

## Migration Path

### Phase 1: Foundation (No Behavior Change)

**Changes**:
- Add `language_config.rs`
- Add `languages.toml` (embedded)
- Initialize registry in `main()`

**Backward compatibility**: 100% (registry exists but isn't used yet)

**Risk**: Very low (new code, no changes to existing paths)

### Phase 2: Refactor LSP Init (Hybrid Mode)

**Changes**:
- Refactor `lsp_init/mod.rs` to use registry
- Keep `java.rs` as special case
- Remove `rust.rs`, `python.rs`, `javascript.rs`

**Backward compatibility**: 100% (same LSPs start, unified logic)

**Risk**: Medium (behavior change, but well-tested)

### Phase 3: Add TypeScript Auto-Install

**Changes**:
- Add `auto_install` to TypeScript config
- Implement npm installer

**Backward compatibility**: 100% (opt-in feature)

**Risk**: Low (new feature, doesn't affect existing workflows)

### Phase 4: Cleanup & Document

**Changes**:
- Remove deprecated modules
- Add user documentation
- Add CLI introspection

**Backward compatibility**: 100% (public APIs unchanged)

**Risk**: Very low (documentation + convenience features)

---

## Summary

| Aspect | Current | Proposed | Improvement |
|--------|---------|----------|-------------|
| **Lines of code** (per language) | ~60 | ~15 | 75% less |
| **Files to edit** (add language) | 2+ | 1 | 50% less |
| **Recompilation required** | Yes | No | 100% faster |
| **Duplication** | High | None | Shared logic |
| **Error messages** | Generic | Helpful | Better UX |
| **Auto-install** | Java only | Any language | Universal |
| **Fallback search** | No | Yes | More robust |
| **User extensibility** | No | Yes | Empowering |
| **CLI introspection** | No | Yes | Debuggable |
| **Startup overhead** | 0ms | ~2ms | Negligible |

**Conclusion**: The declarative approach is:
- **Less code** (75% reduction per language)
- **More maintainable** (centralized logic)
- **More extensible** (users can add languages)
- **Better UX** (fallbacks, install hints, auto-install)
- **Same performance** (O(1) lookup, <2ms startup overhead)

The only tradeoff is ~300 lines of infrastructure code (`language_config.rs`), but this pays for itself after adding just 2-3 languages via config instead of code.
