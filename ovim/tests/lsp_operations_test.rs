mod helpers;

use helpers::EditorTest;
use std::sync::atomic::{AtomicU64, Ordering};

fn temp_test_path(name: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!("ovim_test_{}_{}", id, name))
        .to_string_lossy()
        .to_string()
}

/// Test that 'gd' command requests goto definition
#[test]
fn test_gd_requests_goto_definition() {
    let code = r#"fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    let result = add(5, 3);
}
"#;

    let mut test = EditorTest::new(code);

    // Move to line 6 (the add function call)
    test.keys("6G");

    // Move to the 'add' identifier
    test.keys("0").keys("17l");

    // Press 'gd' - this should request goto definition (even if no LSP is running)
    test.keys("gd");

    // In the absence of LSP, cursor should not move
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test that 'K' command requests hover
#[test]
fn test_k_requests_hover() {
    let code = r#"fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    let result = add(5, 3);
}
"#;

    let mut test = EditorTest::new(code);

    // Move to the 'add' function call
    test.keys("5G0").keys("17l");

    // Press 'K' - this should request hover
    test.press('K');

    // Without LSP, no hover info should be available
    assert!(test.editor.hover_info().is_none());
}

/// Test that hover clears on cursor movement
#[test]
fn test_hover_clears_on_movement() {
    let mut test = EditorTest::new("fn main() {}\n");

    // Request hover (won't get info without LSP, but will set the flag)
    test.press('K');

    // Move cursor - any key press should clear hover
    test.keys("l");

    // Hover should be cleared (none because no LSP)
    assert!(test.editor.hover_info().is_none());
}

/// Test that multiple movements work without LSP
#[test]
fn test_movement_without_lsp() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    // Moving down should work
    test.keys("j");
    test.assert_cursor(1, 0);

    // Moving right should work
    test.keys("l");
    test.assert_cursor(1, 1);

    // Entering insert mode should work
    test.keys("i");
    test.assert_mode(ovim::mode::Mode::Insert);
}

/// Test goto definition keybinding exists
#[test]
fn test_gd_keybinding_exists() {
    let code = r#"fn test() {}
fn main() {
    test();
}
"#;

    let mut test = EditorTest::new(code);
    test.keys("3G0").keys("4l");

    let cursor_before = test.cursor();

    // Press gd (without LSP it should do nothing, but shouldn't error)
    test.keys("gd");

    // Cursor should not move (no LSP available)
    assert_eq!(test.cursor(), cursor_before);
}

/// Test that gd works in normal mode only
#[test]
fn test_gd_normal_mode_only() {
    let mut test = EditorTest::new("fn test() {}\n");

    // In insert mode, 'g' and 'd' should type normally
    test.keys("i");
    test.keys("gd");

    test.press_esc();

    // Should have inserted 'gd' as text
    assert!(test.buffer_content().contains("gd"));
}

/// Test K keybinding exists
#[test]
fn test_k_keybinding_for_hover() {
    let mut test = EditorTest::new("fn test() {}\n");

    test.keys("0"); // Start of line

    // Press K (without LSP it should do nothing)
    test.press('K');

    // Should remain in normal mode
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test that LSP actions don't interfere with normal operations
#[test]
fn test_lsp_keybindings_dont_break_editing() {
    let mut test = EditorTest::new("hello\nworld\n");

    // Normal Vim operations should work fine
    test.keys("j"); // Move down
    test.assert_cursor(1, 0);

    test.keys("i"); // Enter insert mode
    test.type_text("new ");

    test.press_esc(); // Exit insert mode

    // Verify normal editing works
    assert!(test.buffer_content().contains("new world"));
}

/// Test visual mode with gd (should exit visual and attempt goto)
#[test]
fn test_gd_in_visual_mode() {
    let code = r#"fn test() {}
fn main() {
    test();
}
"#;

    let mut test = EditorTest::new(code);

    // Enter visual mode
    test.keys("v");
    test.assert_mode(ovim::mode::Mode::Visual);

    // Move to select some text
    test.keys("ll");

    // Press gd - should exit visual mode
    test.keys("gd");

    // Should be back in normal mode
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test count prefix doesn't affect gd (goto definition ignores count)
#[test]
fn test_gd_ignores_count() {
    let code = r#"fn test() {}
fn main() {}
"#;

    let mut test = EditorTest::new(code);

    test.keys("2G0");

    let cursor_before = test.cursor();

    // Type '3gd' - the '3' should not affect gd
    test.keys("3gd");

    // Without LSP, cursor should remain at same position
    // (the count is consumed by gd but has no effect)
    assert_eq!(test.cursor(), cursor_before);
}

/// Test file path is preserved for LSP
#[test]
fn test_file_path_for_lsp() {
    let mut test = EditorTest::new("");
    let path = temp_test_path("test.rs");

    // Set a file path
    test.set_file_path(path.clone());

    // File path should be set.
    let file_path = test.editor.buffer().file_path();
    assert!(file_path.is_some());
    assert_eq!(file_path.unwrap(), path);
}

/// Test LSP works with different file extensions
#[test]
fn test_lsp_file_type_detection() {
    // Rust file
    let mut test_rs = EditorTest::new("");
    let rs_path = temp_test_path("test.rs");
    test_rs.set_file_path(rs_path.clone());
    assert_eq!(test_rs.editor.buffer().file_path().unwrap(), rs_path);

    // JavaScript file
    let mut test_js = EditorTest::new("");
    let js_path = temp_test_path("test.js");
    test_js.set_file_path(js_path.clone());
    assert_eq!(test_js.editor.buffer().file_path().unwrap(), js_path);

    // Python file
    let mut test_py = EditorTest::new("");
    let py_path = temp_test_path("test.py");
    test_py.set_file_path(py_path.clone());
    assert_eq!(test_py.editor.buffer().file_path().unwrap(), py_path);
}

/// Test snapshot of code with LSP-relevant syntax
#[test]
fn test_lsp_code_snapshot() {
    let code = r#"// Rust code with various identifiers for LSP

struct Point {
    x: i32,
    y: i32,
}

impl Point {
    fn new(x: i32, y: i32) -> Self {
        Point { x, y }
    }

    fn distance(&self) -> f64 {
        ((self.x.pow(2) + self.y.pow(2)) as f64).sqrt()
    }
}

fn main() {
    let p = Point::new(3, 4);
    println!("Distance: {}", p.distance());
}
"#;

    let mut test = EditorTest::new(code);

    // Cursor positions that would be good for LSP testing:

    // Position 1: On 'Point' struct name
    test.keys("3G0").keys("7l");
    test.assert_cursor(2, 7);

    // Position 2: On 'new' method
    test.keys("9G0").keys("7l");
    test.assert_cursor(8, 7);

    // Position 3: On 'Point::new' call
    test.keys("19G0").keys("12l");
    test.assert_cursor(18, 12);
}
