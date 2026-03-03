mod helpers;
use helpers::EditorTest;

// ============================================================================
// yl - Yank character(s) forward
// ============================================================================

#[test]
fn test_yl_basic() {
    let mut test = EditorTest::new("hello");
    test.keys("yl");
    assert_eq!(test.get_register_content('"'), Some("h".to_string()));
    // Buffer unchanged
    assert_eq!(test.buffer_content(), "hello\n");
    // Cursor stays
    test.assert_cursor(0, 0);
}

#[test]
fn test_yl_middle_of_line() {
    let mut test = EditorTest::new("hello");
    test.keys("llyl");
    assert_eq!(test.get_register_content('"'), Some("l".to_string()));
    test.assert_cursor(0, 2);
}

#[test]
fn test_y2l() {
    let mut test = EditorTest::new("hello");
    test.keys("2yl");
    assert_eq!(test.get_register_content('"'), Some("he".to_string()));
    test.assert_cursor(0, 0);
}

#[test]
fn test_y3l_from_middle() {
    let mut test = EditorTest::new("abcdef");
    test.keys("l3yl");
    assert_eq!(test.get_register_content('"'), Some("bcd".to_string()));
    test.assert_cursor(0, 1);
}

#[test]
fn test_yl_at_end_of_line() {
    let mut test = EditorTest::new("abc");
    test.keys("$yl");
    // On last char, should yank that char
    assert_eq!(test.get_register_content('"'), Some("c".to_string()));
}

#[test]
fn test_yl_clamps_to_line_end() {
    let mut test = EditorTest::new("ab");
    test.keys("10yl");
    // Should yank whole line (clamped)
    assert_eq!(test.get_register_content('"'), Some("ab".to_string()));
}

#[test]
fn test_yl_then_paste() {
    let mut test = EditorTest::new("abcdef");
    test.keys("ylp");
    // Yank 'a', paste after cursor → "aabcdef"
    assert_eq!(test.buffer_content(), "aabcdef\n");
}

// ============================================================================
// y% - Yank to matching bracket
// ============================================================================

#[test]
fn test_y_percent_parens() {
    let mut test = EditorTest::new("(hello)");
    test.keys("y%");
    assert_eq!(test.get_register_content('"'), Some("(hello)".to_string()));
    assert_eq!(test.buffer_content(), "(hello)\n");
}

#[test]
fn test_y_percent_brackets() {
    let mut test = EditorTest::new("[1, 2, 3]");
    test.keys("y%");
    assert_eq!(
        test.get_register_content('"'),
        Some("[1, 2, 3]".to_string())
    );
}

#[test]
fn test_y_percent_from_closing() {
    let mut test = EditorTest::new("(hello)");
    test.keys("$y%");
    assert_eq!(test.get_register_content('"'), Some("(hello)".to_string()));
}

#[test]
fn test_y_percent_no_bracket() {
    let mut test = EditorTest::new("hello");
    test.keys("y%");
    // No bracket under cursor — nothing should be yanked
    assert_eq!(test.get_register_content('"'), None);
}

// ============================================================================
// c% - Change to matching bracket
// ============================================================================

#[test]
fn test_c_percent_parens() {
    let mut test = EditorTest::new("(hello) world");
    test.keys("c%");
    // Should delete "(hello)" and enter insert mode
    assert_eq!(test.buffer_content(), " world\n");
    assert_eq!(test.editor.mode(), ovim::mode::Mode::Insert);
}

#[test]
fn test_c_percent_curly() {
    let mut test = EditorTest::new("fn { body } rest");
    test.keys("www"); // move to '{'
    test.keys("c%");
    assert_eq!(test.buffer_content(), "fn  rest\n");
    assert_eq!(test.editor.mode(), ovim::mode::Mode::Insert);
}

// ============================================================================
// Visual mode r{char} - Replace selection with character
// ============================================================================

#[test]
fn test_visual_r_basic() {
    let mut test = EditorTest::new("hello world");
    test.keys("vllllrx");
    // "hello" → "xxxxx"
    assert_eq!(test.buffer_content(), "xxxxx world\n");
    assert_eq!(test.editor.mode(), ovim::mode::Mode::Normal);
}

#[test]
fn test_visual_r_single_char() {
    let mut test = EditorTest::new("abcdef");
    test.keys("vrx");
    // Just 'a' → 'x'
    assert_eq!(test.buffer_content(), "xbcdef\n");
}

#[test]
fn test_visual_r_middle_of_line() {
    let mut test = EditorTest::new("hello world");
    test.keys("wvllr-");
    // "wor" → "---"
    assert_eq!(test.buffer_content(), "hello ---ld\n");
}

#[test]
fn test_visual_line_r() {
    let mut test = EditorTest::new("hello\nworld\n");
    test.keys("Vr-");
    // Entire first line replaced char-by-char with '-'
    assert_eq!(test.buffer_content(), "-----\nworld\n");
}

#[test]
fn test_visual_r_multiline() {
    let mut test = EditorTest::new("abc\ndef\n");
    test.keys("vjr*");
    // Select from 'a' through 'd' (across newline), replace non-newline chars
    // "abc\nd" → "***\n*"
    assert_eq!(test.buffer_content(), "***\n*ef\n");
}
