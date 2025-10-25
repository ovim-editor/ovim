# Test File for Syntax Highlighting Performance

This file has multiple lines to test the initial load performance.

## Code Block 1
```rust
fn main() {
    println!("Hello, world!");
}
```

## Code Block 2
```rust
pub struct Buffer {
    rope: Rope,
    cursor: Cursor,
}
```

## More Text

This is some regular text.

**Bold text** and *italic text*.

- List item 1
- List item 2
- List item 3

1. Numbered item 1
2. Numbered item 2
3. Numbered item 3

## Code Block 3
```rust
impl Buffer {
    pub fn new() -> Self {
        Self {
            rope: Rope::new(),
            cursor: Cursor::new(0, 0),
        }
    }
}
```

End of test file.
