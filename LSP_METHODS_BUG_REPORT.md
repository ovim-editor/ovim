# LSP Integration Methods - Bug Report

## Executive Summary
Critical data loss bugs found in all newly implemented LSP methods. Location/symbol data from LSP responses is discarded and replaced with meaningless indices, making navigation completely non-functional.

---

## Bug #1: Complete Location Data Loss in find_references_impl()
**Severity:** CRITICAL  
**Location:** `/workspace/src/editor/mod.rs:1811-1902`  
**Lines:** 1881-1897

### Description
The `find_references_impl()` method receives complete location data from the LSP server (including full file paths, line numbers, and columns) but **discards all of it** when creating the picker. The picker stores only the **display string** (like "file.rs:10:5") and loses the actual file path needed for navigation.

### Root Cause
```rust
// Line 1881-1891: Creates display-only strings
let items: Vec<String> = locations
    .iter()
    .map(|loc| {
        let file_path = loc.uri.to_file_path().ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        let line = loc.range.start.line + 1;
        let col = loc.range.start.character + 1;
        format!("{}:{}:{}", file_path, line, col)  // ONLY filename, not full path!
    })
    .collect();

// Line 1897: Passes to new_custom which loses all location data
let picker = crate::editor::Picker::new_custom(base_dir, items);
```

The `new_custom()` method (picker.rs:75-96) then replaces the location with:
```rust
location: idx.to_string(), // Use index as location identifier
line: idx,
col: 0,
```

### Impact
When a user selects a reference from the picker, the code at `input.rs:3146-3163` tries to:
```rust
let location = result.location.clone();  // Gets "0", "1", "2" etc instead of file path!
let line = result.line;                  // Gets index, not actual line number!
let col = result.col;                    // Gets 0, not actual column!

if let Err(e) = editor.load_file(&location) {  // Tries to load file "0"!
    eprintln!("Failed to load file {}: {}", location, e);
}
```

**Result:** Complete failure to navigate to any reference. User sees "Failed to load file 0" or similar errors.

### Reproduction
1. Open a Java/TypeScript file with LSP enabled
2. Place cursor on a symbol used in multiple places
3. Press the find references key binding
4. Select any reference from the picker
5. **Expected:** Jump to selected reference location
6. **Actual:** Error "Failed to load file 0" (or whichever index was selected)

### Fix Required
Store the actual `Location` objects and retrieve them on selection. Options:
1. Create a separate storage for location data indexed by the picker item index
2. Encode full path + line + col in the location string that can be parsed back
3. Change picker to support typed data instead of just strings

---

## Bug #2: Complete Symbol Data Loss in document_symbols_impl()
**Severity:** CRITICAL  
**Location:** `/workspace/src/editor/mod.rs:1905-1987`  
**Lines:** 1970-1982

### Description
Identical bug to #1. Document symbols (functions, classes, methods) contain location data (line, column, range) but all of it is discarded.

### Root Cause
```rust
// Line 1970-1976: Creates display strings, loses actual line/col data
let items: Vec<String> = symbols
    .iter()
    .map(|sym| {
        let line = sym.range.start.line + 1;
        format!("{} ({:?}:{})", sym.name, sym.kind, line)
    })
    .collect();

// Line 1982: Same new_custom() call that loses all data
let picker = crate::editor::Picker::new_custom(base_dir, items);
```

### Impact
- User cannot navigate to any symbol from document outline
- Selecting a symbol tries to load file "0", "1", etc.
- Even if it worked, it would jump to index 0, 1, 2 instead of the actual line numbers

### Reproduction
1. Open a Java/TypeScript file with multiple functions
2. Request document symbols (outline)
3. Select any symbol from picker
4. **Expected:** Jump to symbol definition in the current file
5. **Actual:** Error or wrong location

---

## Bug #3: Complete Symbol Data Loss in workspace_symbols_impl()
**Severity:** CRITICAL  
**Location:** `/workspace/src/editor/mod.rs:1990-2059`  
**Lines:** 2039-2054

### Description
Workspace symbols contain both file URI and position data. Both are discarded.

### Root Cause
```rust
// Line 2039-2048: Extracts FILENAME ONLY, loses full path!
let items: Vec<String> = symbols
    .iter()
    .map(|sym| {
        let file_name = sym.location.uri.to_file_path().ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        let line = sym.location.range.start.line + 1;
        format!("{} - {} ({:?}:{})", sym.name, file_name, sym.kind, line)
    })
    .collect();
```

### Impact
- Cannot navigate to any workspace symbol
- Same file loading failure as Bug #1

---

## Bug #4: Complete Call Hierarchy Data Loss in call_hierarchy_incoming_impl()
**Severity:** CRITICAL  
**Location:** `/workspace/src/editor/mod.rs:2062-2171`  
**Lines:** 2150-2166

### Description
Incoming call hierarchy shows who calls the current function. Each call has complete location data (file URI, line, column, range). All discarded.

### Root Cause
```rust
// Line 2150-2160: Only extracts filename, loses path
let items: Vec<String> = calls
    .iter()
    .map(|call| {
        let name = &call.from.name;
        let file_path = call.from.uri.to_file_path().ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        let line = call.from.range.start.line + 1;
        format!("{} - {}:{}", name, file_path, line)
    })
    .collect();
```

### Impact
Same navigation failure pattern.

---

## Bug #5: Complete Call Hierarchy Data Loss in call_hierarchy_outgoing_impl()
**Severity:** CRITICAL  
**Location:** `/workspace/src/editor/mod.rs:2174-2283`  
**Lines:** 2262-2278

### Description
Outgoing call hierarchy shows which functions the current function calls. Same data loss bug.

### Root Cause
```rust
// Line 2262-2272: Same pattern - filename only, no full path
let items: Vec<String> = calls
    .iter()
    .map(|call| {
        let name = &call.to.name;
        let file_path = call.to.uri.to_file_path().ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        let line = call.to.range.start.line + 1;
        format!("{} - {}:{}", name, file_path, line)
    })
    .collect();
```

---

## Bug #6: Complete Type Hierarchy Data Loss in type_hierarchy_impl()
**Severity:** CRITICAL  
**Location:** `/workspace/src/editor/mod.rs:2286-2406`  
**Lines:** 2367-2401

### Description
Type hierarchy shows supertypes and subtypes. Each has full location data. All discarded.

### Root Cause
```rust
// Line 2370-2378: Supertypes - filename only
for super_type in supers {
    let name = &super_type.name;
    let file_path = super_type.uri.to_file_path().ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string());
    let line = super_type.range.start.line + 1;
    all_types.push(format!("↑ {} - {}:{}", name, file_path, line));
}

// Line 2381-2389: Subtypes - same bug
for sub_type in subs {
    let name = &sub_type.name;
    let file_path = sub_type.uri.to_file_path().ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string());
    let line = sub_type.range.start.line + 1;
    all_types.push(format!("↓ {} - {}:{}", name, file_path, line));
}
```

---

## Bug #7: Resource Leak - Locations Vector Not Cleaned Up
**Severity:** MEDIUM  
**Location:** All methods  

### Description
The original `locations` or `symbols` vectors from LSP are dropped after string formatting, but they contain `Url` objects and potentially large strings that should be explicitly moved or reused rather than cloned-then-dropped.

### Impact
Unnecessary memory allocation and GC pressure, especially for large result sets (e.g., 1000+ references).

---

## Bug #8: Missing Error Context in File Path Conversion
**Severity:** LOW  
**Location:** All methods  

### Description
When `uri.to_file_path()` fails, code uses `.unwrap_or_else(|| "unknown".to_string())` which hides why the conversion failed. The error is silently swallowed.

### Code Pattern
```rust
.and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
.unwrap_or_else(|| "unknown".to_string());
```

### Impact
- Debugging failures is difficult
- User sees "unknown" without understanding what went wrong
- LSP might be returning valid non-file URIs (e.g., jar:// for Java stdlib) that should be handled differently

---

## Bug #9: Potential Integer Overflow in Line Number Conversion
**Severity:** LOW  
**Location:** All methods  

### Description
Code converts LSP line numbers (u32) to display format with:
```rust
let line = loc.range.start.line + 1;  // u32 + 1 could overflow at u32::MAX
```

### Impact
Extremely unlikely in practice (would need 4 billion line file), but represents unsafe arithmetic that could panic in debug mode.

### Fix
```rust
let line = loc.range.start.line.saturating_add(1);
```

---

## Bug #10: Inconsistent Error Handling Between Methods
**Severity:** LOW  
**Location:** All methods  

### Description
Some methods use `try_lock()` and return `Err(anyhow::anyhow!("LSP busy"))` on lock failure, but there's no consistent retry mechanism or user feedback about the busy state.

### Code Pattern
```rust
let lsp_guard = match lsp.try_lock() {
    Ok(guard) => guard,
    Err(_) => {
        return Err(anyhow::anyhow!("LSP busy"));
    }
};
```

### Impact
- User might see transient failures during LSP initialization (especially Java with jdtls)
- No indication to user that they should retry
- Inconsistent with other LSP methods that might block

---

## Bug #11: Missing Jump List Management in Picker Navigation
**Severity:** MEDIUM  
**Location:** `/workspace/src/editor/input.rs:3146-3163`  

### Description
When navigating from a picker (references, symbols, etc.), the code at `input.rs:3157-3163` does NOT add the current position to the jump list before loading a new file, unlike `goto_implementation_impl()` and `goto_type_impl()` which properly save positions at lines 1653-1655 and 1767-1769.

### Impact
User cannot use Ctrl-O to return to previous position after navigating from picker results.

### Fix
Before `editor.load_file(&location)` in input.rs, add:
```rust
let current_line = editor.buffer().cursor().line();
let current_col = editor.buffer().cursor().col();
editor.jump_list_mut().add_jump(current_line, current_col);
```

---

## Bug #12: Unsafe unwrap() in Call Hierarchy Methods
**Severity:** MEDIUM  
**Location:** 
- `/workspace/src/editor/mod.rs:2133` (call_hierarchy_incoming_impl)
- `/workspace/src/editor/mod.rs:2245` (call_hierarchy_outgoing_impl)  
- `/workspace/src/editor/mod.rs:2357` (type_hierarchy_impl)

### Description
After checking that `items` is non-empty, code uses `.unwrap()` to get first item:

```rust
let items = match items {
    Some(items) if !items.is_empty() => items,
    _ => {
        drop(lsp_guard);
        self.set_lsp_status("No call hierarchy item at cursor".to_string());
        return Ok(false);
    }
};

// Get incoming calls for the first item
let first_item = items.into_iter().next().unwrap();  // Line 2133 - UNSAFE!
```

### Root Cause
The check confirms items is non-empty, but `into_iter().next()` consumes the iterator. While this *should* never fail, it's unnecessary unsafe code.

### Impact
- Panic if the Vec implementation changes or is corrupted
- Code analysis tools flag as unsafe pattern

### Fix
```rust
let first_item = items.into_iter().next()
    .ok_or_else(|| anyhow::anyhow!("Empty items vector after non-empty check"))?;
```

Or more idiomatically:
```rust
let mut items_iter = items.into_iter();
let first_item = items_iter.next()
    .ok_or_else(|| anyhow::anyhow!("Expected at least one item"))?;
```

---

## Summary Statistics

| Severity | Count | Impact |
|----------|-------|--------|
| CRITICAL | 6 | Complete feature non-functionality |
| MEDIUM | 3 | Resource leaks, missing UX features |
| LOW | 3 | Hidden errors, edge cases |

**Total Bugs Found:** 12

---

## Recommended Fix Priority

### P0 (Immediate - Blocks All Features)
1. Bug #1-6: Fix data loss in picker creation
   - Implement proper location data storage/retrieval mechanism
   - Affects: find_references, document_symbols, workspace_symbols, call_hierarchy (both), type_hierarchy

### P1 (High - Affects User Experience)
2. Bug #11: Add jump list management to picker navigation
3. Bug #12: Remove unsafe unwrap() calls in hierarchy methods

### P2 (Medium - Code Quality)
4. Bug #7: Optimize memory usage in location data handling
5. Bug #10: Improve error handling consistency

### P3 (Low - Edge Cases & Observability)
6. Bug #8: Add proper error context for URI conversion failures
7. Bug #9: Use saturating arithmetic for line number conversion

---

## Suggested Implementation Approach

### Option 1: Store Location Data Separately (Recommended)
Add a field to Editor struct:
```rust
/// Stores actual location data for custom picker items
/// Indexed by the picker item's line field (which stores the index)
lsp_picker_locations: Vec<lsp_types::Location>,
```

When creating picker:
```rust
// Store actual locations
self.lsp_picker_locations = locations.clone();

// Create display items
let items: Vec<String> = locations.iter().enumerate()
    .map(|(idx, loc)| {
        // Format for display, but index is stored in PickerResult.line
        format!("{}:{}:{}", filename, line, col)
    })
    .collect();
```

When selecting:
```rust
if picker_mode == PickerMode::Custom {
    let index = result.line;  // This is the index we stored
    if let Some(location) = editor.lsp_picker_locations().get(index) {
        // Use actual location data to navigate
        let target_path = location.uri.to_file_path()?;
        let target_line = location.range.start.line as usize;
        let target_col = location.range.start.character as usize;
        
        editor.load_file(&target_path)?;
        editor.buffer_mut().cursor_mut().set_position(target_line, target_col);
    }
}
```

### Option 2: Encode Location in String (Simpler but Less Efficient)
Store full path in location field:
```rust
location: format!("{}:{}:{}", 
    loc.uri.to_file_path()?.to_string_lossy(),
    loc.range.start.line,
    loc.range.start.character),
```

Parse on selection:
```rust
let parts: Vec<&str> = location.split(':').collect();
if parts.len() >= 3 {
    let path = parts[0..parts.len()-2].join(":");
    let line = parts[parts.len()-2].parse()?;
    let col = parts[parts.len()-1].parse()?;
    // Navigate...
}
```

**Issue with Option 2:** Parsing ambiguity with Windows paths (C:\path\to\file.rs:10:5)

---

## Testing Recommendations

1. **Unit Tests:** Create tests for each LSP method with mock location data
2. **Integration Tests:** Test navigation from picker with actual LSP server
3. **Edge Cases:** 
   - Files with same name in different directories
   - Very long file paths (>256 chars)
   - Non-UTF8 file paths
   - Network URIs (file://, jar://, etc.)
4. **Regression Tests:** Ensure fix doesn't break existing file picker or live grep

---

## Additional Notes

- The goto_implementation_impl() and goto_type_impl() methods do NOT have these bugs because they handle single-location results and navigate directly without using a picker
- The existing file finder and live grep pickers work correctly because they use different modes (FindFiles, LiveGrep) and populate location data properly
- This bug suggests insufficient testing of the custom picker mode with actual location data
