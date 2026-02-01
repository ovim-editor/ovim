# Snapshot Testing Design for ovim

## Overview

Snapshot testing would make it easier to write and maintain tests by capturing the complete state of the editor after a sequence of operations and comparing it against a saved "snapshot" file.

## Benefits

1. **Easy to write**: Just describe the operations, run the test, and approve the snapshot
2. **Comprehensive**: Captures full editor state (buffer, cursor, mode, etc.) in one assertion
3. **Visual diffs**: Changes in behavior show up as readable diffs in snapshot files
4. **Regression detection**: Any unintended changes are immediately visible
5. **Documentation**: Snapshots serve as executable documentation of behavior

## Proposed Design

### 1. Test Structure

```rust
#[test]
fn test_o_command_middle_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('o')
        .type_text("new line")
        .press_esc();

    test.assert_snapshot();
}
```

### 2. Snapshot Format

Snapshots would be stored as TOML or RON files with a human-readable format:

```toml
[buffer]
content = """
line 1
new line
line 2
line 3
"""
line_count = 4

[cursor]
line = 1
col = 8

[state]
mode = "Normal"
modified = true

[visual]
selection = null

[metadata]
test_name = "test_o_command_middle_line"
```

### 3. Helper API

```rust
pub struct EditorTest {
    editor: Editor,
    snapshots_dir: PathBuf,
}

impl EditorTest {
    pub fn new(content: &str) -> Self { ... }

    pub fn press(&mut self, c: char) -> &mut Self { ... }
    pub fn press_key(&mut self, key: KeyCode) -> &mut Self { ... }
    pub fn press_esc(&mut self) -> &mut Self { ... }
    pub fn type_text(&mut self, text: &str) -> &mut Self { ... }
    pub fn keys(&mut self, keys: &str) -> &mut Self { ... } // Parse vim key notation

    pub fn assert_snapshot(&self) { ... }
    pub fn assert_snapshot_matches(&self, name: &str) { ... }

    // Partial snapshots for focused testing
    pub fn assert_buffer_snapshot(&self) { ... }
    pub fn assert_cursor(&self, line: usize, col: usize) { ... }
}
```

### 4. Snapshot Management

- **Location**: `tests/snapshots/` directory
- **Naming**: `{test_module}__{test_function}.snap`
- **Update mode**: `OVIM_UPDATE_SNAPSHOTS=1 cargo test` to regenerate snapshots
- **Review**: Use `git diff` to review snapshot changes before committing

### 5. Advanced Features

#### Inline Snapshots (Future)

For small tests, embed snapshots directly in the test file:

```rust
#[test]
fn test_delete_word() {
    let mut test = EditorTest::new("hello world");
    test.keys("dw");

    insta::assert_snapshot!(test.buffer_content(), @"world");
}
```

#### Visual Snapshots (Future)

Capture the rendered UI state:

```rust
#[test]
fn test_visual_line_mode() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");
    test.keys("V"); // Enter visual line mode

    // Snapshot includes highlighted line in a visual format:
    test.assert_visual_snapshot();
}
```

Expected snapshot:
```
┌────────────────┐
│[line 1]        │  <- highlighted
│ line 2         │
│ line 3         │
│                │
│-- VISUAL LINE -│
└────────────────┘
```

## Implementation Options

### Option 1: Use `insta` crate (Recommended)

- **Pros**:
  - Mature, widely used
  - Great CLI tools (`cargo insta review`)
  - Supports inline snapshots
  - Automatic snapshot management
  - Good VS Code integration

- **Cons**:
  - Additional dependency
  - Learning curve for contributors

### Option 2: Custom Implementation

- **Pros**:
  - Full control over format
  - No external dependencies
  - Simpler for basic needs

- **Cons**:
  - More maintenance work
  - Need to implement snapshot diffing, updating, etc.
  - Reinventing the wheel

## Recommendation

**Start with `insta`** for the following reasons:

1. It's battle-tested and widely adopted in the Rust ecosystem
2. Excellent developer experience with `cargo insta review`
3. Handles edge cases (file management, diffing, updating)
4. Easy to integrate with existing tests

## Migration Strategy

1. **Phase 1**: Add `insta` dependency and create helper utilities
2. **Phase 2**: Convert a few existing tests to use snapshots
3. **Phase 3**: Write new tests using snapshot approach
4. **Phase 4**: Gradually migrate remaining tests (if beneficial)

## Example Implementation

```rust
// tests/helpers/mod.rs
use ovim::editor::{Editor, InputHandler};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct EditorTest {
    pub editor: Editor,
}

impl EditorTest {
    pub fn new(content: &str) -> Self {
        Self {
            editor: Editor::with_content(content),
        }
    }

    pub fn press(&mut self, c: char) -> &mut Self {
        let event = KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty());
        InputHandler::handle_key_event(&mut self.editor, event).unwrap();
        self
    }

    pub fn press_esc(&mut self) -> &mut Self {
        let event = KeyEvent::new(KeyCode::Esc, KeyModifiers::empty());
        InputHandler::handle_key_event(&mut self.editor, event).unwrap();
        self
    }

    pub fn snapshot_state(&self) -> String {
        format!(
            "Buffer:\n{}\n\nCursor: {}:{}\nMode: {:?}\n",
            self.buffer_content(),
            self.editor.buffer().cursor().line(),
            self.editor.buffer().cursor().col(),
            self.editor.mode()
        )
    }

    fn buffer_content(&self) -> String {
        let mut content = String::new();
        for i in 0..self.editor.buffer().line_count() {
            if let Some(line) = self.editor.buffer().line(i) {
                content.push_str(line);
            }
        }
        content
    }
}

// tests/snapshot_example_test.rs
use insta::assert_snapshot;
mod helpers;
use helpers::EditorTest;

#[test]
fn test_o_command_snapshot() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('o')
        .press('n')
        .press('e')
        .press('w')
        .press_esc();

    assert_snapshot!(test.snapshot_state());
}
```

## Alternative: Hybrid Approach

Keep both traditional tests and snapshot tests:

- **Traditional tests**: For specific assertions (cursor position, line count, etc.)
- **Snapshot tests**: For complex multi-step operations and regression testing

This gives us the best of both worlds:
- Precise, focused assertions when needed
- Comprehensive state capture for complex scenarios
- Easy to review changes in editor behavior

## Testing the `o` Command with Snapshots

```rust
#[test]
fn test_o_scenarios() {
    // Test 1: Middle of file
    let mut test = EditorTest::new("line 1\nline 2\nline 3");
    test.press('o').press_esc();
    assert_snapshot!("o_middle_file", test.snapshot_state());

    // Test 2: With indentation
    let mut test = EditorTest::new("start\n    indented\nend");
    test.press('j').press('o').press_esc();
    assert_snapshot!("o_with_indentation", test.snapshot_state());

    // Test 3: Last line no newline
    let mut test = EditorTest::new("line 1\nline 2");
    test.press('j').press('o').press_esc();
    assert_snapshot!("o_last_line", test.snapshot_state());
}
```

This creates three snapshot files that capture the complete state after each operation.
