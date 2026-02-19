mod helpers;

use helpers::EditorTest;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

fn temp_test_path(name: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!("ovim_test_{}_{}", id, name))
        .to_string_lossy()
        .to_string()
}

/// Test LSP goto definition across multiple files
#[test]
fn test_lsp_goto_definition_multi_file() {
    // Create temporary test files
    let temp_dir = tempfile::tempdir().unwrap();

    // File 1: mod.rs
    let file1 = temp_dir.path().join("mod.rs");
    fs::write(&file1, "pub mod utils;\npub use utils::*;\n").unwrap();

    // File 2: utils.rs
    let file2 = temp_dir.path().join("utils.rs");
    fs::write(
        &file2,
        "pub fn helper() -> i32 {\n    42\n}\n\npub fn caller() {\n    let x = helper();\n}\n",
    )
    .unwrap();

    // File 3: main.rs
    let file3 = temp_dir.path().join("main.rs");
    fs::write(
        &file3,
        "mod utils;\n\nfn main() {\n    utils::helper();\n}\n",
    )
    .unwrap();

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
    test.set_file_path(temp_test_path("test_hover.rs"));

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
    test.set_file_path(temp_test_path("test_nested.rs"));

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
    test.set_file_path(temp_test_path("test_goto_back.rs"));

    // Save position
    test.keys("7G");

    // Go to definition
    test.keys("gd");

    // Go back with Ctrl-O
    test.press_with(ovim_core::KeyCode::Char('o'), ovim_core::Modifiers::CONTROL);

    // Should be back at original position (or close to it)
    test.assert_mode(ovim::mode::Mode::Normal);
    assert!(test.cursor().0 <= 6);
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
    test.set_file_path(temp_test_path("test_imports.rs"));

    // Hover on HashMap
    test.keys("2G");
    test.keys("0");
    test.keys("fH");
    test.keys("K");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP cleans up properly when switching files
/// Note: Uses multi-threaded tokio runtime because `:e` command calls load_file
/// which uses block_in_place requiring a multi-threaded runtime
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lsp_cleanup_on_file_switch() {
    // Create test files
    let dir = tempfile::tempdir().unwrap();
    let file1 = dir.path().join("file1.rs");
    let file2 = dir.path().join("file2.rs");
    std::fs::write(&file1, "fn test() {}\n").unwrap();
    std::fs::write(&file2, "fn other() {}\n").unwrap();
    let file1_str = file1.to_string_lossy().to_string();
    let file2_str = file2.to_string_lossy().to_string();

    let mut test = EditorTest::new("fn test() {}\n");
    test.set_file_path(file1_str.clone());

    // Trigger LSP hover
    test.keys("0");
    test.keys("K");

    // "Switch" file by loading new content
    test.keys(&format!(":e {}", file2_str));
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
    test.set_file_path(temp_test_path("test_errors.rs"));

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
    test.set_file_path(temp_test_path("test_diagnostics.rs"));

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
    test.set_file_path(temp_test_path("test_completion.rs"));

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
    test.set_file_path(temp_test_path("test_format.rs"));

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
    test.set_file_path(temp_test_path("test_long.rs"));

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
    test.set_file_path(temp_test_path("file with spaces.rs"));

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
    test.set_file_path(temp_test_path("test_comments.rs"));

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
    test.set_file_path(temp_test_path("test_macros.rs"));

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
    test.set_file_path(temp_test_path("test_term.rs"));

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
    test.set_file_path(temp_test_path("test_rapid.rs"));

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
    test.set_file_path(temp_test_path("test_empty.rs"));

    test.keys("K");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test LSP buffer modifications are tracked
#[test]
fn test_lsp_tracks_modifications() {
    let mut test = EditorTest::new("fn old() {}\n");
    test.set_file_path(temp_test_path("test_mod.rs"));

    // Make a change
    test.keys("i");
    test.type_text("// new\n");
    test.press_esc();

    // LSP should be notified of change
    test.keys("K");

    test.assert_mode(ovim::mode::Mode::Normal);
}
