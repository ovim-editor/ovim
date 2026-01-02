# Dot Repeat Implementation Bugs

## Summary
All 8 dot_repeat tests now have CORRECT expectations based on verified Vim behavior. The tests are failing because ovim has bugs in its dot repeat implementation.

## Bug Details

### 1. test_dot_repeat_upper_case_X
**Input:** "hello"
**Keys:** `$X.`
**Expected (Vim):** "heo\n" with cursor at (0, 1)
**Actual (ovim):** "hel\n"
**Bug:** ovim's dot repeat of X command is deleting wrong characters or not properly tracking cursor position

### 2. test_dot_repeat_o_command
**Input:** "line 1\nline 2"
**Keys:** `o` + "new" + Esc + `j.`
**Expected (Vim):** "line 1\nnew\nline 2\nnew\n" with cursor at (3, 2)
**Actual (ovim):** "line 1\nnew\nli\nnewne 2\n"
**Bug:** ovim's dot repeat of `o` command is inserting text in wrong place, corrupting existing content

### 3. test_dot_repeat_O_command
**Input:** "line 1\nline 2"
**Keys:** `O` + "new" + Esc + `j.`
**Expected (Vim):** "new\nnew\nline 1\nline 2\n" with cursor at (1, 2)
**Actual (ovim):** "new\nli\nnewne 1\nline 2\n"
**Bug:** ovim's dot repeat of `O` command is inserting text in wrong place, corrupting existing content

### 4. test_dot_repeat_R_command
**Input:** "hello world"
**Keys:** `R` + "HI" + Esc + `w.`
**Expected (Vim):** "HIllo HIrld\n" with cursor at (0, 7)
**Actual (ovim):** "HIllo Iorld\n"
**Bug:** ovim's dot repeat of Replace mode is only inserting partial text ("I" instead of "HI")

### 5. test_dot_repeat_ci_quote
**Input:** `"hello" and "world"`
**Keys:** `f"ci"` + "X" + Esc + `f".`
**Expected (Vim):** `"X" and "world"\n` with cursor at (0, 2)
**Actual (ovim):** `"hello"XX"world"\n`
**Bug:** ovim's dot repeat of `ci"` is inserting "XX" instead of changing the content inside quotes. The first `ci"` is not working correctly.

### 6. test_dot_repeat_di_paren
**Input:** "func(arg1) and func(arg2)"
**Keys:** `f(di(f(.`
**Expected (Vim):** "func() and func()\n" with cursor at (0, 20)
**Actual (ovim):** "func() and func2)\n"
**Bug:** ovim's dot repeat of `di(` is partially deleting content, corrupting the text ("arg2)" → "2)")

### 7. test_dot_repeat_cw_at_different_word_lengths
**Input:** "short longerword"
**Keys:** `cw` + "X" + Esc + `w.`
**Expected (Vim):** "X X\n" with cursor at (0, 2)
**Actual (ovim):** "X Xrword\n"
**Bug:** ovim's dot repeat of `cw` is only changing partial word ("longe" instead of "longerword")

## Pattern Analysis

**Common themes:**
1. **Partial text insertion/deletion** - Several bugs show incomplete operations (R command, cw, di()
2. **Text corruption** - o/O commands and text objects are corrupting existing content instead of clean edits
3. **Recording issues** - The dot repeat seems to be recording incomplete or wrong operations

**Root cause hypotheses:**
1. The last change recording might not be capturing the full operation
2. Text object operations might not be properly recorded for dot repeat
3. Insert mode operations (o, O, R) might have issues with position tracking during replay
4. The "length" of the change might be hardcoded instead of being operation-based

## Next Steps

To fix these bugs, investigate:
1. `src/editor/mod.rs` - `last_change` field and how it's populated
2. `src/editor/input/commands.rs` - How operations record themselves for dot repeat
3. Text object implementation - How `ci"`, `di(` etc. record their changes
4. Insert mode tracking - How `o`, `O`, `R` record what was inserted

## Test Status

**Passing:** 1/8
- test_dot_repeat_r_command ✓

**Failing:** 7/8
- test_dot_repeat_upper_case_X
- test_dot_repeat_o_command
- test_dot_repeat_O_command
- test_dot_repeat_R_command
- test_dot_repeat_ci_quote
- test_dot_repeat_di_paren
- test_dot_repeat_cw_at_different_word_lengths

All failing tests now have CORRECT expectations. Once the implementation bugs are fixed, these tests should pass.
