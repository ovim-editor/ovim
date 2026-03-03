mod helpers;
use helpers::EditorTest;

// ============================================================================
// 'q' command - Record macro
// ============================================================================

#[test]
fn test_q_basic_record() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('q') // Start recording
        .press('a') // To register 'a'
        .keys("dd") // Delete line
        .press('q'); // Stop recording

    assert_eq!(test.buffer_content(), "line 2\nline 3\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_q_record_and_playback() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('q')
        .press('a')
        .keys("dd") // Record: delete line
        .press('q')
        .press('@') // Playback
        .press('a');

    assert_eq!(test.buffer_content(), "line 3\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_q_record_multiple_commands() {
    let mut test = EditorTest::new("hello world\ntest line");

    test.press('q')
        .press('a')
        .press('i') // Enter insert mode
        .type_text("PREFIX: ")
        .press_esc()
        .press('j') // Move down
        .press('q') // Stop recording
        .press('@')
        .press('a'); // Replay on second line

    assert_eq!(
        test.buffer_content(),
        "PREFIX: hello world\ntest liPREFIX: ne\n"
    );
    test.assert_cursor(1, 14);
}

#[test]
fn test_q_record_change_operation() {
    let mut test = EditorTest::new("one\ntwo\nthree");

    test.press('q')
        .press('a')
        .keys("ciw") // Change word
        .type_text("X")
        .press_esc()
        .press('j')
        .press('q')
        .press('@')
        .press('a');

    assert_eq!(test.buffer_content(), "X\nX\nthree\n");
    test.assert_cursor(2, 0);
}

// ============================================================================
// '@' command - Play macro
// ============================================================================

#[test]
fn test_at_playback_simple() {
    let mut test = EditorTest::new("a\nb\nc\nd");

    test.press('q')
        .press('a')
        .press('x') // Delete char
        .press('j') // Move down
        .press('q')
        .press('@')
        .press('a') // Play once
        .press('@')
        .press('a'); // Play again

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
        .keys("3@a"); // Play 3 times

    // Recording phase deletes 'a' (iter 0), then 3@a deletes b,c,d (iters 1-3)
    assert_eq!(test.buffer_content(), "\n\n\n\ne\n");
    test.assert_cursor(4, 0);
}

#[test]
fn test_at_at_repeat_last() {
    let mut test = EditorTest::new("a\nb\nc\nd");

    // Recording deletes 'a', @a deletes 'b', @@ deletes 'c'
    test.press('q')
        .press('a')
        .press('x')
        .press('j')
        .press('q')
        .press('@')
        .press('a') // Play macro
        .press('@')
        .press('@'); // Repeat last macro with @@

    assert_eq!(test.buffer_content(), "\n\n\nd\n");
    test.assert_cursor(3, 0);
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
        .press('I') // Insert at beginning
        .type_text("A: ")
        .press_esc()
        .press('j')
        .press('q');

    // Record macro in register 'b'
    test.press('q')
        .press('b')
        .press('A') // Append at end
        .type_text(" [END]")
        .press_esc()
        .press('k')
        .press('q');

    // Play both
    test.press('@').press('a').press('@').press('b');

    assert_eq!(
        test.buffer_content(),
        "A: A: line 1\nline 2 [END] [END]\nline 3\n"
    );
    test.assert_cursor(0, 11);
}

#[test]
fn test_overwrite_macro_register() {
    let mut test = EditorTest::new("test");

    // Record first macro
    test.press('q').press('a').press('x').press('q');

    // Overwrite with new macro
    test.press('q')
        .press('a')
        .press('i')
        .type_text("NEW ")
        .press_esc()
        .press('q');

    // Play - should execute new macro
    test.press('@').press('a');

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
        .press('x') // Delete char
        .press('j') // Move down
        .press('@') // Call self
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
        .keys("diw") // Delete inner word
        .press('j')
        .press('q')
        .press('@')
        .press('a')
        .press('@')
        .press('a');

    assert_eq!(test.buffer_content(), " two\n four\n six\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_macro_with_search() {
    let mut test = EditorTest::new("hello world hello test");

    test.press('q')
        .press('a')
        .press('/') // Search
        .type_text("hello")
        .press_enter()
        .press('x') // Delete first char
        .press('q')
        .press('@')
        .press('a'); // Play - should find next and delete

    assert_eq!(test.buffer_content(), "ello world ello test\n");
    test.assert_cursor(0, 11);
}

// ============================================================================
// Macros with visual mode
// ============================================================================

#[test]
fn test_macro_with_visual_mode() {
    let mut test = EditorTest::new("hello\nworld\ntest");

    test.press('q')
        .press('a')
        .press('v') // Visual mode
        .keys("e") // Select to end of word
        .press('d') // Delete
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
        .press('V') // Visual line
        .press('d') // Delete line
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
        .press('A') // Append at end
        .type_text(" - done")
        .press_esc()
        .press('j')
        .press('q')
        .press('@')
        .press('a')
        .press('@')
        .press('a');

    assert_eq!(
        test.buffer_content(),
        "line 1 - done\nline 2 - done\nline 3 - done\n"
    );
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

    test.press('@').press('a');

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
        .keys("yy") // Yank line
        .press('j')
        .press('p') // Paste
        .press('q')
        .press('@')
        .press('a');

    assert_eq!(test.buffer_content(), "copy\nline 2\ncopy\nline 3\ncopy\n");
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
        .press('a') // Play macro
        .press('u'); // Undo macro

    assert_eq!(test.buffer_content(), "\nb\nc\n");
    test.assert_cursor(1, 0);
}

#[test]
fn test_macro_with_undo_inside() {
    let mut test = EditorTest::new("hello world");

    test.press('q')
        .press('a')
        .press('x') // Delete
        .press('u') // Undo inside macro
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
        .press('q') // Immediately stop
        .press('@')
        .press('a'); // Play empty macro

    assert_eq!(test.buffer_content(), "test\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_macro_stop_without_start() {
    let mut test = EditorTest::new("test");

    test.press('q'); // Press 'q' but don't record

    assert_eq!(test.buffer_content(), "test\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_macro_at_eof() {
    let mut test = EditorTest::new("a\nb");

    test.press('q')
        .press('a')
        .press('j') // Move down
        .press('q')
        .press('@')
        .press('a') // Try to play - should hit EOF
        .press('@')
        .press('a'); // Play again

    assert_eq!(test.buffer_content(), "a\nb\n");
    test.assert_cursor(1, 0);
}

#[test]
fn test_macro_uppercase_register() {
    let mut test = EditorTest::new("test");

    // Record to 'a'
    test.press('q').press('a').press('x').press('q');

    // Append to 'a' using 'A'
    test.press('q')
        .press('A') // Uppercase appends
        .press('x')
        .press('q');

    // Play combined macro
    test.press('@').press('a');

    assert_eq!(test.buffer_content(), "st\n");
    test.assert_cursor(0, 1);
}

// ============================================================================
// Macros with counts and repeats
// ============================================================================

#[test]
fn test_macro_with_count_inside() {
    let mut test = EditorTest::new("abcdefgh");

    test.press('q')
        .press('a')
        .keys("3x") // Delete 3 chars
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
        .keys("dw") // Delete word: "one " → "two three"
        .press('q')
        .press('@')
        .press('a') // Replay dw: "two " → "three"
        .press('.'); // Repeat dw (re-evaluates at cursor): "three" → ""

    assert_eq!(test.buffer_content(), "\n");
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
        .press('q') // This should stop recording, not start nested
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
        .keys("dd") // Delete line
        .press('q')
        .keys("3@a"); // Delete 3 more lines

    assert_eq!(test.buffer_content(), "\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_macro_with_o_command() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('q')
        .press('a')
        .press('o') // Open line below
        .type_text("inserted")
        .press_esc()
        .press('j')
        .press('q')
        .press('@')
        .press('a');

    assert_eq!(
        test.buffer_content(),
        "line 1\ninserted\nline 2\ninserted\n"
    );
    test.assert_cursor(3, 7);
}

// ============================================================================
// Macro abort on failed motion (Vim parity)
// ============================================================================

#[test]
fn test_macro_aborts_on_failed_j_at_eof() {
    // qq0i><Esc>jq then 1000@q on a 5-line file
    // Should insert '>' on each line and stop at the last line
    let mut test = EditorTest::new("aaa\nbbb\nccc\nddd\neee");

    // Record macro: go to col 0, insert '>', escape, move down
    test.press('q')
        .press('q')
        .press('0')
        .press('i')
        .type_text(">")
        .press_esc()
        .press('j')
        .press('q');

    // First line already got '>' during recording, cursor is on line 1
    assert_eq!(test.buffer_content(), ">aaa\nbbb\nccc\nddd\neee\n");

    // Play 1000 times — should stop after 4 more iterations (lines 1-4)
    test.keys("1000@q");

    assert_eq!(
        test.buffer_content(),
        ">aaa\n>bbb\n>ccc\n>ddd\n>eee\n"
    );
    // Cursor should be on the last line
    assert_eq!(test.editor.buffer().cursor().line(), 4);
}

#[test]
fn test_macro_aborts_counted_at_eof() {
    // 10@q near the end — should not repeat on last line
    let mut test = EditorTest::new("aa\nbb\ncc");

    // Record macro: insert '>' at start, move down
    test.press('q')
        .press('a')
        .press('0')
        .press('i')
        .type_text(">")
        .press_esc()
        .press('j')
        .press('q');

    // Line 0 got '>', cursor on line 1
    assert_eq!(test.buffer_content(), ">aa\nbb\ncc\n");

    // Play 10 times — only 2 lines left (1 and 2)
    test.keys("10@a");

    assert_eq!(test.buffer_content(), ">aa\n>bb\n>cc\n");
    // Last line should have exactly one '>'
}

#[test]
fn test_macro_abort_repeat_at_at() {
    // @@ should also respect abort
    let mut test = EditorTest::new("x\ny\nz\nw");

    test.press('q')
        .press('a')
        .press('x')
        .press('j')
        .press('q');

    // Recording deleted 'x' on line 0 and moved to line 1
    assert_eq!(test.buffer_content(), "\ny\nz\nw\n");

    // Play @a once: deletes 'y', moves to line 2
    test.keys("@a");
    assert_eq!(test.buffer_content(), "\n\nz\nw\n");

    // Now 100@@ — should play twice more (lines 2,3) then abort at EOF
    test.keys("100@@");
    assert_eq!(test.buffer_content(), "\n\n\n\n");
}

#[test]
fn test_macro_k_abort_at_first_line() {
    // Macro with k (move up) should abort at first line
    let mut test = EditorTest::new("aaa\nbbb\nccc");

    // Start at last line
    test.keys("G");

    // Record macro: insert '>' at start, move up
    test.press('q')
        .press('a')
        .press('0')
        .press('i')
        .type_text(">")
        .press_esc()
        .press('k')
        .press('q');

    assert_eq!(test.buffer_content(), "aaa\nbbb\n>ccc\n");

    // Play 100 times — should process lines 1 and 0 then stop
    test.keys("100@a");

    assert_eq!(test.buffer_content(), ">aaa\n>bbb\n>ccc\n");
}
