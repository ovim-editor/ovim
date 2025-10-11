# Java IDE Support in ovim

## Overview

ovim now has comprehensive Java support powered by Eclipse JDT.LS (jdtls), providing IDE-like features for Java development. This implementation takes inspiration from leading IDEs like IntelliJ IDEA, Eclipse, and VS Code's Java extensions.

## Features

### Language Server Protocol (LSP) Integration

ovim integrates with **Eclipse JDT Language Server (jdtls)**, the same language server used by VS Code's Java extension, providing:

- **Code Completion**: Intelligent autocomplete with type inference
- **Go to Definition**: Navigate to method/class definitions (bound to `gd` in Normal mode)
- **Hover Information**: View documentation and type information (bound to `K` in Normal mode)
- **Diagnostics**: Real-time error and warning detection
- **Code Actions**: Quick fixes and refactorings
- **Find References**: Locate all usages of a symbol
- **Rename**: Refactor symbol names across the project
- **Document Formatting**: Auto-format Java code
- **Signature Help**: View method signatures while typing

### Project Detection

ovim automatically detects Java project types and finds the project root:

- **Maven Projects**: Detects `pom.xml` in parent directories
- **Gradle Projects**: Detects `build.gradle`, `build.gradle.kts`, `settings.gradle`, or `settings.gradle.kts`

The LSP server is initialized with the correct project root, ensuring proper classpath resolution and dependency management.

### Syntax Highlighting

Tree-sitter-based syntax highlighting for Java with support for:

- Keywords (class, interface, enum, package, import, etc.)
- Method declarations and invocations
- Class and interface declarations
- Type identifiers (including generics and arrays)
- Primitive types
- Field declarations and access
- Parameters and local variables
- String and character literals
- Numeric literals (decimal, hex, octal, binary)
- Comments (line and block)
- Annotations
- Operators and punctuation
- this/super keywords
- Package and import statements

## Usage

### Opening a Java File

Simply open a Java file:

```bash
ovim MyClass.java
```

ovim will:
1. Detect the file extension (.java)
2. Find the project root (Maven/Gradle)
3. Start jdtls language server
4. Initialize LSP with the project root
5. Provide syntax highlighting and LSP features

### Prerequisites

**Eclipse JDT.LS (jdtls)** must be installed and available in your PATH.

#### Installation Options:

1. **Manual Installation**:
   - Download from http://download.eclipse.org/jdtls/snapshots/
   - Extract and add to PATH

2. **Linux Package Managers**:
   - Some distributions provide jdtls packages
   - Check your package manager (apt, dnf, pacman)

3. **Homebrew (macOS)**:
   ```bash
   brew install jdtls
   ```

4. **From Source**:
   - Clone https://github.com/eclipse-jdtls/eclipse.jdt.ls
   - Build following repository instructions

### System Requirements

- **Java Runtime**: JDT.LS requires Java 21+ to run
- **Project Compatibility**: Supports Java projects from 1.8 through 24

### LSP Features in Action

#### Go to Definition
Position cursor on a method/class name and press `gd` in Normal mode.

#### Hover Documentation
Position cursor on a symbol and press `K` in Normal mode.

#### Code Actions
Access via LSP commands (implementation-specific key bindings).

#### Find References
Use LSP references command to find all usages.

## Architecture

### LSP Integration Flow

1. **File Detection**: ovim detects `.java` files
2. **Project Root Discovery**: Searches parent directories for Maven/Gradle markers
3. **LSP Server Startup**: Spawns jdtls with project root as workspace folder
4. **Initialization**: Handshake with LSP server, capability negotiation
5. **Document Sync**: Sends `textDocument/didOpen` notification
6. **Real-time Updates**: Syncs changes via `textDocument/didChange`
7. **Feature Requests**: Handles LSP requests (completion, hover, goto definition)

### Project Root Detection Logic

```rust
fn find_jvm_project_root(file_path: &Path) -> &Path {
    // Search parent directories for:
    // 1. pom.xml (Maven)
    // 2. build.gradle or build.gradle.kts (Gradle)
    // 3. settings.gradle or settings.gradle.kts (Gradle multi-module)
    //
    // Returns first match or falls back to file's parent directory
}
```

## Comparison with Leading IDEs

### IntelliJ IDEA
- **ovim**: Uses jdtls for LSP features
- **IntelliJ**: Proprietary language engine with deeper integration
- **Common**: Both provide code completion, navigation, refactoring

### Eclipse
- **ovim**: Uses Eclipse's jdtls language server
- **Eclipse**: Same underlying technology (Eclipse JDT compiler)
- **Common**: Identical diagnostics and completion quality

### VS Code (with Java extensions)
- **ovim**: Direct jdtls integration
- **VS Code**: Also uses jdtls under the hood
- **Common**: Nearly identical feature set and capabilities

## Future Enhancements

### Kotlin Support (Planned)
Kotlin support was researched and partially implemented but deferred due to tree-sitter version conflicts:
- tree-sitter-kotlin uses older tree-sitter API
- Will be added once tree-sitter ecosystem stabilizes
- Alternative: Use kotlin-language-server directly without tree-sitter

### Additional JVM Languages
- **Scala**: tree-sitter-scala exists but needs integration
- **Groovy**: Limited tree-sitter support
- **Clojure**: Separate language server (clojure-lsp)

### Advanced IDE Features
- **Debugging**: Debug Adapter Protocol (DAP) integration
- **Testing**: JUnit test runner integration
- **Build Tools**: Direct Maven/Gradle command execution
- **Refactoring**: Enhanced code actions and transformations
- **Code Generation**: Generate getters, setters, constructors
- **Import Organization**: Auto-organize and optimize imports

## Technical Details

### Dependencies Added

```toml
[dependencies]
tree-sitter = "0.23"
tree-sitter-java = "0.23"
lsp-types = "0.95"
```

### Files Modified

- `src/syntax/languages.rs`: Added Java language enum and detection
- `src/syntax/queries/java.scm`: Java syntax highlighting queries
- `src/main.rs`: Added jdtls configuration and project root detection
- `Cargo.toml`: Added tree-sitter-java dependency

### Key Functions

```rust
// Project root detection
fn find_jvm_project_root(file_path: &Path) -> &Path

// LSP initialization
async fn initialize_lsp_for_file(editor: &mut Editor, file_path: &str)

// Language detection
fn detect_from_extension(extension: &str) -> Option<Language>
```

## Troubleshooting

### jdtls not found
- Ensure jdtls is installed and in PATH
- Check `which jdtls` or `where jdtls` (Windows)

### LSP not starting
- Check file extension is `.java`
- Verify project has pom.xml or build.gradle
- Check LSP status in editor status line

### No completions/diagnostics
- Ensure Java 21+ is installed for jdtls
- Wait for jdtls initialization (can take several seconds)
- Check project builds successfully with mvn/gradle

### Tree-sitter syntax highlighting issues
- Verify `src/syntax/queries/java.scm` exists
- Check for query syntax errors
- Tree-sitter grammar version: 0.23

## Contributing

To enhance Java support:

1. **Add new LSP features**: Extend LSP manager in `src/lsp/mod.rs`
2. **Improve syntax highlighting**: Edit `src/syntax/queries/java.scm`
3. **Add key bindings**: Modify `src/editor/input.rs`
4. **Test with real projects**: Use Maven/Gradle projects for testing

## Resources

- [Eclipse JDT.LS](https://github.com/eclipse-jdtls/eclipse.jdt.ls)
- [LSP Specification](https://microsoft.github.io/language-server-protocol/)
- [tree-sitter Java](https://github.com/tree-sitter/tree-sitter-java)
- [Maven](https://maven.apache.org/)
- [Gradle](https://gradle.org/)

---

**Status**: ✅ Java IDE support fully implemented and tested
**Version**: ovim 0.1.0
**Date**: 2025-10-07
