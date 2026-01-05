# Language Support Architecture Diagrams

## Current Architecture (Hardcoded)

```
┌─────────────────────────────────────────────────────────────┐
│                        File Open Event                       │
│                     (user opens file.ts)                     │
└───────────────────────────┬─────────────────────────────────┘
                            │
                            v
┌─────────────────────────────────────────────────────────────┐
│              lsp_init::initialize_lsp_for_file()            │
│                                                              │
│  let extension = get_extension(path);                       │
│  match extension {                                           │
│    "rs"  => rust::initialize_rust_lsp(...),                 │
│    "py"  => python::initialize_python_lsp(...),             │
│    "js"  => javascript::initialize_javascript_lsp(...),     │
│    "ts"  => javascript::initialize_javascript_lsp(...),     │
│    _     => (),  // No support!                             │
│  }                                                           │
└───────────────────────────┬─────────────────────────────────┘
                            │
            ┌───────────────┼───────────────┐
            v               v               v
    ┌───────────┐   ┌───────────┐   ┌───────────┐
    │ rust.rs   │   │ python.rs │   │javascript │
    │           │   │           │   │   .rs     │
    │ 54 lines  │   │ 40 lines  │   │ 46 lines  │
    └───────────┘   └───────────┘   └───────────┘
            │               │               │
            v               v               v
    ┌───────────────────────────────────────────┐
    │      Duplicate Logic Per Language:        │
    │  - find_project_root() [custom per lang]  │
    │  - start_server() call                    │
    │  - error handling                         │
    │  - status updates                         │
    └───────────────────────────────────────────┘

Problems:
  ❌ High duplication (root finding, error handling)
  ❌ No reuse across languages
  ❌ Adding language = new file + code changes
  ❌ No fallback search for LSP binaries
  ❌ Generic error messages
  ❌ Can't extend without recompiling
```

---

## Proposed Architecture (Config-Driven)

```
┌─────────────────────────────────────────────────────────────┐
│                     Application Startup                      │
└───────────────────────────┬─────────────────────────────────┘
                            │
                            v
┌─────────────────────────────────────────────────────────────┐
│              LanguageRegistry::init()                       │
│                                                              │
│  1. Load embedded config (languages.toml)                   │
│  2. Load user config (~/.config/ovim/languages.toml)        │
│  3. Merge configs (user overrides embedded)                 │
│  4. Build indices (extension→lang, filename→lang)           │
│  5. Store in global singleton (OnceLock)                    │
└───────────────────────────┬─────────────────────────────────┘
                            │
                            v
                    ┌───────────────┐
                    │ Ready! (~2ms) │
                    └───────────────┘

┌─────────────────────────────────────────────────────────────┐
│                        File Open Event                       │
│                     (user opens file.ts)                     │
└───────────────────────────┬─────────────────────────────────┘
                            │
                            v
┌─────────────────────────────────────────────────────────────┐
│         lsp_init::initialize_lsp_for_file() [UNIFIED]       │
│                                                              │
│  // 1. Detect language                                      │
│  let config = LanguageRegistry::get().detect(path)?;        │
│                                                              │
│  // 2. Check if LSP configured                              │
│  let lsp_config = config.lsp?;                              │
│                                                              │
│  // 3. Find LSP binary (primary + fallbacks)                │
│  let command = find_lsp_command(&lsp_config)?;              │
│                                                              │
│  // 4. Find project root (using markers)                    │
│  let root = find_project_root(path, &lsp_config.markers);   │
│                                                              │
│  // 5. Start server                                         │
│  lsp_manager.start_server(&config.id, &command, root);      │
└───────────────────────────┬─────────────────────────────────┘
                            │
            ┌───────────────┼───────────────┐
            │               │               │
            v               v               v
    ┌─────────────┐ ┌─────────────┐ ┌─────────────┐
    │  Syntax      │ │   Command   │ │    Root     │
    │  Highlighting│ │   Search    │ │   Finding   │
    └─────────────┘ └─────────────┘ └─────────────┘
            │               │               │
            v               v               v
    ┌───────────────────────────────────────────────┐
    │         Shared Logic (All Languages)          │
    │  - find_lsp_command() [tries fallbacks]       │
    │  - find_project_root() [checks markers]       │
    │  - show_install_hint() [on failure]           │
    │  - attempt_auto_install() [if configured]     │
    └───────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│                  Configuration Source                        │
│                                                              │
│  languages.toml (embedded) + ~/.config/ovim/languages.toml  │
│                                                              │
│  [[language]]                                                │
│  id = "typescript"                                           │
│  extensions = ["ts", "tsx"]                                  │
│  [language.lsp]                                              │
│    command = "typescript-language-server"                   │
│    fallback_commands = ["node_modules/.bin/..."]            │
│    root_markers = ["package.json", "tsconfig.json"]         │
│    install_hint = "npm install -g typescript-language-..."  │
│    auto_install = { type = "npm", package = "..." }         │
└─────────────────────────────────────────────────────────────┘

Benefits:
  ✅ Single code path (all languages use same logic)
  ✅ Shared root finding, command search, error handling
  ✅ Adding language = edit TOML (no recompile)
  ✅ Fallback search (tries multiple locations)
  ✅ Helpful errors (install hints, searched paths)
  ✅ User extensible (config override)
  ✅ Auto-install support (npm, cargo, etc.)
```

---

## Data Flow: Language Detection

### Current Approach

```
┌────────────┐
│ "file.ts"  │
└─────┬──────┘
      │
      v
┌─────────────────────┐
│ get_extension()     │
│ → "ts"              │
└──────┬──────────────┘
       │
       v
┌─────────────────────┐
│ match extension {   │
│   "ts" => ...       │  ← Hardcoded!
│ }                   │
└──────┬──────────────┘
       │
       v
┌─────────────────────────────┐
│ javascript::initialize_...() │
└─────────────────────────────┘
```

### Proposed Approach

```
┌────────────┐
│ "file.ts"  │
└─────┬──────┘
      │
      v
┌───────────────────────────────┐
│ LanguageRegistry::detect()    │
│                                │
│ 1. Extract extension: "ts"    │
│ 2. Lookup in HashMap:         │
│    by_extension.get("ts")     │  ← O(1) lookup
│    → index 2                  │
│ 3. Return: languages[2]       │
│    → LanguageConfig {         │
│        id: "typescript",      │
│        lsp: LspConfig {...}   │
│      }                        │
└────────┬──────────────────────┘
         │
         v
┌────────────────────────┐
│ Unified init function  │
│ (same for all langs)   │
└────────────────────────┘
```

**Advantage**: Extension mapping is data (TOML), not code (match statement).

---

## LSP Command Discovery Flow

### Current Approach

```
┌──────────────────────────┐
│ "typescript-language-    │
│  server"                 │
└────────┬─────────────────┘
         │
         v
┌──────────────────────────┐
│ Spawn process            │  ← Assumes it's in PATH!
└────────┬─────────────────┘
         │
    ┌────┴────┐
    v         v
┌───────┐ ┌──────────────────────┐
│Success│ │Fail: No such file... │  ← Generic error!
└───────┘ └──────────────────────┘
```

### Proposed Approach

```
┌─────────────────────────────────────┐
│ LspConfig {                         │
│   command: "typescript-language-    │
│            server",                 │
│   fallback_commands: [              │
│     "node_modules/.bin/...",        │
│     "~/.npm-global/bin/...",        │
│   ]                                 │
│ }                                   │
└────────┬────────────────────────────┘
         │
         v
┌────────────────────────────┐
│ find_lsp_command()         │
│                            │
│ 1. Try: which command      │  → Not found
│ 2. Try: fallback[0]        │  → Not found
│ 3. Try: fallback[1]        │  → Not found
│ 4. Return: None            │
└────────┬───────────────────┘
         │
         v
┌────────────────────────────┐
│ Show install hint:         │
│ "TypeScript LSP not found. │
│  Install with: npm install │
│  -g typescript-language-   │
│  server"                   │
│                            │
│ Searched:                  │
│  ✗ typescript-language-... │
│  ✗ node_modules/.bin/...   │
│  ✗ ~/.npm-global/bin/...   │
└────────┬───────────────────┘
         │
         v
┌────────────────────────────┐
│ Offer auto-install:        │
│ "Press <Leader>li to       │
│  auto-install"             │
└────────────────────────────┘
```

**Advantages**:
- Tries multiple locations (robust)
- Shows exactly what was searched (educational)
- Offers install instructions (helpful)
- Can auto-install if configured (convenient)

---

## Project Root Finding Flow

### Current Approach (Per-Language)

```rust
// rust.rs
fn find_cargo_root(path: &Path) -> &Path {
    loop {
        if dir.join("Cargo.toml").exists() {  ← Hardcoded!
            return dir;
        }
    }
}

// python.rs
fn find_python_root(path: &Path) -> &Path {
    loop {
        if dir.join("pyproject.toml").exists() {  ← Hardcoded!
            return dir;
        }
    }
}

// javascript.rs
fn find_js_root(path: &Path) -> &Path {
    path.parent()  ← Doesn't even search!
}
```

### Proposed Approach (Unified)

```rust
fn find_project_root(path: &Path, markers: &[String]) -> PathBuf {
    let mut current = path.parent();

    while let Some(dir) = current {
        for marker in markers {  ← Config-driven!
            if dir.join(marker).exists() {
                return dir.to_path_buf();
            }
        }
        current = dir.parent();
    }

    // Fallback
    path.parent().unwrap_or("/").to_path_buf()
}
```

```toml
# Configuration
[[language]]
id = "rust"
[language.lsp]
root_markers = ["Cargo.toml"]

[[language]]
id = "python"
[language.lsp]
root_markers = ["pyproject.toml", "setup.py", "requirements.txt"]

[[language]]
id = "typescript"
[language.lsp]
root_markers = ["package.json", "tsconfig.json", "node_modules"]
```

**Advantages**:
- Single function for all languages (no duplication)
- Priority order (tries markers in sequence)
- Config-driven (add markers without code changes)

---

## Auto-Install Flow (New Feature)

```
┌─────────────────────────────────┐
│ User opens file.ts              │
│ (typescript-language-server     │
│  not installed)                 │
└────────┬────────────────────────┘
         │
         v
┌─────────────────────────────────┐
│ find_lsp_command() → None       │
└────────┬────────────────────────┘
         │
         v
┌─────────────────────────────────┐
│ Check: auto_install configured? │
└────────┬────────────────────────┘
         │
    ┌────┴────┐
    v         v
┌────────┐ ┌─────────────────────────┐
│  No    │ │  Yes                    │
│        │ │  LspConfig {            │
│ Show   │ │    auto_install: Some(  │
│ hint   │ │      AutoInstallConfig {│
└────────┘ │        method: Npm {    │
           │          package: "..." │
           │        }                │
           │      }                  │
           │    )                    │
           │  }                      │
           └────┬────────────────────┘
                │
                v
┌─────────────────────────────────┐
│ Prompt user:                    │
│ "TypeScript LSP not found.      │
│  Auto-install? (y/n)"           │
└────────┬────────────────────────┘
         │
    ┌────┴────┐
    v         v
┌────────┐ ┌─────────────────────────┐
│  No    │ │  Yes                    │
└────────┘ └────┬────────────────────┘
                │
                v
┌─────────────────────────────────┐
│ Run: npm install -g typescript- │
│      language-server            │
│                                 │
│ Show progress:                  │
│ "Installing TypeScript LSP..."  │
└────────┬────────────────────────┘
         │
    ┌────┴────┐
    v         v
┌────────┐ ┌─────────────────────────┐
│ Failed │ │ Success                 │
│        │ │                         │
│ Show   │ │ "Installed! Reloading   │
│ error  │ │  LSP..."                │
└────────┘ └────┬────────────────────┘
                │
                v
┌─────────────────────────────────┐
│ Re-run find_lsp_command()       │
│ → Success!                      │
│                                 │
│ Start LSP server                │
└─────────────────────────────────┘
```

**Advantages**:
- One-click installation (better UX)
- Generalizable (works for npm, cargo, etc.)
- User consent (prompts before installing)

---

## Configuration Override Flow

```
┌─────────────────────────────────────────┐
│         Embedded languages.toml         │
│         (shipped with binary)           │
│                                         │
│  [[language]]                           │
│  id = "rust"                            │
│  extensions = ["rs"]                    │
│  [language.lsp]                         │
│    command = "rust-analyzer"            │
└───────────────┬─────────────────────────┘
                │
                v
┌─────────────────────────────────────────┐
│      User's ~/.config/ovim/languages.   │
│      toml (optional override)           │
│                                         │
│  [[language]]                           │
│  id = "rust"                            │
│  [language.lsp]                         │
│    command = "/custom/path/rust-        │
│               analyzer"                 │
│    args = ["--custom-flag"]             │
└───────────────┬─────────────────────────┘
                │
                v
┌─────────────────────────────────────────┐
│           Merge Strategy:               │
│  - User config overrides embedded      │
│    (by language ID)                     │
│  - User can add new languages          │
│  - User can modify LSP config          │
└───────────────┬─────────────────────────┘
                │
                v
┌─────────────────────────────────────────┐
│          Final Configuration:           │
│                                         │
│  LanguageConfig {                       │
│    id: "rust",                          │
│    extensions: ["rs"],                  │
│    lsp: LspConfig {                     │
│      command: "/custom/path/rust-       │
│                analyzer",  ← User value │
│      args: ["--custom-flag"], ← User    │
│    }                                    │
│  }                                      │
└─────────────────────────────────────────┘
```

**Use cases**:
- User has custom LSP installation
- User wants to try experimental LSP
- User needs custom args for project
- User wants to disable LSP for a language

---

## Performance Comparison

### Startup Time

```
Current:
┌──────────────┐
│ main()       │  0ms - no config loading
└──────────────┘

Proposed:
┌──────────────┐
│ main()       │
└──────┬───────┘
       v
┌──────────────────────────┐
│ LanguageRegistry::init() │
│                          │
│ - Parse embedded TOML    │  ~1ms
│ - Parse user TOML        │  ~0.5ms (if exists)
│ - Build indices          │  ~0.1ms
│ - Total:                 │  ~2ms
└──────────────────────────┘

Overhead: 2ms (negligible)
```

### Language Detection

```
Current:
┌──────────────┐
│ "file.rs"    │
└──────┬───────┘
       v
┌──────────────────────┐
│ match extension {    │  O(1) - compiler optimized
│   "rs" => Rust,      │
│   "py" => Python,    │
│   ...                │
│ }                    │
└──────────────────────┘
Time: ~1ns (hash + jump table)

Proposed:
┌──────────────┐
│ "file.rs"    │
└──────┬───────┘
       v
┌──────────────────────────┐
│ by_extension.get("rs")   │  O(1) - HashMap lookup
│ → index                  │
└──────┬───────────────────┘
       v
┌──────────────────────────┐
│ languages[index]         │  O(1) - array access
└──────────────────────────┘
Time: ~5ns (hash + lookup + deref)

Overhead: ~4ns (negligible)
```

**Conclusion**: Performance is identical - both O(1) lookup.

---

## Error Handling Comparison

### Current

```
┌──────────────────────────┐
│ LSP spawn fails          │
└──────┬───────────────────┘
       v
┌──────────────────────────────────────┐
│ Error: No such file or directory     │  ← Generic!
│        (os error 2)                  │
└──────────────────────────────────────┘

User thinks: "What file? What's error 2?"
```

### Proposed

```
┌──────────────────────────┐
│ LSP spawn fails          │
└──────┬───────────────────┘
       v
┌──────────────────────────────────────┐
│ find_lsp_command() → None            │
└──────┬───────────────────────────────┘
       v
┌──────────────────────────────────────┐
│ TypeScript LSP not found.            │  ← Helpful!
│                                      │
│ Install with:                        │
│   npm install -g typescript-         │
│   language-server typescript         │
│                                      │
│ Searched:                            │
│   ✗ typescript-language-server       │
│   ✗ node_modules/.bin/typescript-... │
│   ✗ ~/.npm-global/bin/typescript-... │
│                                      │
│ Auto-install: Press <Leader>li      │
└──────────────────────────────────────┘

User thinks: "Ah, I need to install it via npm!"
```

**Advantages**:
- Clear problem statement
- Exact install command
- Shows what was searched (educational)
- Offers auto-install (convenient)

---

## Summary: Why This Architecture Is Better

| Aspect | Current | Proposed | Why |
|--------|---------|----------|-----|
| **Code Duplication** | High | None | Unified functions |
| **Adding Language** | 2+ files, 60 lines | 1 file, 15 lines | Config-driven |
| **User Extensibility** | None | Full | Config override |
| **Error Messages** | Generic | Specific | Context-aware |
| **Command Discovery** | PATH only | Fallbacks | Robust search |
| **Root Finding** | Per-language | Unified | Shared logic |
| **Auto-Install** | Java only | Any language | Generalized |
| **Performance** | O(1) | O(1) | Same complexity |
| **Startup Overhead** | 0ms | 2ms | Negligible |
| **Maintainability** | 370 lines | 450 lines total | But supports ∞ languages |

**Key insight**: Slight increase in infrastructure code (300 lines for registry) pays for itself after adding just 2-3 languages via config instead of code. The architecture scales to unlimited languages with no code changes.

---

**Diagrams Created**: 9 visual diagrams showing current vs proposed architecture, data flows, error handling, and performance characteristics.
