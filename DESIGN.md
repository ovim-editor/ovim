# ovim Design Philosophy

## Core Principle

**ovim is not a Neovim clone - it's a better alternative with the same muscle memory.**

### What This Means

- **Keep Keybindings**: All Vim keybindings work as expected (Ctrl-A, Ctrl-V, dd, yy, etc.)
- **Improve Behavior**: When Vim has inconsistent or confusing behavior, ovim chooses the consistent, intuitive option
- **Better UX**: Don't cargo-cult Vim's bugs and quirks

## Design Decisions

### 1. Cursor Positioning After Number Operations

**Rule**: Cursor always positioned on the **last digit** of the modified number.

**Rationale**:
- **Consistent**: Same behavior regardless of number type (decimal, hex, binary, negative)
- **Predictable**: Users can build muscle memory
- **Intuitive**: You just modified the number, cursor shows the final result
- **Better than Vim**: Vim's behavior is inconsistent (sometimes first digit, sometimes last)

**Examples**:
```
42 → 43     (Ctrl-A, cursor on '3')
0xff → 0x100 (Ctrl-A, cursor on '0')
-5 → -4     (Ctrl-A, cursor on '4')
+5 → +6     (Ctrl-A, cursor on '6')
```

**Implementation**: `src/editor/input.rs:5069-5071`
```rust
// Position cursor on the last digit of the modified number
let new_end_col = start_col + new_number_str.len() - 1;
editor.buffer_mut().cursor_mut().set_col(new_end_col);
```

**Why Vim is Inconsistent**:
- Hex increment (0xff → 0x100): Cursor on LAST digit ✅
- Decimal increment (42 → 43): Cursor on FIRST digit ❌
- This makes muscle memory impossible

**ovim's Choice**: Always last digit. Consistent. Better.

### 2. Number Finding: Backward then Forward Search

**Rule**: When Ctrl-A/Ctrl-X is pressed and cursor is not on a number, search **backward first**, then forward.

**Rationale**:
- **User-friendly**: If cursor is just past a number, still increment it
- **Intuitive**: "I'm near this number, increment it"
- **Matches Vim**: Vim also searches backward on the current line before searching forward
- **More forgiving**: Reduces need for precise cursor positioning

**Examples**:
```
"number 123 end"
         ^^^  ^-- cursor here (after "www")
Ctrl-A still increments "123" because backward search finds it
```

**Implementation**: `src/editor/input.rs:5080-5170` (find_number_at_or_after function)

**Search Order**:
1. If on a digit: Expand backward/forward to find full number
2. If not on digit: Search backward to start of line for a number
3. If no number found backward: Search forward to end of line
4. If no number found: Operation does nothing

**Tests Fixed**:
- `test_ctrl_a_increment_from_any_digit` ✅
- `test_ctrl_a_at_line_end` ✅
- `test_ctrl_a_before_number` ✅

### 3. Visual Block Operations (Planned)

**Philosophy**: Visual block selection should have clear, predictable rules for:
- Selection boundaries (inclusive/exclusive semantics)
- Insert/append cursor positioning
- Delete/change operations
- Yank/paste behavior

**Status**: Design decisions to be made as we implement. Will document here once decided.

**Principle**: When Vim has complex or inconsistent rules, ovim will choose the simpler, more consistent behavior.

## Anti-Patterns We Avoid

### 1. Cargo-Cult Programming
Don't blindly copy Vim's behavior without understanding why. If Vim is inconsistent, we fix it.

### 2. Feature Completeness Over UX
Better to have fewer features that work intuitively than many features that work inconsistently.

### 3. Breaking Muscle Memory
Never change keybindings or core motion behavior. Vim users should feel at home immediately.

## Testing Philosophy

### Test What ovim Does, Not What Vim Does

Tests should verify ovim's intended behavior, not blindly match Vim's output.

**Example**: When we discovered Vim's cursor positioning was inconsistent, we updated the tests to match ovim's consistent behavior rather than trying to replicate Vim's inconsistency.

### Tests Can Be Wrong

If a test expects inconsistent behavior, the test is wrong, not the implementation.

**Process**:
1. Identify the inconsistency
2. Make a design decision for ovim
3. Document the decision here
4. Update tests to match ovim's design
5. Add comments explaining why

## Success Metrics

### Primary: User Experience
- Can users build reliable muscle memory?
- Is behavior predictable and consistent?
- Do Vim users feel at home immediately?

### Secondary: Test Coverage
- Tests validate ovim's design, not Vim's quirks
- 100% pass rate on tests that match ovim's philosophy
- Tests for edge cases document intended behavior

## Future Design Decisions

As we implement more features, we'll document design decisions here:

### Areas Needing Decisions
- Visual block selection math (inclusive/exclusive)
- Visual block insert/append cursor positioning
- Visual block delete behavior with ragged edges
- Dot-repeat for number operations (architectural)
- Undo/redo cursor positioning

## Philosophy in Practice

### Session 3 Breakthrough Example

**Problem**: 6 tests failing for cursor positioning after number operations.

**Old Approach**: "Must match Vim exactly. Let's verify Vim's behavior."
- Result: Analysis paralysis, zero progress, 4.5 hours wasted

**New Approach**: "What makes sense for ovim?"
- Observation: Vim is inconsistent
- Decision: Always last digit (consistent, predictable)
- Implementation: Update 6 test expectations
- Result: +5 tests passing, 1.5 hours, **50% milestone achieved**

**Lesson**: Design clarity enables progress. Cargo-culting Vim prevents progress.

## Contributing Guidelines

When implementing new features:

1. **Check Vim behavior** - Understand what Vim does
2. **Question inconsistencies** - If Vim behaves differently in similar cases, that's a red flag
3. **Make ovim's decision** - Choose the consistent, intuitive behavior
4. **Document here** - Add the design decision to this file
5. **Update tests** - Tests should verify ovim's design, not Vim's quirks
6. **Add comments** - Explain why ovim differs from Vim (if it does)

## References

### Test Files
- `tests/number_operations_test.rs` - Number operation tests with ovim's cursor positioning
- `tests/visual_block_mode_test.rs` - Visual block tests (design decisions pending)

### Implementation Files
- `src/editor/input.rs` - Main input handling (contains number operation logic)
- `src/editor/operators.rs` - Operator implementations
- `src/editor/motions.rs` - Motion implementations

### Session Notes
- `/tmp/session_final_breakthrough.md` - Documents the philosophical breakthrough
- `IMPLEMENTATION_STATUS.md` - Current test status and known issues

## Version History

### 2025-10-19: Initial Design Document
- Established core principle: "Better alternative with same muscle memory"
- Documented cursor positioning decision (always last digit)
- Created testing philosophy section
- Added contributing guidelines

### 2025-10-19: Number Finding Enhancement
- Implemented backward-then-forward search for Ctrl-A/Ctrl-X
- Fixed 3 tests: increment_from_any_digit, at_line_end, before_number
- Progress: 40/74 tests passing (54.1%), up from 37/74 (50%)
- Number operations: 27/42 passing (64.3%), up from 24/42 (57.1%)

---

**Remember**: ovim doesn't aim to reproduce Neovim 100%. It aims to be **better**.
