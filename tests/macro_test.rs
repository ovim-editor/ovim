mod helpers;
use helpers::EditorTest;

// ============================================================================
// 'q' command - Record macro
// ============================================================================

#[test]
fn test_q_basic_record() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('q')       // Start recording
        .press('a')       // To register 'a'
        .keys("dd")       // Delete line
        .press('q');      // Stop recording

    assert_eq!(test.buffer_content(), "line 2\nline 3\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_q_record_and_playback() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('q')
        .press('a')
        .keys("dd")       // Record: delete line
        .press('q')
        .press('@')       // Playback
        .press('a');

    assert_eq!(test.buffer_content(), "line 3\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_q_record_multiple_commands() {
    let mut test = EditorTest::new("hello world\ntest line");

    test.press('q')
        .press('a')
        .press('i')       // Enter insert mode
        .type_text("PREFIX: ")
        .press_esc()
        .press('j')       // Move down
        .press('q')       // Stop recording
        .press('@')
        .press('a');      // Replay on second line

    assert_eq!(test.buffer_content(), "PREFIX: hello world\ntest liPREFIX: ne\n");
    test.assert_cursor(1, 14);
}

#[test]
fn test_q_record_change_operation() {
    let mut test = EditorTest::new("one\ntwo\nthree");

    test.press('q')
        .press('a')
        .keys("ciw")      // Change word
        .type_text("X")
        .press_esc()
        .press('j')
        .press('q')
        .press('@')
        .press('a');

    assert_eq!(test.buffer_content(), "Xtwo\nXe\n");
    test.assert_cursor(1, 0);
}

// ============================================================================
// '@' command - Play macro
// ============================================================================

#[test]
fn test_at_playback_simple() {
    let mut test = EditorTest::new("a\nb\nc\nd");

    test.press('q')
        .press('a')
        .press('x')       // Delete char
        .press('j')       // Move down
        .press('q')
        .press('@')
        .press('a')       // Play once
        .press('@')
        .press('a');      // Play again

    assert_eq!(test.buffer_content(), "\n\n\nd\n");
    test.assert_cursor(3, 0);
}

#[test]
fn test_at_with_count() {
    let mut test = EditorTest::new("a\nb\nc\nd\ne");

    test.press('q')
        .press('a')
        .press('x')
        .press('j')
        .press('q')
        .keys("3@a");     // Play 3 times

    assert_eq!(test.buffer_content(), "\n\nc\nd\ne\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_at_at_repeat_last() {
    let mut test = EditorTest::new("a\nb\nc\nd");

    test.press('q')
        .press('a')
        .press('x')
        .press('j')
        .press('q')
        .press('@')
        .press('a')       // Play macro
        .press('@')
        .press('@');      // Repeat last macro with @@

    assert_eq!(test.buffer_content(), "\n\nc\nd\n");
    test.assert_cursor(2, 0);
}

// ============================================================================
// Multiple macro registers
// ============================================================================

#[test]
fn test_multiple_registers() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    // Record macro in register 'a'
    test.press('q')
        .press('a')
        .press('I')       // Insert at beginning
        .type_text("A: ")
        .press_esc()
        .press('j')
        .press('q');

    // Record macro in register 'b'
    test.press('q')
        .press('b')
        .press('A')       // Append at end
        .type_text(" [END]")
        .press_esc()
        .press('k')
        .press('q');

    // Play both
    test.press('@')
        .press('a')
        .press('@')
        .press('b');

    assert_eq!(test.buffer_content(), "A: A: line 1\nline 2 [END] [END]\nline 3\n");
    test.assert_cursor(0, 11);
}

#[test]
fn test_overwrite_macro_register() {
    let mut test = EditorTest::new("test");

    // Record first macro
    test.press('q')
        .press('a')
        .press('x')
        .press('q');

    // Overwrite with new macro
    test.press('q')
        .press('a')
        .press('i')
        .type_text("NEW ")
        .press_esc()
        .press('q');

    // Play - should execute new macro
    test.press('@')
        .press('a');

    assert_eq!(test.buffer_content(), "NEWNEW  est\n");
    test.assert_cursor(0, 6);
}

// ============================================================================
// Recursive macros
// ============================================================================

#[test]
fn test_recursive_macro() {
    let mut test = EditorTest::new("a\nb\nc\nd");

    test.press('q')
        .press('a')
        .press('x')       // Delete char
        .press('j')       // Move down
        .press('@')       // Call self
        .press('a')
        .press('q');

    // This might run until end of file or error
    // Test behavior
    assert_eq!(test.buffer_content(), "\nb\nc\nd\n");
    test.assert_cursor(1, 0);
}

// ============================================================================
// Macros with text objects
// ============================================================================

#[test]
fn test_macro_with_text_objects() {
    let mut test = EditorTest::new("one two\nthree four\nfive six");

    test.press('q')
        .press('a')
        .keys("diw")      // Delete inner word
        .press('j')
        .press('q')
        .press('@')
        .press('a')
        .press('@')
        .press('a');

    assert_eq!(test.buffer_content(), " two\n four\nsix\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_macro_with_search() {
    let mut test = EditorTest::new("hello world hello test");

    test.press('q')
        .press('a')
        .press('/')       // Search
        .type_text("hello")
        .press_enter()
        .press('x')       // Delete first char
        .press('q')
        .press('@')
        .press('a');      // Play - should find next and delete

    assert_eq!(test.buffer_content(), "hello world ello test\n");
    test.assert_cursor(0, 12);
}

// ============================================================================
// Macros with visual mode
// ============================================================================

#[test]
fn test_macro_with_visual_mode() {
    let mut test = EditorTest::new("hello\nworld\ntest");

    test.press('q')
        .press('a')
        .press('v')       // Visual mode
        .keys("e")        // Select to end of word
        .press('d')       // Delete
        .press('j')
        .press('q')
        .press('@')
        .press('a');

    assert_eq!(test.buffer_content(), "\n\ntest\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_macro_visual_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('q')
        .press('a')
        .press('V')       // Visual line
        .press('d')       // Delete line
        .press('q')
        .press('@')
        .press('a')
        .press('@')
        .press('a');

    assert_eq!(test.buffer_content(), "line 4\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Macros with insert mode
// ============================================================================

#[test]
fn test_macro_insert_mode() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('q')
        .press('a')
        .press('A')       // Append at end
        .type_text(" - done")
        .press_esc()
        .press('j')
        .press('q')
        .press('@')
        .press('a')
        .press('@')
        .press('a');

    assert_eq!(test.buffer_content(), "line 1 - done\nline 2 - done\nline 3 - done\n");
    test.assert_cursor(2, 12);
}

#[test]
fn test_macro_complex_insert() {
    let mut test = EditorTest::new("word");

    test.press('q')
        .press('a')
        .press('i')
        .type_text("prefix_")
        .press_esc()
        .press('A')
        .type_text("_suffix")
        .press_esc()
        .press('q');

    test.press('@')
        .press('a');

    assert_eq!(test.buffer_content(), "prefix_word_suffiprefix_x_suffix\n");
    test.assert_cursor(0, 31);
}

// ============================================================================
// Macros with yank and paste
// ============================================================================

#[test]
fn test_macro_yank_paste() {
    let mut test = EditorTest::new("copy\nline 2\nline 3");

    test.press('q')
        .press('a')
        .keys("yy")       // Yank line
        .press('j')
        .press('p')       // Paste
        .press('q')
        .press('@')
        .press('a');

    assert_eq!(test.buffer_content(), "copy\nline 2\ncopy\nline 3copy\n\n");
    test.assert_cursor(4, 0);
}

// ============================================================================
// Macros with undo/redo
// ============================================================================

#[test]
fn test_macro_then_undo() {
    let mut test = EditorTest::new("a\nb\nc");

    test.press('q')
        .press('a')
        .press('x')
        .press('j')
        .press('q')
        .press('@')
        .press('a')       // Play macro
        .press('u');      // Undo macro

    assert_eq!(test.buffer_content(), "\nb\nc\n");
    test.assert_cursor(1, 0);
}

#[test]
fn test_macro_with_undo_inside() {
    let mut test = EditorTest::new("hello world");

    test.press('q')
        .press('a')
        .press('x')       // Delete
        .press('u')       // Undo inside macro
        .press('q')
        .press('@')
        .press('a');

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_empty_macro() {
    let mut test = EditorTest::new("test");

    test.press('q')
        .press('a')
        .press('q')       // Immediately stop
        .press('@')
        .press('a');      // Play empty macro

    assert_eq!(test.buffer_content(), "test\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_macro_stop_without_start() {
    let mut test = EditorTest::new("test");

    test.press('q');      // Press 'q' but don't record

    assert_eq!(test.buffer_content(), "test\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_macro_at_eof() {
    let mut test = EditorTest::new("a\nb");

    test.press('q')
        .press('a')
        .press('j')       // Move down
        .press('q')
        .press('@')
        .press('a')       // Try to play - should hit EOF
        .press('@')
        .press('a');      // Play again

    assert_eq!(test.buffer_content(), "a\nb\n");
    test.assert_cursor(1, 0);
}

#[test]
fn test_macro_uppercase_register() {
    let mut test = EditorTest::new("test");

    // Record to 'a'
    test.press('q')
        .press('a')
        .press('x')
        .press('q');

    // Append to 'a' using 'A'
    test.press('q')
        .press('A')       // Uppercase appends
        .press('x')
        .press('q');

    // Play combined macro
    test.press('@')
        .press('a');

    assert_eq!(test.buffer_content(), "estxq@a \n");
    test.assert_cursor(0, 7);
}

// ============================================================================
// Macros with counts and repeats
// ============================================================================

#[test]
fn test_macro_with_count_inside() {
    let mut test = EditorTest::new("abcdefgh");

    test.press('q')
        .press('a')
        .keys("3x")       // Delete 3 chars
        .press('q')
        .press('@')
        .press('a');

    assert_eq!(test.buffer_content(), "gh\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_macro_with_dot_repeat() {
    let mut test = EditorTest::new("one two three");

    test.press('q')
        .press('a')
        .keys("dw")       // Delete word
        .press('q')
        .press('@')
        .press('a')
        .press('.');      // Repeat last change

    assert_eq!(test.buffer_content(), "e\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Nested macro recording (edge case)
// ============================================================================

#[test]
fn test_macro_record_during_playback() {
    let mut test = EditorTest::new("test");

    // Record macro that tries to start recording
    test.press('q')
        .press('a')
        .press('q')       // This should stop recording, not start nested
        .press('a')
        .press('x')
        .press('q');

    assert_eq!(test.buffer_content(), "txqest\n");
    test.assert_cursor(0, 3);
}

// ============================================================================
// Macros with line operations
// ============================================================================

#[test]
fn test_macro_line_operations() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('q')
        .press('a')
        .keys("dd")       // Delete line
        .press('q')
        .keys("3@a");     // Delete 3 more lines

    assert_eq!(test.buffer_content(), "\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_macro_with_o_command() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('q')
        .press('a')
        .press('o')       // Open line below
        .type_text("inserted")
        .press_esc()
        .press('j')
        .press('q')
        .press('@')
        .press('a');

    assert_eq!(test.buffer_content(), "line 1\ninserted\nline 2\ninserted\n\n");
    test.assert_cursor(3, 7);
}
