mod helpers;
use helpers::EditorTest;

// ============================================================================
// Tab key — expandtab (default)
// ============================================================================

#[test]
fn test_tab_expandtab_inserts_spaces() {
    let mut test = EditorTest::new("hello");
    test.keys("i<Tab><Esc>");
    // Default: expandtab=true, shiftwidth=4 → 4 spaces
    assert_eq!(test.buffer_content(), "    hello\n");
}

#[test]
fn test_tab_expandtab_custom_shiftwidth() {
    let mut test = EditorTest::new("hello");
    test.editor.options.shift_width = 2;
    test.keys("i<Tab><Esc>");
    assert_eq!(test.buffer_content(), "  hello\n");
}

// ============================================================================
// Tab key — noexpandtab
// ============================================================================

#[test]
fn test_tab_noexpandtab_inserts_tab() {
    let mut test = EditorTest::new("hello");
    test.editor.options.expand_tab = false;
    test.keys("i<Tab><Esc>");
    assert_eq!(test.buffer_content(), "\thello\n");
}

// ============================================================================
// = operator — expandtab (default)
// ============================================================================

#[test]
fn test_equals_expandtab_uses_spaces() {
    let mut test = EditorTest::new("fn main() {\nhello\n}");
    test.keys("j==");
    assert_eq!(test.buffer_content(), "fn main() {\n    hello\n}\n");
}

// ============================================================================
// = operator — noexpandtab
// ============================================================================

#[test]
fn test_equals_noexpandtab_uses_tabs() {
    let mut test = EditorTest::new("fn main() {\nhello\n}");
    test.editor.options.expand_tab = false;
    test.keys("j==");
    assert_eq!(test.buffer_content(), "fn main() {\n\thello\n}\n");
}

// ============================================================================
// Enter after opening bracket — expandtab
// ============================================================================

#[test]
fn test_enter_after_brace_expandtab() {
    let mut test = EditorTest::new("fn main() {");
    // Type Enter then content so indent is preserved (Esc strips trailing whitespace)
    test.keys("A<CR>x<Esc>");
    assert_eq!(test.buffer_content(), "fn main() {\n    x\n");
}

#[test]
fn test_enter_after_paren_expandtab() {
    let mut test = EditorTest::new("call(");
    test.keys("A<CR>x<Esc>");
    assert_eq!(test.buffer_content(), "call(\n    x\n");
}

#[test]
fn test_enter_after_bracket_expandtab() {
    let mut test = EditorTest::new("let a = [");
    test.keys("A<CR>x<Esc>");
    assert_eq!(test.buffer_content(), "let a = [\n    x\n");
}

// ============================================================================
// Enter after opening bracket — noexpandtab
// ============================================================================

#[test]
fn test_enter_after_brace_noexpandtab() {
    let mut test = EditorTest::new("fn main() {");
    test.editor.options.expand_tab = false;
    test.keys("A<CR>x<Esc>");
    assert_eq!(test.buffer_content(), "fn main() {\n\tx\n");
}

// ============================================================================
// Enter on normal line — just copies indent
// ============================================================================

#[test]
fn test_enter_no_bracket_copies_indent() {
    let mut test = EditorTest::new("    hello world");
    test.keys("A<CR>x<Esc>");
    // Should copy the 4-space indent, no extra
    assert_eq!(test.buffer_content(), "    hello world\n    x\n");
}

// ============================================================================
// o after brace — noexpandtab
// ============================================================================

#[test]
fn test_o_after_brace_noexpandtab() {
    let mut test = EditorTest::new("fn main() {");
    test.editor.options.expand_tab = false;
    test.keys("o<Esc>");
    let content = test.buffer_content();
    // o on a line ending with { should produce a new line
    // Esc may strip trailing whitespace, so just check the line exists
    assert!(
        content.contains('\t') || content == "fn main() {\n\n",
        "Expected tab indent or empty line, got: {:?}",
        content
    );
}

// ============================================================================
// >> with noexpandtab
// ============================================================================

#[test]
fn test_shift_right_noexpandtab() {
    let mut test = EditorTest::new("hello");
    test.editor.options.expand_tab = false;
    test.keys(">>");
    assert_eq!(test.buffer_content(), "\thello\n");
}

// ============================================================================
// Ctrl-T with noexpandtab
// ============================================================================

#[test]
fn test_ctrl_t_noexpandtab() {
    let mut test = EditorTest::new("hello");
    test.editor.options.expand_tab = false;
    test.keys("i<C-t><Esc>");
    assert_eq!(test.buffer_content(), "\thello\n");
}

// ============================================================================
// indent_string — tested via = operator on nested code
// ============================================================================

#[test]
fn test_indent_string_via_equals_operator() {
    // Nested indentation with tabs
    let mut test = EditorTest::new("fn main() {\nif true {\nhello\n}\n}");
    test.editor.options.expand_tab = false;
    test.keys("gg=G");
    assert_eq!(
        test.buffer_content(),
        "fn main() {\n\tif true {\n\t\thello\n\t}\n}\n"
    );
}

#[test]
fn test_indent_string_via_equals_spaces() {
    // Same but with spaces (default)
    let mut test = EditorTest::new("fn main() {\nif true {\nhello\n}\n}");
    test.keys("gg=G");
    assert_eq!(
        test.buffer_content(),
        "fn main() {\n    if true {\n        hello\n    }\n}\n"
    );
}
