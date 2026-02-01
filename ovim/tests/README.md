# ovim Test Suite

This directory contains comprehensive tests for ovim's editor functionality using snapshot testing.

## Test Organization

### Test Files

- **`helpers/mod.rs`** - Test helper utilities including `EditorTest` fluent API
- **`snapshot_test.rs`** - Initial snapshot test examples
- **`insert_operations_test.rs`** - Tests for all insert operations (i, I, a, A, o, O)
- **`paste_operations_test.rs`** - Tests for paste operations (p, P) with various content types
- **`delete_operations_test.rs`** - Tests for delete operations (x, X, dd, dw, d$, etc.)
- **`o_command_test.rs`** - Traditional assertion-based tests for `o` command
- **`paste_undo_test.rs`** - Traditional tests for paste/undo interactions
- **`lsp_operations_test.rs`** - Tests for LSP features (goto definition, hover, keybindings)
- Other test files for specific features (cursor, syntax, etc.)

## Running Tests

### Run All Tests

```bash
cargo test
```

### Run Specific Test File

```bash
cargo test --test insert_operations_test
cargo test --test paste_operations_test
cargo test --test delete_operations_test
cargo test --test lsp_operations_test
```

### Run Specific Test

```bash
cargo test test_o_basic
cargo test test_p_linewise_basic
```

### Run with Output

```bash
cargo test -- --nocapture
```

## Snapshot Testing

We use [insta](https://insta.rs) for snapshot testing. This allows us to capture the complete editor state and compare it against saved snapshots.

### First Time Running Snapshot Tests

The first time you run snapshot tests, they will fail because no snapshots exist yet. You need to review and accept the snapshots:

```bash
# Run tests (they will fail and create .snap.new files)
cargo test

# Review the new snapshots
cargo insta review

# Or accept all snapshots at once (use with caution!)
cargo insta accept
```

### Reviewing Snapshot Changes

When test behavior changes, insta will show you the diff:

```bash
# Review changes interactively
cargo insta review

# This opens an interactive prompt where you can:
# - [a]ccept: Accept this change
# - [r]eject: Reject this change
# - [s]kip: Skip for now
# - [q]uit: Exit review mode
```

### Updating Snapshots After Code Changes

If you intentionally changed editor behavior:

```bash
# Run tests to see what changed
cargo test

# Review and accept the changes
cargo insta review
```

### Environment Variable for Auto-Accept (Use with Caution!)

```bash
# Update all snapshots without review
INSTA_UPDATE=always cargo test

# Or force new snapshots
INSTA_FORCE_UPDATE=1 cargo test
```

⚠️ **Warning**: Only use auto-accept when you're confident the changes are correct!

## Test Helper API

The `EditorTest` helper provides a fluent API for writing tests:

```rust
use helpers::EditorTest;
use insta::assert_snapshot;

#[test]
fn test_example() {
    let mut test = EditorTest::new("initial content");

    test.press('i')           // Enter insert mode
        .type_text("hello")   // Type text
        .press_esc()          // Exit insert mode
        .keys("dd")           // Delete line
        .press('u');          // Undo

    // Capture full state (buffer, cursor, mode)
    assert_snapshot!(test.snapshot_state());

    // Or just buffer content
    assert_snapshot!(test.snapshot_buffer());

    // Or traditional assertions
    test.assert_cursor(0, 0);
    test.assert_mode(Mode::Normal);
    test.assert_line_count(1);
}
```

### Available Methods

#### Creating Tests

- `EditorTest::new(content)` - Create test with initial content
- `EditorTest::empty()` - Create test with empty buffer

#### Input Methods

- `.press(char)` - Press a character key
- `.press_key(KeyCode)` - Press any key
- `.press_esc()` - Press Escape
- `.press_enter()` - Press Enter
- `.press_backspace()` - Press Backspace
- `.press_with(KeyCode, KeyModifiers)` - Press key with modifiers
- `.type_text(str)` - Type multiple characters
- `.keys(str)` - Execute vim key sequence (e.g., "dd", "3j", "yy")

#### Snapshot Methods

- `.snapshot_state()` - Full state with cursor markers
- `.snapshot_buffer()` - Just buffer content
- `.snapshot_buffer_and_cursor()` - Buffer with cursor position

#### Assertion Methods

- `.assert_cursor(line, col)` - Assert cursor position
- `.assert_mode(Mode)` - Assert editor mode
- `.assert_line_count(count)` - Assert number of lines
- `.assert_line(idx, expected)` - Assert specific line content

#### Query Methods

- `.buffer_content()` - Get full buffer as string
- `.line(idx)` - Get specific line
- `.line_count()` - Get line count
- `.mode()` - Get current mode
- `.cursor()` - Get cursor position as (line, col)

## Test Coverage Goals

Our test suite aims to cover:

### ✅ Completed

- [x] Insert operations (i, I, a, A, o, O)
- [x] Paste operations (p, P)
- [x] Delete operations (x, X, dd, dw, d$, etc.)
- [x] Boundary conditions (empty lines, last line, first line)
- [x] Operations with/without trailing newlines
- [x] Indentation preservation
- [x] Undo/redo interactions
- [x] Count prefixes (e.g., 3dd, 5p)
- [x] LSP keybindings (gd for goto definition, K for hover)

### 🔄 To Add

- [ ] Visual mode operations (v, V, Ctrl-V)
- [ ] Change operations (c, C, ciw, caw, etc.)
- [ ] Search and replace (/, ?, n, N, :%s)
- [ ] Macros (q, @)
- [ ] Marks (m, `)
- [ ] Text objects (aw, iw, ap, ip, a", i", etc.)
- [ ] Complex motion combinations
- [ ] Multi-line operations
- [ ] Edge cases with empty buffer
- [ ] Unicode and multi-byte characters

## Snapshot File Organization

Snapshots are stored in `tests/snapshots/` with the naming pattern:
```
tests/snapshots/{module}__{test_name}.snap
```

Example:
```
tests/snapshots/insert_operations_test__test_o_basic.snap
tests/snapshots/paste_operations_test__test_p_linewise_basic.snap
```

## Writing Good Snapshot Tests

### ✅ DO

- Test one concept per test function
- Use descriptive test names
- Test boundary conditions
- Test error cases
- Group related tests with comments
- Review snapshots carefully before accepting

### ❌ DON'T

- Blindly accept all snapshots without reviewing
- Create massive tests that do too many things
- Test implementation details instead of behavior
- Forget to test edge cases
- Auto-accept without understanding what changed

## CI/CD Integration

For continuous integration:

```bash
# In CI, tests should fail if snapshots don't match
cargo test

# Don't use INSTA_UPDATE in CI - we want failures!
```

In your CI configuration, you can optionally:

```yaml
# Example GitHub Actions
- name: Run tests
  run: cargo test

- name: Upload snapshot diff artifacts
  if: failure()
  uses: actions/upload-artifact@v3
  with:
    name: snapshot-diffs
    path: tests/snapshots/*.snap.new
```

## Debugging Test Failures

### Snapshot Mismatch

When a snapshot test fails:

1. Look at the diff shown in the test output
2. Run `cargo insta review` to see the visual diff
3. Determine if the change is expected or a bug
4. Accept (if correct) or fix the bug (if incorrect)

### Test Panic or Error

- Use `cargo test -- --nocapture` to see full output
- Add `.dbg()` calls to inspect intermediate state
- Use traditional assertions alongside snapshots

### Traditional vs Snapshot Tests

Both have their place:

**Snapshot tests** - Best for:
- Complex state validation
- Multi-step operations
- Regression testing
- Visual verification of output

**Traditional assertions** - Best for:
- Simple, specific conditions
- Performance-critical tests
- When exact values matter
- Mathematical correctness

## Tips and Best Practices

1. **Run tests frequently** during development
2. **Review snapshots** carefully before accepting
3. **Use git diff** to see what changed in snapshots
4. **Keep tests focused** - one logical operation per test
5. **Test edge cases** - empty buffers, boundaries, etc.
6. **Document unusual test cases** with comments
7. **Group related tests** with clear section headers

## Resources

- [insta documentation](https://insta.rs)
- [Snapshot testing guide](https://insta.rs/docs/snapshot-testing/)
- [cargo-insta CLI](https://insta.rs/docs/cli/)

## Questions?

If you have questions about the test suite:
1. Read the test files for examples
2. Check the `helpers/mod.rs` for available utilities
3. Review existing snapshots to understand the format
4. Consult the insta documentation for advanced features
