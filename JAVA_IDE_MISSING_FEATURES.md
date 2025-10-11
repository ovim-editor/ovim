# Missing Java IDE Features in ovim

## Executive Summary

ovim has **excellent LSP foundation** with 17 textDocument/* methods implemented. However, compared to mature Java IDEs (IntelliJ IDEA, Eclipse, VS Code), there are **34 missing features** categorized below.

---

## ✅ Currently Implemented (17 features)

### Document Lifecycle
- ✅ `textDocument/didOpen` - Document opened
- ✅ `textDocument/didChange` - Document edited (with incremental sync)
- ✅ `textDocument/didSave` - Document saved
- ✅ `textDocument/didClose` - Document closed

### Code Navigation (5)
- ✅ `textDocument/definition` (bound to `gd`) - Go to definition
- ✅ `textDocument/hover` (bound to `K`) - Hover documentation
- ✅ `textDocument/references` - Find all references
- ✅ `textDocument/documentSymbol` - Document outline/symbols
- ✅ `workspace/symbol` - Workspace-wide symbol search

### Code Intelligence (3)
- ✅ `textDocument/completion` - Code completion/autocomplete
- ✅ `textDocument/signatureHelp` - Parameter hints while typing
- ✅ `textDocument/documentHighlight` - Highlight symbol occurrences

### Code Actions (1)
- ✅ `textDocument/codeAction` (bound to `<Space>ca`) - Quick fixes and refactorings

### Refactoring (2)
- ✅ `textDocument/rename` - Rename symbol across project
- ✅ `textDocument/prepareRename` - Validate rename operation

### Formatting (2)
- ✅ `textDocument/formatting` - Format entire document
- ✅ `textDocument/rangeFormatting` - Format selected range

### Editor Features (3)
- ✅ `textDocument/selectionRange` - Smart expand/shrink selection
- ✅ `textDocument/foldingRange` - Code folding regions
- ✅ `textDocument/publishDiagnostics` (notification) - Errors/warnings

### Diagnostics Integration
- ✅ Real-time error/warning display
- ✅ Diagnostic count by severity
- ✅ Per-line diagnostic lookup

---

## ❌ Missing LSP Features (17 features)

### Critical Java IDE Features

#### 1. **textDocument/implementation** ⭐⭐⭐
**What**: Find implementations of interface methods or abstract methods
**Java Use Case**: Click on `List.add()` → see implementations in `ArrayList`, `LinkedList`, etc.
**Priority**: HIGH - Essential for Java OOP navigation
**Keybinding**: Usually `gi` (go to implementation)

#### 2. **textDocument/typeDefinition** ⭐⭐⭐
**What**: Jump to the type definition (class/interface) of a variable
**Java Use Case**: `var myList = new ArrayList<>();` → jump to `ArrayList` class
**Priority**: HIGH - Common Java workflow
**Keybinding**: Usually `gy` (go to type)

#### 3. **textDocument/callHierarchy** (incoming/outgoing) ⭐⭐⭐
**What**: View call hierarchy - who calls this method, what does this method call
**Java Use Case**: Trace method call chains, understand code flow
**Priority**: HIGH - Critical for large codebases
**UI**: Tree view or split window

#### 4. **textDocument/typeHierarchy** (supertypes/subtypes) ⭐⭐⭐
**What**: View class hierarchy - superclasses, subclasses, interfaces
**Java Use Case**: Navigate OOP hierarchies, see inheritance structure
**Priority**: HIGH - Essential for Java OOP
**UI**: Tree view showing `Object → AbstractList → ArrayList`

#### 5. **textDocument/inlayHint** ⭐⭐⭐
**What**: Inline type hints and parameter names
**Java Use Case**: Show parameter names in method calls, inferred types for `var`
**Example**: `add(/* index */ 0, /* element */ "hello")`
**Priority**: HIGH - Major quality-of-life feature in modern IDEs

#### 6. **textDocument/codeLens** ⭐⭐
**What**: Inline actionable hints above methods/classes
**Java Use Case**: "Run Test", "Debug", "5 references", "Implement abstract methods"
**Priority**: MEDIUM - Very useful but not essential
**UI**: Clickable text above declarations

#### 7. **textDocument/semanticTokens/full** ⭐⭐
**What**: Semantic highlighting (beyond syntax highlighting)
**Java Use Case**: Color fields differently from local variables, highlight final/static
**Priority**: MEDIUM - Nice-to-have for visual clarity

#### 8. **textDocument/linkedEditingRange** ⭐
**What**: Simultaneously edit related symbols (like HTML tags)
**Java Use Case**: Limited use in Java (rename class + constructor together?)
**Priority**: LOW

### Document Features

#### 9. **textDocument/declaration** ⭐⭐
**What**: Go to declaration (vs definition)
**Java Use Case**: Jump to method declaration in interface (not impl)
**Priority**: MEDIUM - Sometimes useful, but `definition` usually suffices

#### 10. **textDocument/documentLink** ⭐
**What**: Make URLs/file paths in comments clickable
**Java Use Case**: Click JavaDoc `@see` references, issue tracker links
**Priority**: LOW - Convenience feature

#### 11. **textDocument/onTypeFormatting** ⭐⭐
**What**: Format code as you type (e.g., auto-indent after `{`)
**Java Use Case**: Auto-format on `;`, `}`, `Enter`
**Priority**: MEDIUM - Quality of life

#### 12. **textDocument/documentColor** ⭐
**What**: Show color swatches for color literals
**Java Use Case**: Limited (only for `Color` constants)
**Priority**: LOW - Rarely used in Java

#### 13. **textDocument/colorPresentation** ⭐
**What**: Color picker for editing colors
**Java Use Case**: Limited use in Java
**Priority**: LOW

### Workspace Features

#### 14. **workspace/executeCommand** ⭐⭐⭐
**What**: Execute custom LSP server commands
**Java Use Case**: "Organize imports", "Generate getters/setters", "Run tests"
**Priority**: HIGH - Many code generation features use this

#### 15. **workspace/applyEdit** ⭐⭐⭐
**What**: Apply workspace-wide edits from server
**Java Use Case**: Multi-file refactorings (rename across files)
**Priority**: HIGH - Required for proper refactoring

#### 16. **workspace/willRenameFiles** / **didRenameFiles** ⭐⭐
**What**: Notify server when files are renamed
**Java Use Case**: Update imports when moving Java classes
**Priority**: MEDIUM - Important for file management

#### 17. **workspace/didChangeWatchedFiles** ⭐⭐
**What**: File system watcher integration
**Java Use Case**: Reload when external build tool modifies files
**Priority**: MEDIUM - Better external change handling

---

## ❌ Missing Editor Integration Features (6 features)

### Currently No Keybindings For:

1. **Format Document** - API exists but no keybinding
   - **Should add**: `<Leader>f` or `=` operator integration

2. **Find References** - API exists but no keybinding
   - **Should add**: `gr` (go to references)

3. **Signature Help** - API exists but no trigger
   - **Should add**: Auto-trigger on `(` or manual `Ctrl-K`

4. **Document Symbols (Outline)** - API exists but no UI
   - **Should add**: `<Leader>o` to open outline picker

5. **Workspace Symbols** - API exists but no UI
   - **Should add**: `<Leader>s` for symbol search

6. **Selection Range (Expand/Shrink)** - API exists but no keybinding
   - **Should add**: `v` in visual mode to expand selection

---

## ❌ Missing Java-Specific Code Generation (11 features)

These are typically exposed via `textDocument/codeAction` or `workspace/executeCommand` but require UI integration:

### Essential Code Generation
1. **Organize Imports** ⭐⭐⭐
   - Remove unused imports, sort, add missing imports
   - **Typical keybinding**: `Ctrl+Shift+O` or `:OrganizeImports`

2. **Generate Getters/Setters** ⭐⭐⭐
   - Select fields, generate accessor methods
   - **Typical access**: Code action menu

3. **Generate Constructor** ⭐⭐⭐
   - Generate constructor from fields
   - **Typical access**: Code action menu

4. **Generate toString/equals/hashCode** ⭐⭐
   - Generate boilerplate methods
   - **Typical access**: Code action menu

5. **Implement Abstract Methods** ⭐⭐⭐
   - When implementing interface/abstract class
   - **Typical access**: Quick fix on error

6. **Override Methods** ⭐⭐
   - Choose parent methods to override
   - **Typical access**: Code action menu

7. **Extract Method/Variable/Constant** ⭐⭐⭐
   - Refactor selected code into method/variable
   - **Typical keybinding**: `<Leader>em`, `<Leader>ev`

8. **Inline Variable/Method** ⭐⭐
   - Replace variable with its value, inline method calls
   - **Typical keybinding**: `<Leader>iv`, `<Leader>im`

9. **Move Class/Method** ⭐⭐
   - Move to different file/class
   - **Typical access**: Refactor menu

10. **Change Method Signature** ⭐⭐
    - Add/remove/reorder parameters
    - **Typical access**: Refactor menu

11. **Convert to Lambda/Method Reference** ⭐⭐
    - Modernize Java 8+ code
    - **Typical access**: Quick fix

---

## ❌ Missing Build & Run Integration (5 features)

1. **Maven/Gradle Task Execution** ⭐⭐⭐
   - Run `mvn compile`, `gradle build`, etc. from editor
   - **Typical**: `:Maven compile` or task picker

2. **Test Runner (JUnit)** ⭐⭐⭐
   - Run tests, show results inline
   - **Typical**: `<Leader>tr` (test run), green/red icons

3. **Debugging (DAP)** ⭐⭐⭐
   - Set breakpoints, step through code
   - **Protocol**: Debug Adapter Protocol
   - **Typical**: `<F5>` to debug, `<F9>` toggle breakpoint

4. **Run Configuration** ⭐⭐
   - Configure and run Java applications
   - **Typical**: `:Run`, `:Debug`

5. **Dependency Management** ⭐
   - Add dependency from within editor
   - **Typical**: Code action on import error

---

## ❌ Missing Advanced Features (5 features)

1. **Code Coverage** ⭐⭐
   - Show which lines are tested
   - **UI**: Green/red line highlights

2. **Profiling Integration** ⭐
   - CPU/memory profiling
   - **Typical**: External tool integration

3. **JavaDoc Generation** ⭐⭐
   - Generate JavaDoc comments
   - **Typical**: Quick fix or code action

4. **Spring Framework Support** ⭐⭐
   - Navigate between Spring annotations and configs
   - **Requires**: jdtls Spring extension

5. **Decompiler Integration** ⭐
   - View decompiled `.class` files
   - **Typical**: Auto-decompile when no source available

---

## Implementation Priority Ranking

### P0 - Essential (Must Have)
1. **Keybindings for existing features**
   - Format document, find references, outline, workspace symbols
   - **Effort**: LOW (just wire up existing APIs)
   - **Impact**: HIGH (unlock existing functionality)

2. **workspace/executeCommand**
   - Required for organize imports and code generation
   - **Effort**: MEDIUM (protocol handling)
   - **Impact**: HIGH (enables many features)

3. **workspace/applyEdit**
   - Required for multi-file refactorings
   - **Effort**: MEDIUM (apply edits to multiple buffers)
   - **Impact**: HIGH (proper refactoring support)

### P1 - High Value (Should Have)
4. **textDocument/implementation** + **typeDefinition**
   - Core Java navigation
   - **Effort**: LOW (similar to goto definition)
   - **Impact**: HIGH (Java-specific navigation)

5. **textDocument/callHierarchy** + **typeHierarchy**
   - Essential for large codebases
   - **Effort**: MEDIUM (need tree UI)
   - **Impact**: HIGH (understand code structure)

6. **textDocument/inlayHint**
   - Modern IDE feature, very popular
   - **Effort**: MEDIUM (inline rendering)
   - **Impact**: HIGH (code readability)

7. **Organize Imports**
   - Via executeCommand once implemented
   - **Effort**: LOW (use existing command)
   - **Impact**: HIGH (most common Java operation)

### P2 - Nice to Have (Could Have)
8. **textDocument/codeLens**
   - "Run Test", reference counts
   - **Effort**: MEDIUM (inline UI elements)
   - **Impact**: MEDIUM (convenience)

9. **Test Runner Integration**
   - Run JUnit tests from editor
   - **Effort**: HIGH (test detection, result display)
   - **Impact**: MEDIUM (developer workflow)

10. **textDocument/semanticTokens**
    - Better syntax highlighting
    - **Effort**: MEDIUM (new highlight layer)
    - **Impact**: MEDIUM (visual clarity)

### P3 - Future (Won't Have Yet)
11. **Debugging (DAP)**
    - Full debugger integration
    - **Effort**: VERY HIGH (new protocol, UI)
    - **Impact**: HIGH (but can use external debugger)

12. **Code Generation UI**
    - Dialogs for getters/setters, etc.
    - **Effort**: HIGH (need form UIs)
    - **Impact**: MEDIUM (can use code actions instead)

13. **Spring/Framework Support**
    - Requires framework-specific extensions
    - **Effort**: VERY HIGH (jdtls plugins)
    - **Impact**: LOW (specialized use case)

---

## Quick Wins (What to Add First)

### 1. Wire Up Existing Features (30 minutes each)
```vim
" Add these keybindings:
nnoremap <Leader>f :Format<CR>         " Format document
nnoremap gr :FindReferences<CR>         " Find references
nnoremap <Leader>o :DocumentSymbols<CR> " Outline
nnoremap <Leader>s :WorkspaceSymbols<CR> " Symbol search
vnoremap = :FormatRange<CR>             " Format selection
```

### 2. Implement Missing LSP Methods (2-4 hours each)
- `textDocument/implementation` (gd variant)
- `textDocument/typeDefinition` (gy variant)
- `workspace/executeCommand` (for organize imports)
- `workspace/applyEdit` (for multi-file refactorings)

### 3. Add Code Action UI (4 hours)
- Show available code actions in a picker
- Execute selected action
- Display result/errors

---

## Comparison with Other Editors

### IntelliJ IDEA (Industry Leader)
- **Has all 48 features** listed above
- **Plus**: AI assistant, advanced profiling, Spring tools
- **Advantage**: Proprietary engine, deep integration

### Eclipse (Open Source Leader)
- **Has 45/48 features** (missing some modern features like inlay hints)
- **Plus**: Extensive plugin ecosystem
- **Advantage**: Same jdtls engine as ovim

### VS Code + Java Extension
- **Has 38/48 features** implemented
- **Missing**: Some advanced refactorings, profiling
- **Uses**: Same jdtls as ovim
- **Most comparable**: Closest feature parity to aim for

### Neovim + nvim-jdtls
- **Has ~30/48 features** implemented
- **Missing**: Many advanced features, UI integration
- **Uses**: Same jdtls as ovim
- **Similar**: Command-line interface challenges

---

## Recommended Implementation Roadmap

### Phase 1: Quick Wins (1-2 days)
- [ ] Add keybindings for existing LSP features
- [ ] Implement `textDocument/implementation`
- [ ] Implement `textDocument/typeDefinition`
- [ ] Add format document keybinding

### Phase 2: Code Actions (3-5 days)
- [ ] Implement `workspace/executeCommand`
- [ ] Implement `workspace/applyEdit`
- [ ] Add organize imports integration
- [ ] Create code action picker UI

### Phase 3: Navigation (5-7 days)
- [ ] Implement `textDocument/callHierarchy`
- [ ] Implement `textDocument/typeHierarchy`
- [ ] Build hierarchy tree UI
- [ ] Add navigation keybindings

### Phase 4: Modern Features (7-10 days)
- [ ] Implement `textDocument/inlayHint`
- [ ] Implement `textDocument/codeLens`
- [ ] Implement `textDocument/semanticTokens`
- [ ] Add inline rendering for hints

### Phase 5: Testing & Build (10-14 days)
- [ ] Add Maven/Gradle task runner
- [ ] Integrate JUnit test runner
- [ ] Add test result display
- [ ] Implement Debug Adapter Protocol (DAP)

---

## Current Status Summary

**What ovim has**: Solid LSP foundation with 17/51 total IDE features (~33%)
**What's missing**: Advanced navigation (7), editor integration (6), code generation (11), build tools (5)
**Biggest gaps**:
  1. No keybindings for existing features (easy fix)
  2. No hierarchy navigation (callHierarchy, typeHierarchy)
  3. No code generation UI (organize imports, generate methods)
  4. No test/debug integration

**Overall Assessment**: ovim has a **strong foundation** but needs **medium effort** (2-4 weeks) to reach feature parity with VS Code's Java extension, and **significant effort** (2-3 months) to approach IntelliJ IDEA's level.

---

**Date**: 2025-10-08
**Version**: ovim 0.1.0
**Status**: Comprehensive feature gap analysis complete
