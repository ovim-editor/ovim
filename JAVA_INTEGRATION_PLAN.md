# Java Integration Plan - Fix Syntax Highlighting & LSP Recognition

**Status:** ✅ COMPLETED - All issues resolved and tests passing

## Issues Found

### Issue #1: "Language not supported for LSP"
**Location:** `src/editor/mod.rs` - 5 different functions

```rust
// In goto_definition, hover, completion, format, code_actions:
let language_id = if file_path.ends_with(".rs") {
    "rust"
} else if file_path.ends_with(".js") || file_path.ends_with(".ts") {
    "javascript"
} else if file_path.ends_with(".py") {
    "python"
} else {
    self.set_lsp_status("Language not supported for LSP".to_string());
    return Ok(false);
};
```

**Problem:** Java (`.java`) is not in the list! Each LSP function manually checks file extensions.

### Issue #2: Syntax Highlighting Not Working
**Location:** `src/buffer/mod.rs:280`

```rust
pub fn enable_syntax_highlighting(&mut self) {
    if let Some(ref path) = self.file_path {
        if let Some(lang) = LanguageRegistry::detect_from_path(path) {
            if let Ok(mut highlighter) = SyntaxHighlighter::new(lang) {
                // ... works!
            }
        }
    }
}
```

**Analysis:**
- ✅ `LanguageRegistry::detect_from_path()` correctly detects `.java` → `Language::Java`
- ✅ `SyntaxHighlighter::new(Language::Java)` should work with our `java.scm`
- ⚠️  Need to verify the syntax highlighting is actually enabled when loading Java files

## Root Cause

The code has **hardcoded file extension checks** in multiple places instead of using a centralized language detection function. This is technical debt that causes:
1. Duplication (5 places check extensions)
2. Easy to miss when adding new languages
3. Inconsistency between modules

## Solution Architecture

### Design Principle
**Use centralized language detection everywhere.**

```
LanguageRegistry
    ↓
detect_from_path(".java") → Language::Java
    ↓
    ├─> Syntax highlighting ✓ (already works)
    ├─> LSP language_id     ✗ (needs fix)
    └─> All LSP operations  ✗ (needs fix)
```

### New Helper Function

```rust
// In src/syntax/languages.rs or src/editor/mod.rs
impl LanguageRegistry {
    /// Get LSP language ID from file path
    pub fn get_lsp_language_id(file_path: &str) -> Option<&'static str> {
        Self::detect_from_path(file_path).and_then(|lang| {
            match lang {
                Language::Rust => Some("rust"),
                Language::JavaScript => Some("javascript"),
                Language::Python => Some("python"),
                Language::Java => Some("java"),
            }
        })
    }
}
```

## Implementation Plan

### Phase 1: Add Helper Function ✅
- [x] Add `get_lsp_language_id()` to `LanguageRegistry`
- [x] Test with all supported languages
- [x] Unit tests for mapping

### Phase 2: Update Editor LSP Functions ✅
Update 5 functions in `src/editor/mod.rs`:

1. **`goto_definition()`** (line ~1309)
2. **`hover()`** (line ~1418)
3. **`completion()`** (line ~1492)
4. **`format_document()`** (line ~1576)
5. **`code_actions()`** (line ~1647)

Replace hardcoded checks with:
```rust
let language_id = match LanguageRegistry::get_lsp_language_id(file_path) {
    Some(id) => id,
    None => {
        self.set_lsp_status("Language not supported for LSP".to_string());
        return Ok(false);
    }
};
```

### Phase 3: Verify Syntax Highlighting ✅
- [x] Test Java file opens with highlighting
- [x] Verify `java.scm` queries work
- [x] Check performance (no regression)

### Phase 4: Integration Test ✅
- [x] Open Java file
- [x] Verify syntax highlighting appears
- [x] Press `gd` → should work (not "Language not supported")
- [x] Press `K` → should work
- [x] Check completion works
- [x] Check diagnostics appear

### Phase 5: Documentation ✅
- [x] Update CLAUDE.md with Java LSP support
- [x] Document the centralized language detection pattern
- [x] Add troubleshooting for syntax highlighting

## Detailed Changes

### File: `src/syntax/languages.rs`

**Add:**
```rust
impl LanguageRegistry {
    /// Get LSP language identifier from file path
    /// Returns None if language is not supported by LSP
    pub fn get_lsp_language_id(file_path: &str) -> Option<&'static str> {
        Self::detect_from_path(file_path).and_then(|lang| {
            match lang {
                Language::Rust => Some("rust"),
                Language::JavaScript => Some("javascript"),
                Language::Python => Some("python"),
                Language::Java => Some("java"),
            }
        })
    }

    /// Check if a file path has LSP support
    pub fn has_lsp_support(file_path: &str) -> bool {
        Self::get_lsp_language_id(file_path).is_some()
    }
}
```

### File: `src/editor/mod.rs`

**Replace in 5 locations:**

**Before:**
```rust
let language_id = if file_path.ends_with(".rs") {
    "rust"
} else if file_path.ends_with(".js") || file_path.ends_with(".ts") {
    "javascript"
} else if file_path.ends_with(".py") {
    "python"
} else {
    self.set_lsp_status("Language not supported for LSP".to_string());
    return Ok(false);
};
```

**After:**
```rust
let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
    Some(id) => id,
    None => {
        self.set_lsp_status("Language not supported for LSP".to_string());
        return Ok(false);
    }
};
```

## Benefits

### 1. Single Source of Truth
- Language detection in one place
- Easy to add new languages
- No duplication

### 2. Consistency
- LSP and syntax highlighting use same logic
- Guaranteed to stay in sync
- Type-safe (enum-based)

### 3. Maintainability
- Add language once, works everywhere
- Clear ownership (`LanguageRegistry`)
- Easy to test

### 4. Extensibility
```rust
// Future: Add Kotlin
Language::Kotlin => Some("kotlin"),

// Future: Multiple file extensions
".kt" | ".kts" => Language::Kotlin
```

## Testing Strategy

### Unit Tests
```rust
#[test]
fn test_lsp_language_id_mapping() {
    assert_eq!(
        LanguageRegistry::get_lsp_language_id("test.java"),
        Some("java")
    );
    assert_eq!(
        LanguageRegistry::get_lsp_language_id("test.rs"),
        Some("rust")
    );
    assert_eq!(
        LanguageRegistry::get_lsp_language_id("test.txt"),
        None
    );
}
```

### Integration Tests
1. Open `FileData.java`
2. Verify status line shows "Java: Ready ✓" (not "Language not supported")
3. Press `gd` on a class name → should work
4. Press `K` on a method → should show hover
5. Type incomplete code → should show completions
6. Verify syntax highlighting (keywords, strings, comments)

### Manual Tests
```bash
# Test Java
echo "public class Test {}" > Test.java
ovim Test.java
# Press gd, K, etc.

# Test existing languages still work
ovim test.rs   # Rust
ovim test.py   # Python
ovim test.js   # JavaScript
```

## Rollout Plan

### Step 1: Add Helper Function
- Low risk
- No breaking changes
- Can test independently

### Step 2: Update LSP Functions
- Medium risk
- Test each function
- Regression test other languages

### Step 3: Verify & Test
- High confidence
- Full integration test
- Performance check

### Step 4: Document
- Update docs
- Add examples
- Troubleshooting guide

## Timeline

- **Phase 1 (Helper):** 5 minutes
- **Phase 2 (LSP):** 10 minutes
- **Phase 3 (Verify):** 5 minutes
- **Phase 4 (Test):** 10 minutes
- **Phase 5 (Docs):** 5 minutes

**Total:** ~35 minutes

## Success Criteria

✅ Java files show syntax highlighting
✅ `gd` works on Java files
✅ `K` works on Java files
✅ Completions work
✅ No regression on other languages
✅ Code is cleaner (no duplication)
✅ Easy to add more languages

## Risk Mitigation

### Risk: Break existing languages
**Mitigation:** Test all 4 languages (Rust, JS, Python, Java)

### Risk: Performance regression
**Mitigation:** Benchmark syntax highlighting, should be identical

### Risk: Edge cases with file paths
**Mitigation:** Use existing `detect_from_path()` which handles this

## Future Improvements

### Phase 6: Kotlin Support
- Add `Language::Kotlin`
- Add LSP mapping
- Same pattern, no duplication

### Phase 7: Auto-enable Syntax
- Enable syntax highlighting on file load automatically
- No manual call needed

### Phase 8: LSP Capabilities Query
```rust
impl LanguageRegistry {
    pub fn get_lsp_capabilities(lang: Language) -> LspCapabilities {
        match lang {
            Language::Java => LspCapabilities {
                completion: true,
                hover: true,
                goto_definition: true,
                formatting: true,
                code_actions: true,
            },
            // ...
        }
    }
}
```

## Summary

**Problem:** Java not recognized for LSP operations, causing "Language not supported" errors and potential syntax highlighting issues.

**Solution:** Centralize language detection in `LanguageRegistry` and use it consistently across all modules.

**Impact:** Clean architecture, easy maintenance, full Java support.

**Effort:** ~35 minutes, low risk, high value.

**Can you dig it?** Yes. This is the right way. ✨

---

**Status:** Ready to implement
**Version:** ovim 0.1.0
**Date:** 2025-10-07
