# Quick Start: Testing in ovim

A 5-minute guide to running and writing tests in ovim.

## 🚀 Running Tests (First Time)

```bash
# 1. Run all tests (they will fail - that's expected!)
cargo test

# 2. Review and accept snapshots
cargo insta review
# Press 'a' to accept each snapshot after reviewing

# 3. Run tests again (should all pass now)
cargo test
```

## ✅ Verify Everything Works

```bash
# Run specific test suites
cargo test --test insert_operations_test
cargo test --test paste_operations_test
cargo test --test delete_operations_test

# Run a single test
cargo test test_o_basic
```

## 📝 Writing Your First Test

Create `tests/my_test.rs`:

```rust
mod helpers;
use helpers::EditorTest;
use insta::assert_snapshot;

#[test]
fn test_my_operation() {
    // 1. Create editor with initial content
    let mut test = EditorTest::new("hello world");

    // 2. Perform operations (fluent API)
    test.press('w')        // Move to "world"
        .press('i')        // Insert mode
        .type_text("big ") // Type text
        .press_esc();      // Exit insert mode

    // 3. Capture snapshot
    assert_snapshot!(test.snapshot_state());
}
```

Run your test:
```bash
cargo test test_my_operation
cargo insta review  # Accept the snapshot
cargo test test_my_operation  # Should pass
```

## 🔍 Common Test Patterns

### Insert and check
```rust
#[test]
fn test_insert() {
    let mut test = EditorTest::new("line 1");

    test.press('o')
        .type_text("line 2")
        .press_esc();

    assert_snapshot!(test.snapshot_state());
}
```

### Delete and undo
```rust
#[test]
fn test_delete_undo() {
    let mut test = EditorTest::new("hello world");

    test.keys("dw")  // Delete word
        .press('u'); // Undo

    assert_snapshot!(test.snapshot_state());
}
```

### Yank and paste
```rust
#[test]
fn test_yank_paste() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.keys("yy")  // Yank line
        .keys("p");  // Paste

    assert_snapshot!(test.snapshot_state());
}
```

### Traditional assertions
```rust
#[test]
fn test_with_assertions() {
    let mut test = EditorTest::new("hello");

    test.keys("$");  // Move to end

    // Use assertions for specific checks
    test.assert_cursor(0, 4);  // Line 0, column 4
    test.assert_line_count(1);

    // Still use snapshot for full state
    assert_snapshot!(test.snapshot_state());
}
```

## 🛠️ Helper API Cheat Sheet

### Creating Tests
```rust
EditorTest::new("content")  // With initial content
EditorTest::empty()         // Empty buffer
```

### Input
```rust
.press('x')           // Press character
.press_key(KeyCode::Esc)  // Press any key
.press_esc()          // Escape
.press_enter()        // Enter
.type_text("hello")   // Type multiple chars
.keys("dd")           // Vim key sequence
```

### Snapshots
```rust
assert_snapshot!(test.snapshot_state());        // Full state with cursor marker
assert_snapshot!(test.snapshot_buffer());       // Just buffer content
assert_snapshot!(test.snapshot_buffer_and_cursor());  // Buffer + cursor position
```

### Assertions
```rust
test.assert_cursor(line, col);  // Check cursor position
test.assert_mode(Mode::Normal); // Check mode
test.assert_line_count(3);      // Check line count
test.assert_line(0, "hello\n"); // Check line content
```

### Queries
```rust
let content = test.buffer_content();  // Get buffer as string
let line = test.line(0);              // Get specific line
let count = test.line_count();        // Get line count
let mode = test.mode();               // Get current mode
let (line, col) = test.cursor();      // Get cursor position
```

## 🐛 What to Do When Tests Fail

### Scenario 1: Code Changed (Expected)
```bash
cargo test  # Fails with snapshot diff
cargo insta review  # Review the change
# If correct: press 'a' to accept
# If wrong: press 'r' to reject, then fix your code
```

### Scenario 2: Unexpected Failure
```bash
# See full output
cargo test -- --nocapture

# Run just that test
cargo test test_name -- --nocapture

# Check the diff shown in terminal
# Fix the bug or accept if behavior is correct
```

## 📚 Examples to Learn From

Look at these files for examples:
- `tests/insert_operations_test.rs` - Insert operations (i, I, a, A, o, O)
- `tests/paste_operations_test.rs` - Paste operations (p, P)
- `tests/delete_operations_test.rs` - Delete operations (x, dd, dw, etc.)
- `tests/snapshot_test.rs` - Simple examples

## ⚡ Tips

1. **Name tests descriptively**: `test_o_with_indentation` not `test1`
2. **Test one thing**: Focus on one operation per test
3. **Test edge cases**: Empty lines, boundaries, etc.
4. **Review snapshots carefully**: Don't blindly accept
5. **Use assertions for specifics**: `assert_cursor()` for exact position
6. **Use snapshots for everything else**: Catches unexpected changes

## 🚫 Common Mistakes

❌ **Don't** blindly accept all snapshots
```bash
cargo insta accept  # Dangerous! Review first!
```

✅ **Do** review each one
```bash
cargo insta review  # Safe - shows diffs
```

❌ **Don't** create giant tests
```rust
#[test]
fn test_everything() {
    // 100 lines of operations...
}
```

✅ **Do** create focused tests
```rust
#[test]
fn test_o_basic() { /* ... */ }

#[test]
fn test_o_with_indentation() { /* ... */ }
```

## 🎯 Your Next Steps

1. ✅ Run tests and review snapshots
2. ✅ Read one test file to understand patterns
3. ✅ Write a test for a new operation
4. ✅ Fix a bug and update snapshots
5. ✅ Read `tests/README.md` for more details

## 📖 Further Reading

- **Detailed guide**: `tests/README.md`
- **Implementation details**: `TESTING_IMPROVEMENTS.md`
- **Snapshot testing docs**: https://insta.rs

## 💡 Quick Reference

```bash
# Essential commands
cargo test                        # Run all tests
cargo test --test <name>         # Run specific file
cargo test <test_name>           # Run specific test
cargo insta review               # Review snapshot changes
cargo test -- --nocapture        # See full output

# Helper commands
cargo test -- --list             # List all tests
cargo test --help                # See all options
cargo insta --help               # See insta options
```

---

**You're ready to go!** Start with `cargo test` and take it from there. 🚀
