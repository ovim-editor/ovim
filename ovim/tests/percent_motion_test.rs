mod helpers;
use helpers::EditorTest;

// ============================================================================
// % motion from OPENING delimiters (should work - baseline)
// ============================================================================

#[test]
fn percent_forward_parens() {
    let mut test = EditorTest::new("(hello)");
    // Cursor starts at col 0, which is '('
    test.press('%');
    test.assert_cursor(0, 6); // Should jump to ')'
}

#[test]
fn percent_forward_brackets() {
    let mut test = EditorTest::new("[hello]");
    test.press('%');
    test.assert_cursor(0, 6);
}

#[test]
fn percent_forward_braces() {
    let mut test = EditorTest::new("{hello}");
    test.press('%');
    test.assert_cursor(0, 6);
}

#[test]
fn percent_forward_nested() {
    let mut test = EditorTest::new("((inner))");
    test.press('%');
    test.assert_cursor(0, 8); // Outer ')'
}

// ============================================================================
// % motion from CLOSING delimiters (reported buggy)
// ============================================================================

#[test]
fn percent_backward_parens() {
    let mut test = EditorTest::new("(hello)");
    test.keys("$"); // Move to last char ')'
    test.press('%');
    test.assert_cursor(0, 0); // Should jump to '('
}

#[test]
fn percent_backward_brackets() {
    let mut test = EditorTest::new("[hello]");
    test.keys("$");
    test.press('%');
    test.assert_cursor(0, 0);
}

#[test]
fn percent_backward_braces() {
    let mut test = EditorTest::new("{hello}");
    test.keys("$");
    test.press('%');
    test.assert_cursor(0, 0);
}

#[test]
fn percent_backward_nested_outer() {
    let mut test = EditorTest::new("((inner))");
    test.keys("$"); // Last char is outer ')'
    test.press('%');
    test.assert_cursor(0, 0); // Should jump to outer '('
}

#[test]
fn percent_backward_nested_inner() {
    let mut test = EditorTest::new("((inner))");
    test.keys("f)"); // First ')' at col 7
    test.press('%');
    test.assert_cursor(0, 1); // Should jump to inner '('
}

// ============================================================================
// % motion from closing delimiters - multiline
// ============================================================================

#[test]
fn percent_backward_multiline_parens() {
    let mut test = EditorTest::new("(\n  hello\n)");
    test.keys("jj0"); // Go to line 2 (0-indexed), col 0 = ')'
    test.press('%');
    test.assert_cursor(0, 0); // Should jump to '(' on line 0
}

#[test]
fn percent_backward_multiline_braces() {
    let mut test = EditorTest::new("{\n  hello\n}");
    test.keys("jj0");
    test.press('%');
    test.assert_cursor(0, 0);
}

// ============================================================================
// % roundtrip: open -> close -> open
// ============================================================================

#[test]
fn percent_roundtrip_parens() {
    let mut test = EditorTest::new("(hello)");
    test.press('%'); // ( -> )
    test.assert_cursor(0, 6);
    test.press('%'); // ) -> (
    test.assert_cursor(0, 0);
}

#[test]
fn percent_roundtrip_braces() {
    let mut test = EditorTest::new("{hello}");
    test.press('%');
    test.assert_cursor(0, 6);
    test.press('%');
    test.assert_cursor(0, 0);
}

#[test]
fn percent_roundtrip_brackets() {
    let mut test = EditorTest::new("[hello]");
    test.press('%');
    test.assert_cursor(0, 6);
    test.press('%');
    test.assert_cursor(0, 0);
}

// ============================================================================
// Edge cases from closing delimiters
// ============================================================================

#[test]
fn percent_backward_adjacent_parens() {
    let mut test = EditorTest::new("()");
    test.keys("$"); // On ')'
    test.press('%');
    test.assert_cursor(0, 0);
}

#[test]
fn percent_backward_with_prefix_text() {
    let mut test = EditorTest::new("fn foo(bar)");
    test.keys("$"); // On ')'
    test.press('%');
    test.assert_cursor(0, 6); // Should jump to '('
}

#[test]
fn percent_backward_closing_at_start_of_line() {
    let mut test = EditorTest::new("if (\n  true\n)");
    test.keys("jj0"); // Line 2 (0-indexed), col 0 = ')'
    test.press('%');
    test.assert_cursor(0, 3); // Should jump to '(' on line 0, col 3
}

#[test]
fn percent_backward_mixed_brackets() {
    // Test that different bracket types don't interfere
    let mut test = EditorTest::new("([inner])");
    test.keys("$"); // On ')'
    test.press('%');
    test.assert_cursor(0, 0); // Should match outer '('
}

#[test]
fn percent_backward_deep_nesting() {
    let mut test = EditorTest::new("(((deep)))");
    test.keys("$"); // On outermost ')'
    test.press('%');
    test.assert_cursor(0, 0);
}

#[test]
fn percent_backward_with_content_after() {
    let mut test = EditorTest::new("(hello) world");
    test.keys("f)"); // Move to ')'
    test.press('%');
    test.assert_cursor(0, 0);
}

// ============================================================================
// Angle brackets: Vim's % does NOT match < and >
// ============================================================================

#[test]
fn percent_no_match_on_angle_bracket_close() {
    let mut test = EditorTest::new("<hello>");
    test.keys("$"); // On '>'
    test.press('%');
    // Vim: % doesn't match angle brackets, cursor stays
    test.assert_cursor(0, 6);
}

#[test]
fn percent_no_match_on_angle_bracket_open() {
    let mut test = EditorTest::new("<hello>");
    test.press('%');
    // Vim: % doesn't match angle brackets, cursor stays
    test.assert_cursor(0, 0);
}

// ============================================================================
// v% - visual mode with % motion
// ============================================================================

#[test]
fn v_percent_forward_from_open_paren() {
    // v% from '(' should select to matching ')'
    let mut test = EditorTest::new("(hello)");
    test.keys("v%");
    // Cursor should be on ')'
    test.assert_cursor(0, 6);
}

#[test]
fn v_percent_backward_from_close_paren() {
    // v% from ')' should select back to matching '('
    let mut test = EditorTest::new("(hello)");
    test.keys("$v%");
    // Cursor should be on '('
    test.assert_cursor(0, 0);
}

#[test]
fn v_percent_forward_from_open_brace() {
    let mut test = EditorTest::new("{hello}");
    test.keys("v%");
    test.assert_cursor(0, 6);
}

#[test]
fn v_percent_backward_from_close_brace() {
    let mut test = EditorTest::new("{hello}");
    test.keys("$v%");
    test.assert_cursor(0, 0);
}

#[test]
fn v_percent_multiline_backward() {
    let mut test = EditorTest::new("{\n  hello\n}");
    test.keys("jj0v%"); // On '}', enter visual, jump to '{'
    test.assert_cursor(0, 0);
}

#[test]
fn v_percent_roundtrip() {
    // v% from '(' to ')', then % back to '('
    let mut test = EditorTest::new("(hello)");
    test.keys("v%");
    test.assert_cursor(0, 6);
    test.press('%');
    test.assert_cursor(0, 0);
}

// ============================================================================
// Visual yank moves cursor to start of selection
// ============================================================================

#[test]
fn v_yank_moves_cursor_to_start() {
    let mut test = EditorTest::new("hello world");
    test.keys("vey"); // Select "hello", yank
    test.assert_cursor(0, 0); // Cursor should be at start of selection
}

#[test]
fn v_yank_backward_selection() {
    let mut test = EditorTest::new("hello world");
    test.keys("$vby"); // Move to end, select backward, yank
    test.assert_cursor(0, 6); // Cursor at start of "world"
}

#[test]
fn v_percent_yank_from_close() {
    let mut test = EditorTest::new("(hello)");
    test.keys("$v%y"); // On ')', visual, % to '(', yank
    test.assert_cursor(0, 0); // Cursor at '('
}

// ============================================================================
// Realistic code patterns - closing delimiters
// ============================================================================

#[test]
fn percent_backward_rust_fn() {
    // Real Rust code pattern: cursor on closing brace of function body
    let mut test = EditorTest::new("fn main() {\n    println!(\"hi\");\n}");
    test.keys("jj0"); // Line 2, col 0 = '}'
    test.press('%');
    test.assert_cursor(0, 10); // Should jump to '{' on line 0
}

#[test]
fn percent_backward_rust_nested() {
    // Nested blocks
    let mut test = EditorTest::new("{\n  {\n    inner\n  }\n}");
    test.keys("jjjj0"); // Line 4 = outer '}'
    test.press('%');
    test.assert_cursor(0, 0); // Should jump to outer '{'
}

#[test]
fn percent_backward_rust_inner_block() {
    // Cursor on inner closing brace
    let mut test = EditorTest::new("{\n  {\n    inner\n  }\n}");
    test.keys("jjj0f}"); // Line 3, find '}'
    test.press('%');
    test.assert_cursor(1, 2); // Should jump to inner '{' at line 1, col 2
}

#[test]
fn percent_backward_unbalanced_no_match() {
    // Unbalanced: extra closing bracket, no match
    let mut test = EditorTest::new("hello)");
    test.keys("$"); // On ')'
    test.press('%');
    test.assert_cursor(0, 5); // Should stay put - no matching '('
}

#[test]
fn percent_backward_closing_bracket_at_col_0() {
    // Closing bracket is the very first character of the buffer
    let mut test = EditorTest::new(")");
    test.press('%');
    test.assert_cursor(0, 0); // Should stay put - no matching '('
}

// ============================================================================
// d% operator from closing delimiters
// ============================================================================

#[test]
fn d_percent_from_closing_paren() {
    let mut test = EditorTest::new("(hello)");
    test.keys("$"); // On ')'
    test.keys("d%"); // Delete from ')' back to '(' (inclusive)
    test.assert_cursor(0, 0);
    // Should delete "(hello)" entirely
    assert_eq!(test.buffer_content(), "\n");
}

#[test]
fn d_percent_from_closing_brace_multiline() {
    let mut test = EditorTest::new("{\n  hello\n}");
    test.keys("jj0"); // On '}'
    test.keys("d%"); // Delete from '}' back to '{'
    test.assert_cursor(0, 0);
    // Should delete everything from '{' through '}'
    assert_eq!(test.buffer_content(), "\n");
}
