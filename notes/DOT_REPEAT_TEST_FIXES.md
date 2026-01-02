# Dot Repeat Test Expectation Fixes

## Summary
Fixed expectations for 8 dot_repeat tests based on verified Vim behavior. All tests now have CORRECT expectations. Tests are failing due to ovim implementation bugs (documented in DOT_REPEAT_BUGS.md).

## Test Fixes Applied

### 1. test_dot_repeat_upper_case_X
**Before:** Expected "hello\n" (unchanged - WRONG)
**After:** Expected "heo\n" with cursor at (0, 1) - CORRECT
**Vim behavior:** `$` positions on 'o', `X` deletes 'l' leaving "helo", `.` deletes 'l' leaving "heo"

### 2. test_dot_repeat_o_command
**Before:** Expected "new\nlinewne 1\nline 2\n" (gibberish - WRONG)
**After:** Expected "line 1\nnew\nline 2\nnew\n" with cursor at (3, 2) - CORRECT
**Vim behavior:** `o` opens line after line 1, `.` opens line after line 2

### 3. test_dot_repeat_O_command
**Before:** Expected "new\nlinewne 1\nline 2\n" (gibberish - WRONG)
**After:** Expected "new\nnew\nline 1\nline 2\n" with cursor at (1, 2) - CORRECT
**Vim behavior:** `O` opens line before line 1, `j` moves down, `.` opens line before current line

### 4. test_dot_repeat_r_command
**Before:** Expected "hello\nworld\n" (wrong test input and expectation - WRONG)
**After:** Expected "XXllo world\n" with cursor at (0, 1) - CORRECT
**Vim behavior:** `r` replaces h→X, `l` moves right, `.` replaces e→X
**Status:** ✓ PASSING (ovim implementation correct for this case)

### 5. test_dot_repeat_R_command
**Before:** Expected "hello\nworld\n" (wrong test input and expectation - WRONG)
**After:** Expected "HIllo HIrld\n" with cursor at (0, 7) - CORRECT
**Changes:** Also changed input from "hello\nworld" to "hello world" (single line)
**Vim behavior:** `R` replaces "he"→"HI", `w` moves to "world", `.` replaces "wo"→"HI"

### 6. test_dot_repeat_ci_quote
**Before:** Expected `"hello" and "world"\n` (unchanged - WRONG)
**After:** Expected `"X" and "world"\n` with cursor at (0, 2) - CORRECT
**Vim behavior:** `f"` finds opening quote, `ci"` changes to "X", `f"` finds CLOSING quote of "X" (not opening of "world"), so dot repeat doesn't affect "world"

### 7. test_dot_repeat_di_paren
**Before:** Expected "func(arg1) and func(arg2)\n" (unchanged - WRONG)
**After:** Expected "func() and func()\n" with cursor at (0, 20) - CORRECT
**Vim behavior:** `f(` finds first paren, `di(` deletes "arg1", `f(` finds second paren, `.` deletes "arg2"

### 8. test_dot_repeat_cw_at_different_word_lengths
**Before:** Expected "Xreally Xlong short\n" with input "a really long short" - WRONG
**After:** Expected "X X\n" with input "short longerword" - CORRECT
**Changes:** Simplified input from "a really long short" to "short longerword" to match description
**Vim behavior:** `cw` changes "short"→"X", `w` moves to "longerword", `.` changes "longerword"→"X"

## Verification Method

All expectations were verified using Neovim v0.11.1 with `-u NONE` (no config) to test vanilla Vim behavior:

```bash
# Example verification for test_dot_repeat_R_command
echo "hello world" | vim -u NONE -es '+normal! RHI' '+normal! w.' '+%p' '+q!' /dev/stdin
# Output: HIllo HIrld
```

Complete verification script: `/tmp/test_vim_behavior.sh`

## Test Results

- **Total tests:** 47
- **Passing:** 40 (85%)
- **Failing:** 7 (15%)

All 7 failing tests have CORRECT expectations now. Failures are due to ovim implementation bugs.

## Next Actions

1. Fix ovim's dot repeat implementation (see DOT_REPEAT_BUGS.md for details)
2. Re-run tests to verify fixes
3. All tests should pass once implementation is corrected
