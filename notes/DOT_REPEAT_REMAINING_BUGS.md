# Dot-Repeat Remaining Bugs - Root Cause Analysis

## Status

- **Total tests:** 47
- **Passing:** 43 (91%)
- **Failing:** 4

## Remaining Failing Tests

### 1. test_dot_repeat_R_command

**Test:**
```rust
// Input: "hello world"
// Keys: R + "HI" + Esc + w.
// Expected: "HIllo HIrld\n"
```

**Actual behavior:** Only inserts partial text ("I" instead of "HI") on repeat.

**Root cause:** Replace mode creates a Composite change with individual character replacements. When repeated, only the last character replacement is being applied instead of the full sequence.

**Location:** `src/editor/input/mod.rs` (Replace mode handling) and `src/editor/change.rs` (Composite repeat logic)

---

### 2. test_dot_repeat_ci_quote

**Test:**
```rust
// Input: `"hello" and "world"`
// Keys: ci" + "X" + Esc + f".
// Expected: `"X" and "world"\n`
```

**Actual behavior:** Inserts duplicate text instead of changing content.

**Root cause:** The `ci"` operation records absolute positions for the delete and insert. When repeated at a different location, it uses those same absolute positions instead of re-evaluating "inner quotes" at the new cursor position.

**Location:** `src/editor/input/mod.rs` (text object handling) and `src/editor/change.rs`

---

### 3. test_dot_repeat_di_paren

**Test:**
```rust
// Input: "func(arg1) and func(arg2)"
// Keys: di( + f( + .
// Expected: "func() and func()\n"
```

**Actual behavior:** Partially deletes content, corrupting text.

**Root cause:** Same as ci" - the `di(` operation records absolute positions. On repeat, it deletes at those absolute positions instead of finding "inner parentheses" at the new cursor location.

**Location:** `src/editor/input/mod.rs` (text object handling)

---

### 4. test_dot_repeat_cw_at_different_word_lengths

**Test:**
```rust
// Input: "short longerword"
// Keys: cw + "X" + Esc + w.
// Expected: "X X\n"
```

**Actual behavior:** Only changes partial word on repeat.

**Root cause:** The `cw` operation records the exact character range deleted. When repeated on a longer word ("longerword"), it only deletes the same number of characters as the first word ("short" = 5 chars), leaving partial text.

**Location:** `src/editor/operators.rs` and `src/editor/change.rs`

---

## Architectural Problem

### Current Design

The `Change` enum tracks changes with absolute positions:

```rust
pub enum Change {
    InsertText { position: (usize, usize), text: String, ... },
    DeleteText { range: Range, deleted_text: String, ... },
    Composite { changes: Vec<Change>, ... },
    // etc.
}
```

When `repeat()` is called, it applies the same change at the same (or adjusted) absolute positions.

### What Vim Does

Vim commands are **semantic** - they operate relative to context:
- `cw` = "change word" (whatever word is under cursor)
- `ci"` = "change inner quotes" (find quotes around cursor, change content)
- `di(` = "delete inner parens" (find parens around cursor, delete content)

When repeated with `.`, Vim re-evaluates the semantic operation at the new cursor position.

### Required Fix

Option 1: **Store semantic operation type**
```rust
pub enum Change {
    // ... existing variants ...

    // New semantic variants
    ChangeWord { replacement: String, ... },
    ChangeTextObject { object_type: TextObjectType, replacement: String, ... },
    DeleteTextObject { object_type: TextObjectType, ... },
}
```

Option 2: **Store operation metadata**
```rust
pub struct Change {
    // ... existing fields ...
    semantic_type: Option<SemanticOperation>,
}

pub enum SemanticOperation {
    ChangeWord,
    ChangeInner(char),  // ci", ci(, etc.
    DeleteInner(char),  // di", di(, etc.
    // etc.
}
```

The `repeat()` method would then:
1. Check if change has semantic metadata
2. If yes, re-evaluate the semantic operation at current cursor
3. If no, fall back to positional repeat

---

## Files to Modify

1. **`src/editor/change.rs`**
   - Add semantic change variants or metadata
   - Update `repeat()` to handle semantic operations

2. **`src/editor/input/mod.rs`**
   - Update text object handlers to create semantic changes
   - Update `cw` handler to create semantic change

3. **`src/editor/operators.rs`**
   - Update operator implementations to support semantic tracking

4. **`src/editor/text_objects.rs`** (if exists)
   - May need to expose text object finding logic for repeat

---

## Implementation Steps

1. Design the semantic change representation
2. Implement `ChangeWord` semantic repeat
3. Test with `test_dot_repeat_cw_at_different_word_lengths`
4. Implement `ChangeTextObject` for `ci"`, `ci(`, etc.
5. Test with `test_dot_repeat_ci_quote`
6. Implement `DeleteTextObject` for `di"`, `di(`, etc.
7. Test with `test_dot_repeat_di_paren`
8. Fix R command Composite repeat logic
9. Test with `test_dot_repeat_R_command`

---

## References

- Vim documentation on repeat: https://vimdoc.sourceforge.net/htmldoc/repeat.html
- Current change implementation: `src/editor/change.rs`
- Text object implementation: `src/editor/input/mod.rs` (search for "text object")
