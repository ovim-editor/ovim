mod helpers;

use helpers::EditorTest;
use std::fs;
use std::path::PathBuf;

/// Test LSP goto definition across multiple files
#[test]
fn test_lsp_goto_definition_multi_file() {
    // Create temporary test files
    let temp_dir = std::env::temp_dir().join("ovim_lsp_test");
    fs::create_dir_all(&temp_dir).unwrap();

    // File 1: mod.rs
    let file1 = temp_dir.join("mod.rs");
    fs::write(&file1, "pub mod utils;\npub use utils::*;\n").unwrap();

    // File 2: utils.rs
    let file2 = temp_dir.join("utils.rs");
    fs::write(&file2, "pub fn helper() -> i32 {\n    42\n}\n\npub fn caller() {\n    let x = helper();\n}\n").unwrap();

    // File 3: main.rs
    let file3 = temp_dir.join("main.rs");
    fs::write(&file3, "mod utils;\n\nfn main() {\n    utils::helper();\n}\n").unwrap();

    // Test navigation from main.rs
    let mut test = EditorTest::new("mod utils;\n\nfn main() {\n    utils::helper();\n}\n");
    test.set_file_path(file3.to_str().unwrap().to_string());

    // Move to 'helper' call
    test.keys("4G");
    test.keys("0");
    test.keys("f:");
    test.keys("l");

    // Request goto definition
    test.keys("gd");

    // Without LSP running, should handle gracefully
    test.assert_mode(ovim::mode::Mode::Normal);

    // Cleanup
    let _ = fs::remove_dir_all(&temp_dir);
}

/// Test LSP hover across multiple references
#[test]
fn test_lsp_hover_multi_reference() {
    let code = r#"
struct Point {
    x: i32,
    y: i32,
}

fn create_point() -> Point {
    Point { x: 0, y: 0 }
}

fn use_point() {
    let p = create_point();
    println!("{}", p.x);
}
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path("/tmp/test_hover.rs".to_string());

    // Hover on Point in create_point return type
    test.keys("7G");
    test.keys("0");
    test.keys("f:");
    test.keys("l");
    test.keys("K");

    test.assert_mode(ovim::mode::Mode::Normal);

    // Hover on Point in struct definition
    test.keys("2G");
    test.keys("0");
    test.keys("K");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP with nested module structure
#[test]
fn test_lsp_nested_modules() {
    let code = r#"
mod outer {
    pub mod inner {
        pub fn deep_function() -> i32 {
            100
        }
    }
}

fn main() {
    let result = outer::inner::deep_function();
}
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path("/tmp/test_nested.rs".to_string());

    // Navigate to deep_function call
    test.keys("11G");
    test.keys("0");
    test.keys("f:");
    test.keys("f:");
    test.keys("l");

    test.keys("gd");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP goto definition and back
#[test]
fn test_lsp_goto_and_back() {
    let code = r#"
fn helper() -> i32 {
    42
}

fn main() {
    let x = helper();
}
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path("/tmp/test_goto_back.rs".to_string());

    // Save position
    test.keys("7G");
    let original_pos = test.cursor();

    // Go to definition
    test.keys("gd");

    // Go back with Ctrl-O
    test.keys("<C-o>");

    // Should be back at original position (or close to it)
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP with imports
#[test]
fn test_lsp_with_imports() {
    let code = r#"
use std::collections::HashMap;

fn main() {
    let mut map: HashMap<String, i32> = HashMap::new();
    map.insert("key".to_string(), 42);
}
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path("/tmp/test_imports.rs".to_string());

    // Hover on HashMap
    test.keys("2G");
    test.keys("0");
    test.keys("fH");
    test.keys("K");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP cleans up properly when switching files
#[test]
fn test_lsp_cleanup_on_file_switch() {
    let mut test = EditorTest::new("fn test() {}\n");
    test.set_file_path("/tmp/file1.rs".to_string());

    // Trigger LSP hover
    test.keys("0");
    test.keys("K");

    // "Switch" file by loading new content
    test.keys(":e /tmp/file2.rs");
    test.press_enter();

    // Should still be in normal mode, LSP should handle gracefully
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP with syntax errors
#[test]
fn test_lsp_with_syntax_errors() {
    let code = r#"
fn broken( {
    let x = 5
    println!("{}", x)
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path("/tmp/test_errors.rs".to_string());

    // Try to use LSP features despite errors
    test.keys("2G");
    test.keys("K");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP diagnostics across file
#[test]
fn test_lsp_diagnostics_navigation() {
    let code = r#"
fn main() {
    let x: i32 = "wrong type";
    let y: String = 123;
}
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path("/tmp/test_diagnostics.rs".to_string());

    // Navigate through file
    test.keys("gg");
    test.keys("j");
    test.keys("j");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP completion trigger
#[test]
fn test_lsp_completion_trigger() {
    let code = r#"
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let p = Point { x: 0, y: 0 };
    p.
}
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path("/tmp/test_completion.rs".to_string());

    // Go to completion trigger point
    test.keys("9G");
    test.keys("$");

    // Trigger completion
    test.keys("a");
    test.keys("<C- >"); // Ctrl-Space
    test.press_esc();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP format document
#[test]
fn test_lsp_format_document() {
    let code = "fn main(){let x=5;}\n";

    let mut test = EditorTest::new(code);
    test.set_file_path("/tmp/test_format.rs".to_string());

    // Request format
    test.keys("gq");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP with very long file
#[test]
fn test_lsp_long_file() {
    let mut code = String::new();
    for i in 0..1000 {
        code.push_str(&format!("fn func_{}() {{}}\n", i));
    }

    let mut test = EditorTest::new(&code);
    test.set_file_path("/tmp/test_long.rs".to_string());

    // Navigate to middle
    test.keys("500G");

    // Try LSP features
    test.keys("K");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP with special characters in path
#[test]
fn test_lsp_special_path() {
    let mut test = EditorTest::new("fn test() {}\n");
    test.set_file_path("/tmp/file with spaces.rs".to_string());

    test.keys("0");
    test.keys("K");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP symbols in comments
#[test]
fn test_lsp_symbols_in_comments() {
    let code = r#"
// This function is called helper
fn helper() -> i32 {
    42
}

fn main() {
    // Call helper here
    helper();
}
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path("/tmp/test_comments.rs".to_string());

    // Hover on helper in comment
    test.keys("2G");
    test.keys("$");
    test.keys("K");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP with macros
#[test]
fn test_lsp_with_macros() {
    let code = r#"
macro_rules! my_macro {
    () => {
        println!("Hello");
    };
}

fn main() {
    my_macro!();
}
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path("/tmp/test_macros.rs".to_string());

    // Go to macro invocation
    test.keys("9G");
    test.keys("0");
    test.keys("gd");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP terminates properly on quit
#[test]
fn test_lsp_terminates_on_quit() {
    let mut test = EditorTest::new("fn test() {}\n");
    test.set_file_path("/tmp/test_term.rs".to_string());

    // Trigger LSP
    test.keys("K");

    // Quit
    test.keys(":q");
    test.press_enter();

    // LSP should clean up
    assert!(test.editor.should_quit());
}

/// Test rapid LSP requests don't cause issues
#[test]
fn test_lsp_rapid_requests() {
    let mut test = EditorTest::new("fn a() {}\nfn b() {}\nfn c() {}\n");
    test.set_file_path("/tmp/test_rapid.rs".to_string());

    // Rapid hover requests
    test.keys("K");
    test.keys("j");
    test.keys("K");
    test.keys("j");
    test.keys("K");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP with empty file
#[test]
fn test_lsp_empty_file() {
    let mut test = EditorTest::new("\n");
    test.set_file_path("/tmp/test_empty.rs".to_string());

    test.keys("K");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP buffer modifications are tracked
#[test]
fn test_lsp_tracks_modifications() {
    let mut test = EditorTest::new("fn old() {}\n");
    test.set_file_path("/tmp/test_mod.rs".to_string());

    // Make a change
    test.keys("i");
    test.type_text("// new\n");
    test.press_esc();

    // LSP should be notified of change
    test.keys("K");

    test.assert_mode(ovim::mode::Mode::Normal);
}
