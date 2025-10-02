# Supported File Extensions for Syntax Highlighting

ovim uses tree-sitter for syntax highlighting and supports a comprehensive list of file extensions.

## Currently Supported Languages

### Rust
**Extensions:** `.rs`

**Example files:**
- `main.rs`
- `lib.rs`
- `mod.rs`

---

### JavaScript / TypeScript
**Extensions:** `.js`, `.jsx`, `.mjs`, `.cjs`, `.es`, `.es6`, `.es7`, `.ts`, `.tsx`, `.mts`, `.cts`

**Special filenames:**
- `Jakefile`
- `Gulpfile.js`
- `Gruntfile.js`
- `webpack.config.js`
- `rollup.config.js`
- `.eslintrc.js`
- `.prettierrc.js`

**Example files:**
- `index.js`
- `App.jsx`
- `module.mjs` (ES modules)
- `script.cjs` (CommonJS)
- `types.ts` (TypeScript)
- `Component.tsx` (TypeScript + JSX)

---

### Python
**Extensions:** `.py`, `.pyw`, `.pyi`, `.pyx`, `.pxd`, `.pxi`, `.pyc`, `.pyd`, `.pyo`, `.pyz`, `.pywz`, `.py3`, `.pyde`, `.pyt`, `.snakefile`, `.smk`

**Special filenames:**
- `Pipfile`
- `Pipfile.lock`
- `Snakefile`
- `wscript`
- `SConstruct`
- `.pythonstartup`
- `.pythonrc`
- Files starting with `.python`

**Example files:**
- `main.py`
- `script.pyw` (Windows Python without console)
- `types.pyi` (Python type stubs)
- `module.pyx` (Cython)
- `Pipfile` (Python dependencies)

---

## Extension Details

### Case Insensitivity
All extensions are matched case-insensitively, so `.RS`, `.Py`, `.JS` will work.

### Special File Detection
ovim checks both file extension and full filename to detect language, which means files like `Pipfile` (no extension) are still highlighted correctly.

### Detection Algorithm
1. First tries to match file extension
2. If no match, tries to match full filename
3. If still no match, checks for common patterns (e.g., files starting with `.python`)

## Future Language Support

Additional languages can be added by:
1. Adding tree-sitter grammar dependency to `Cargo.toml`
2. Adding language variant to `Language` enum
3. Adding extensions to `detect_from_extension()`
4. Adding highlight query file to `src/syntax/queries/`

### Commonly Requested Languages
- **C/C++**: `.c`, `.cpp`, `.cc`, `.cxx`, `.h`, `.hpp`, `.hxx`
- **Go**: `.go`
- **Java**: `.java`, `.class`
- **Ruby**: `.rb`, `.rake`, `Rakefile`, `Gemfile`
- **PHP**: `.php`, `.phtml`, `.php3`, `.php4`, `.php5`
- **Shell**: `.sh`, `.bash`, `.zsh`, `.fish`, `.bashrc`, `.zshrc`
- **HTML/CSS**: `.html`, `.htm`, `.css`, `.scss`, `.sass`, `.less`
- **Markdown**: `.md`, `.markdown`
- **JSON/YAML**: `.json`, `.yml`, `.yaml`, `.toml`
- **SQL**: `.sql`, `.mysql`, `.pgsql`

## Testing File Detection

You can test file detection by opening files with different extensions:

```bash
# Rust
cargo run -- src/main.rs

# JavaScript
cargo run -- index.js
cargo run -- App.tsx

# Python
cargo run -- script.py
cargo run -- Pipfile
```

The status line will show the filename, and syntax colors will appear automatically if the language is detected.
