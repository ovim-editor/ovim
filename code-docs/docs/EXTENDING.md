# Extending ovim - Development Guide

This guide explains how to add new features to ovim.

## Common Development Tasks

### 1. Add a New Key Binding

**Goal**: Make a key do something new

**Steps**:

1. **Find the appropriate input handler** in `src/editor/input/mod.rs`
   - Look for the mode you're in (Normal, Insert, Visual, etc)
   - Find where other keys for that mode are handled

2. **Add your key handler**:
```rust
// In normal mode handlers (around line 1500)
KeyCode::Char('w') if key_event.modifiers.is_empty() => {
    // Your code here
    Self::jump_to_next_word(editor)?;
}
```

3. **Implement the action**:
```rust
fn jump_to_next_word(editor: &mut Editor) -> Result<()> {
    editor.move_word_forward();
    Ok(())
}
```

4. **Add tests** in `tests/input_test.rs`:
```rust
#[test]
fn test_new_keybinding() {
    let mut test = EditorTest::new("hello world");
    test.send_keys("w");  // Jump to next word
    assert_eq!(test.cursor(), (0, 6));
}
```

5. **Build and test**:
```bash
cargo build
cargo test test_new_keybinding
```

---

### 2. Add a New Ex Command

**Goal**: Make `:mycommand` work

**Steps**:

1. **Add command enum variant** in `src/editor/commands.rs`:
```rust
pub enum ExCommand {
    Write,           // :w
    Quit,            // :q
    MyCommand(String),  // :mycommand [args]
}
```

2. **Add parsing logic**:
```rust
impl ExCommand {
    pub fn parse(input: &str) -> Option<Self> {
        match input {
            "w" => Some(ExCommand::Write),
            "q" => Some(ExCommand::Quit),
            s if s.starts_with("mycommand") => {
                let args = s[9..].trim().to_string();
                Some(ExCommand::MyCommand(args))
            }
            _ => None,
        }
    }
}
```

3. **Add execution logic**:
```rust
impl ExCommand {
    pub fn execute(&self, editor: &mut Editor) -> Result<()> {
        match self {
            ExCommand::Write => editor.write_file(),
            ExCommand::Quit => { editor.should_quit = true; Ok(()) }
            ExCommand::MyCommand(args) => {
                // Your implementation
                println!("Running mycommand with args: {}", args);
                Ok(())
            }
        }
    }
}
```

4. **Add tests**:
```rust
#[test]
fn test_mycommand() {
    let mut test = EditorTest::new("hello");
    test.send_command("mycommand arg1 arg2");
    // Verify result
}
```

---

### 3. Add a New Motion

**Goal**: Make `z` move cursor somewhere new

**Steps**:

1. **Define the motion** in `src/editor/motions.rs`:
```rust
pub fn motion_jump_to_line_start(editor: &Editor) -> (usize, usize) {
    let current_line = editor.cursor().0;
    (current_line, 0)
}
```

2. **Add to motion dispatcher** in `src/editor/input/mod.rs`:
```rust
KeyCode::Char('0') if key_event.modifiers.is_empty() => {
    let new_pos = editor::motions::motion_jump_to_line_start(editor);
    editor.set_cursor(new_pos);
}
```

3. **Handle with operators**:
```rust
// This should automatically work:
// d0 - delete to start of line
// y0 - yank to start of line
// c0 - change to start of line
```

4. **Add tests**:
```rust
#[test]
fn test_motion_jump_to_line_start() {
    let mut test = EditorTest::new("    hello");
    test.send_keys("e0");  // End of word, then jump to start
    assert_eq!(test.cursor(), (0, 0));
}

#[test]
fn test_motion_with_operator() {
    let mut test = EditorTest::new("hello world");
    test.send_keys("d0");  // Delete to line start
    assert_eq!(test.buffer_content(), "world");
}
```

---

### 4. Add a New Text Object

**Goal**: Make `ib` select inside braces `{...}`

**Steps**:

1. **Implement the text object** in `src/editor/textobjects.rs`:
```rust
pub fn select_inner_braces(editor: &Editor) -> Option<(usize, usize)> {
    let pos = editor.get_position();
    let buffer = editor.buffer();

    // Find opening brace before cursor
    let mut open_pos = pos;
    let mut depth = 0;
    for i in (0..pos).rev() {
        match buffer.char_at(i) {
            Some('}') => depth += 1,
            Some('{') => {
                if depth == 0 {
                    open_pos = i;
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }

    // Find closing brace after cursor
    let mut close_pos = pos;
    let mut depth = 0;
    for i in pos..buffer.len() {
        match buffer.char_at(i) {
            Some('{') => depth += 1,
            Some('}') => {
                if depth == 0 {
                    close_pos = i;
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }

    if open_pos < close_pos {
        Some((open_pos + 1, close_pos))
    } else {
        None
    }
}
```

2. **Add to text object handler** in `src/editor/input/mod.rs`:
```rust
'b' if text_object_mode => {
    if let Some((start, end)) = select_inner_braces(editor) {
        editor.select_range(start, end);
    }
}
```

3. **Test**:
```rust
#[test]
fn test_select_inner_braces() {
    let mut test = EditorTest::new("{ hello world }");
    test.send_keys("vib");  // Visual select inside braces
    assert_eq!(test.selection(), (1, 14));
}

#[test]
fn test_delete_inner_braces() {
    let mut test = EditorTest::new("{ hello world }");
    test.send_keys("dib");  // Delete inside braces
    assert_eq!(test.buffer_content(), "{ }");
}
```

---

### 5. Add a New REST API Endpoint

**Goal**: Add `/custom` endpoint

**Steps**:

1. **Add request/response types** in `src/api/state.rs`:
```rust
pub enum ApiRequest {
    GetSnapshot,
    SendKeys(String),
    // ... existing ...
    GetCustomData,  // New
}

pub enum ApiResponse {
    Snapshot(EditorSnapshot),
    Success(SuccessResponse),
    // ... existing ...
    CustomData(CustomDataResponse),  // New
}

#[derive(Serialize, Deserialize)]
pub struct CustomDataResponse {
    pub data: String,
}
```

2. **Add route** in `src/api/routes.rs`:
```rust
pub fn create_router() -> Router {
    Router::new()
        .route("/snapshot", get(handlers::get_snapshot))
        .route("/custom", get(handlers::get_custom_data))  // New
        // ... other routes ...
}
```

3. **Add handler** in `src/api/handlers.rs`:
```rust
pub async fn get_custom_data(
    State(tx): State<Sender<ApiRequest>>,
) -> Json<ApiResponse> {
    let response = send_request(tx, ApiRequest::GetCustomData).await;
    response
}
```

4. **Add request handler** in `src/event_loop.rs`:
```rust
match api_request {
    ApiRequest::GetSnapshot => { /* ... */ }
    ApiRequest::GetCustomData => {
        let data = editor.get_custom_data();
        let response = CustomDataResponse { data };
        send_response(ApiResponse::CustomData(response))
    }
}
```

5. **Test**:
```bash
# Start ovim in headless mode
./target/release/ovim test.txt --headless --session test &

# Call the endpoint
curl http://127.0.0.1:PORT/custom | jq '.'

# Stop
./ovim-ctl kill test
```

---

### 6. Add LSP Feature Support

**Goal**: Support a new LSP method (e.g., `textDocument/implementation`)

**Steps**:

1. **Add method to LspManager** in `src/lsp/mod.rs`:
```rust
pub async fn goto_implementation(&mut self, uri: Url, pos: Position) -> Result<Vec<Location>> {
    let server = self.servers.get_mut(&language)?;

    let params = GotoImplementationParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position: pos,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };

    let response = server.request::<GotoImplementationRequest>(params).await?;
    Ok(response.unwrap_or_default())
}
```

2. **Wire to Editor** in `src/editor/mod.rs`:
```rust
pub fn goto_implementation(&mut self) -> Result<()> {
    let (uri, pos) = self.get_current_position_for_lsp()?;
    self.lsp_manager.goto_implementation(uri, pos).await?;
    Ok(())
}
```

3. **Add keybinding** in `src/editor/input/mod.rs`:
```rust
KeyCode::Char('i') if key_event.modifiers == (KeyModifiers::CTRL | KeyModifiers::SHIFT) => {
    editor.goto_implementation()?;
}
```

4. **Test**:
```rust
#[test]
fn test_goto_implementation() {
    let mut test = EditorTest::new_with_lsp("trait Foo { fn bar(); }\nimpl Foo { fn bar() {} }");
    test.send_keys("gI");  // Jump to implementation
    // Verify cursor moved to impl location
}
```

---

### 7. Add Configuration Option

**Goal**: Let users configure behavior via `init.lua`

**Steps**:

1. **Add option to config** in `src/config/mod.rs`:
```rust
pub struct Config {
    pub highlight_on_yank: bool,
    pub max_undo_history: usize,
    // ... existing options ...
    pub my_new_option: String,
}

impl Config {
    pub fn new() -> Result<Self> {
        let mut config = Self {
            highlight_on_yank: true,
            max_undo_history: 1000,
            my_new_option: String::new(),
            // ... defaults ...
        };
        config.load_lua()?;
        Ok(config)
    }
}
```

2. **Expose to Lua**:
```rust
// In Lua globals setup
lua.globals().set("vim", vim_table)?;
// Users can now:
// vim.opt.my_new_option = "value"
```

3. **Use in code**:
```rust
if editor.config.highlight_on_yank {
    // Show highlight after yank
}
```

4. **Document in init.lua**:
```lua
-- ~/.config/ovim/init.lua
vim.opt.my_new_option = "value"
```

---

### 8. Add Performance Optimization

**Goal**: Make something faster

**Steps**:

1. **Identify bottleneck**:
```bash
# Use profiling
perf record ./target/release/ovim large_file.txt
perf report
```

2. **Implement optimization**:
```rust
// Before: Linear search
let result = items.iter().find(|x| x.key == search_key);

// After: Indexed lookup
let result = items_map.get(search_key);
```

3. **Measure improvement**:
```rust
#[test]
fn bench_operation() {
    let timer = std::time::Instant::now();
    for _ in 0..1000 {
        slow_operation();
    }
    println!("Elapsed: {:?}", timer.elapsed());
}
```

4. **Document trade-offs**:
```rust
// PERF: Use DashMap instead of Mutex<HashMap>
// Trade-off: Slightly more memory, but better concurrent access
// Improvement: 10x faster for 100+ concurrent readers
```

---

## Testing Best Practices

### Unit Tests
```rust
#[test]
fn test_operation() {
    // Setup
    let mut editor = Editor::new();

    // Action
    editor.some_operation();

    // Assert
    assert_eq!(editor.cursor(), (0, 5));
}
```

### Integration Tests with EditorTest
```rust
#[test]
fn test_with_fluent_api() {
    let mut test = EditorTest::new("hello world")
        .send_keys("w")
        .send_keys("dw")
        .verify_content("hello ");
}
```

### LSP Tests
```rust
#[tokio::test]
async fn test_lsp_feature() {
    let mut test = EditorTest::new_with_lsp("fn foo() {}");
    test.send_keys("K");  // Hover

    // Wait for LSP response
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify hover shown
    assert!(test.hover_visible());
}
```

### Property Tests
```rust
#[test]
fn test_property_inverted() {
    for text in ["hello", "世界", "🎉", "  "] {
        let mut editor = Editor::new(text);
        editor.invert_selection();
        editor.invert_selection();
        assert_eq!(editor.buffer_content(), text);
    }
}
```

---

## Code Review Checklist

Before submitting PR:

- [ ] Code builds with `cargo build`
- [ ] No clippy warnings: `cargo clippy`
- [ ] Tests pass: `cargo test`
- [ ] New tests added for new functionality
- [ ] No unwrap() without comment (or proper error handling)
- [ ] No global mutable state
- [ ] Memory safe (no unsafe blocks unless documented)
- [ ] Performance impact analyzed
- [ ] Documentation updated
- [ ] Commit message is clear

---

## Debugging Tips

### Enable Logging
```bash
RUST_LOG=debug ./target/release/ovim file.txt
```

### Print Debug Info
```rust
eprintln!("DEBUG: cursor={:?}, buffer_len={}",
         editor.cursor(),
         editor.buffer().len());
```

### Use Debugger
```bash
lldb ./target/debug/ovim
(lldb) breakpoint set -f mod.rs -l 123
(lldb) run file.txt
(lldb) p editor.cursor()
```

### Trace LSP Messages
```bash
RUST_LOG=trace ./target/release/ovim --headless file.rs 2>&1 | grep LSP
```

---

## Common Patterns

### Pattern 1: Validate Input
```rust
pub fn do_operation(count: usize, text: &str) -> Result<()> {
    if count == 0 || count > 1000000 {
        return Err(anyhow!("Invalid count: {}", count));
    }
    if text.is_empty() {
        return Err(anyhow!("Empty text not allowed"));
    }
    // Proceed with operation
    Ok(())
}
```

### Pattern 2: Graceful Degradation
```rust
// If LSP not available, fall back to basic behavior
pub fn complete(&mut self) -> Vec<String> {
    if let Ok(lsp_completions) = self.lsp_manager.get_completions() {
        return lsp_completions;
    }
    // Fallback to buffer word search
    self.buffer.extract_words()
}
```

### Pattern 3: Minimal Locking
```rust
// Take lock only for minimal time
let copy = {
    let state = self.state.lock().unwrap();
    state.value.clone()
};
// Do work without lock
process_value(copy);
```

### Pattern 4: Early Return
```rust
pub fn process(&mut self) -> Result<()> {
    let Some(value) = self.get_value()? else {
        return Ok(());  // Graceful exit
    };
    // Process value
    Ok(())
}
```

---

## File Organization Guidelines

1. **Keep files under 2000 lines**
   - If approaching 2000, plan split
   - Break into smaller, focused modules

2. **Group related functions**
   - Keep motions in motions.rs
   - Keep operators in operators.rs
   - Don't scatter related code

3. **Public vs Private**
   - Only export what's needed
   - Use private helpers for implementation details

4. **Comments**
   - Document why, not what
   - Add comments for non-obvious logic
   - Include example usage for complex functions

---

## Next Steps

- **Want to extend UI?** → See [UI_SYSTEM.md](./UI_SYSTEM.md)
- **Want to add LSP feature?** → See [LSP_SYSTEM.md](./LSP_SYSTEM.md)
- **Want to optimize?** → See [PERFORMANCE.md](./PERFORMANCE.md)
- **Questions?** → Check existing tests for examples

---

**Last Updated**: 2025-10-26
**Difficulty**: Medium (assumes Rust knowledge)
**Time**: 1-8 hours depending on complexity
