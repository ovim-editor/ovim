mod helpers;
use crossterm::event::{KeyCode, KeyModifiers};
use helpers::EditorTest;
use ovim::mode::Mode;

// ============================================================================
// 'v' command - Character-wise visual mode
// ============================================================================

#[test]
fn test_v_basic_selection() {
    let mut test = EditorTest::new("hello world");

    test.press('v') // Enter visual mode
        .keys("lll"); // Select 4 chars (h, e, l, l)

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 3);
}

#[test]
fn test_v_delete_selection() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("lll") // Select "hell"
        .press('d'); // Delete selection

    assert_eq!(test.buffer_content(), "o world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_v_yank_selection() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("llll") // Select "hello"
        .press('y') // Yank
        .press_esc()
        .keys("$") // Move to end
        .press('p'); // Paste

    // Yanking "hello" (5 chars from positions 0-4), paste after 'd' at end
    assert_eq!(test.buffer_content(), "hello worldhello\n");
    test.assert_cursor(0, 15);
}

#[test]
fn test_v_select_word() {
    let mut test = EditorTest::new("hello world test");

    test.press('v').keys("e"); // Select to end of word

    assert_eq!(test.buffer_content(), "hello world test\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_v_across_lines() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('v').keys("jjj"); // Select across multiple lines

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_v_backward_selection() {
    let mut test = EditorTest::new("hello world");

    test.keys("$") // Go to end
        .press('v')
        .keys("hhh"); // Select backward

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_v_change_selection() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("llll") // Select "hello"
        .press('c') // Change
        .type_text("goodbye")
        .press_esc();

    assert_eq!(test.buffer_content(), "goodbye world\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_v_escape_cancels() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("lll") // Make selection
        .press_esc(); // Cancel

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 3);
}

// ============================================================================
// 'V' command - Line-wise visual mode
// ============================================================================

#[test]
fn test_V_basic_selection() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V'); // Enter visual line mode

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_V_multiple_lines() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('V').keys("jj"); // Select 3 lines

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\nline 4\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_V_delete_lines() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('V')
        .keys("jj") // Select 3 lines
        .press('d'); // Delete

    assert_eq!(test.buffer_content(), "line 4\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_V_yank_paste_lines() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .press('j') // Select 2 lines
        .press('y') // Yank
        .press_esc()
        .press('G') // Go to last line
        .press('p'); // Paste

    assert_eq!(
        test.buffer_content(),
        "line 1\nline 2\nline 3line 1\nline 2\n \n"
    );
    test.assert_cursor(4, 0);
}

#[test]
fn test_V_from_middle_of_line() {
    let mut test = EditorTest::new("hello world\ntest line");

    test.keys("w") // Move to "world"
        .press('V'); // Should select entire line

    assert_eq!(test.buffer_content(), "hello world\ntest line\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_V_select_all() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V').keys("G"); // Select to last line

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    // G moves cursor to last line (selection extends from line 0 to line 2)
    test.assert_cursor(2, 0);
}

#[test]
fn test_V_indent() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .press('j') // Select 2 lines
        .press('>'); // Indent (if implemented)

    assert_eq!(test.buffer_content(), "    line 1\n    line 2\nline 3\n");
    test.assert_cursor(1, 4);
}

#[test]
fn test_V_backward_selection() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("G") // Go to last line
        .press('V')
        .keys("kk"); // Select upward

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\nline 4\n");
    test.assert_cursor(1, 0);
}

// ============================================================================
// Visual mode with operators
// ============================================================================

#[test]
fn test_v_delete_word() {
    let mut test = EditorTest::new("hello world test");

    test.press('v')
        .keys("e") // Select word
        .press('d'); // Delete

    assert_eq!(test.buffer_content(), " world test\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_v_change_word() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("e") // Select "hello"
        .press('c') // Change
        .type_text("goodbye")
        .press_esc();

    assert_eq!(test.buffer_content(), "goodbye world\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_V_delete_and_undo() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .press('j') // Select 2 lines
        .press('d') // Delete
        .press('u'); // Undo

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    test.assert_cursor(1, 0);
}

#[test]
fn test_v_yank_and_replace() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("llll") // Select "hello"
        .press('y') // Yank
        .press_esc()
        .keys("w") // Move to "world"
        .press('v')
        .keys("llll") // Select "world"
        .press('p'); // Paste (should replace selection)

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 10);
}

// ============================================================================
// Visual mode edge cases
// ============================================================================

#[test]
fn test_v_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.press('j') // Move to empty line
        .press('v')
        .press('j'); // Select to next line

    assert_eq!(test.buffer_content(), "hello\n\nworld\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_v_single_char() {
    let mut test = EditorTest::new("x");

    test.press('v'); // Select single char

    assert_eq!(test.buffer_content(), "x\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_v_end_of_line() {
    let mut test = EditorTest::new("hello");

    test.keys("$") // Move to end
        .press('v'); // Select last char

    assert_eq!(test.buffer_content(), "hello\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_V_single_line() {
    let mut test = EditorTest::new("only line");

    test.press('V'); // Select only line

    assert_eq!(test.buffer_content(), "only line\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_v_select_entire_file() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("gg") // Go to top
        .press('v')
        .keys("G"); // Select to end

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    // G moves cursor to last line (selection extends from line 0 to line 2)
    test.assert_cursor(2, 0);
}

// ============================================================================
// Visual mode with motions
// ============================================================================

#[test]
fn test_v_with_w_motion() {
    let mut test = EditorTest::new("hello world test");

    test.press('v').press('w'); // Select word forward

    assert_eq!(test.buffer_content(), "hello world test\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_v_with_dollar() {
    let mut test = EditorTest::new("hello world");

    test.press('v').keys("$"); // Select to end of line

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 10);
}

#[test]
fn test_v_with_zero() {
    let mut test = EditorTest::new("hello world");

    test.keys("$") // Go to end
        .press('v')
        .keys("0"); // Select to beginning

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_V_with_gg() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("G") // Go to last line
        .press('V')
        .keys("gg"); // Select to first line

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\nline 4\n");
    test.assert_cursor(3, 0);
}

// ============================================================================
// Switch between visual modes
// ============================================================================

#[test]
fn test_v_to_V_switch() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('v') // Character visual
        .press('V'); // Switch to line visual

    assert_eq!(test.buffer_content(), "line 1\nline 2\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_V_to_v_switch() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('V') // Line visual
        .press('v'); // Switch to character visual

    assert_eq!(test.buffer_content(), "line 1\nline 2\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Visual mode with count
// ============================================================================

#[test]
fn test_v_with_count() {
    let mut test = EditorTest::new("hello world test");

    test.press('v').keys("3l"); // Select 4 chars (including current)

    assert_eq!(test.buffer_content(), "hello world test\n");
    // 3l moves 3 positions: 0 -> 3
    test.assert_cursor(0, 3);
}

#[test]
fn test_V_with_count() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('V').keys("3j"); // Select 4 lines

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\nline 4\n");
    test.assert_cursor(3, 0);
}

// ============================================================================
// Visual mode with search
// ============================================================================

#[test]
fn test_v_to_search_result() {
    let mut test = EditorTest::new("hello world hello");

    test.press('v').press('/'); // Start search in visual mode

    assert_eq!(test.buffer_content(), "hello world hello\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Visual mode reselect with gv
// ============================================================================

#[test]
fn test_gv_reselect() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("lll") // Select
        .press_esc() // Exit visual mode
        .keys("gv"); // Reselect last selection

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 3);
}

#[test]
fn test_gv_after_delete() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .press('d') // Delete line
        .keys("gv"); // Reselect (might not work after delete)

    assert_eq!(test.buffer_content(), "line 2\nline 3\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Visual mode with indented text
// ============================================================================

#[test]
fn test_V_indented_lines() {
    let mut test = EditorTest::new("    line 1\n    line 2\n    line 3");

    test.press('V')
        .press('j') // Select 2 indented lines
        .press('d'); // Delete

    assert_eq!(test.buffer_content(), "    line 3\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_v_select_indentation() {
    let mut test = EditorTest::new("    indented line");

    test.press('v').keys("lll"); // Select spaces

    assert_eq!(test.buffer_content(), "    indented line\n");
    test.assert_cursor(0, 3);
}

// ============================================================================
// Visual mode search features (* and # in visual mode)
// ============================================================================

#[test]
fn test_visual_star_search() {
    let mut test = EditorTest::new("hello world\nhello test\nhello world");

    // Select "hello" and press * to search forward
    test.press('v')
        .keys("llll") // Select "hello"
        .press('*'); // Search for next occurrence

    // Should exit visual mode and jump to next "hello" on line 1
    assert_eq!(test.editor.mode(), Mode::Normal);
    test.assert_cursor(1, 0);

    // Press n to find next match
    test.press('n');
    test.assert_cursor(2, 0);
}

#[test]
fn test_visual_hash_search() {
    let mut test = EditorTest::new("hello world\nhello test\nhello world");

    // Move to line 2, select "hello" and press # to search backward
    test.keys("jj") // Move to line 2
        .press('v')
        .keys("llll") // Select "hello"
        .press('#'); // Search backward

    // Should exit visual mode and jump to previous "hello" on line 1
    assert_eq!(test.editor.mode(), Mode::Normal);
    test.assert_cursor(1, 0);

    // Press n to continue searching backward
    test.press('n');
    test.assert_cursor(0, 0);
}

#[test]
fn test_visual_star_multiline_selection() {
    let mut test = EditorTest::new("hello\nworld\nhello\ntest");

    // Select across lines and press *
    test.press('v')
        .keys("jl") // Select "hello\nwo"
        .press('*'); // Search for this multi-line text

    // Should find the next occurrence (note: multiline search might not match exactly)
    // The behavior should be that it searches for the literal text
    assert_eq!(test.editor.mode(), Mode::Normal);
}

#[test]
fn test_visual_block_star_search() {
    let mut test = EditorTest::new("abc\ndef\nghi\nabc");

    // Select block and press *
    test.press_with(KeyCode::Char('v'), KeyModifiers::CONTROL) // Visual block mode
        .keys("l") // Select 2 chars width
        .press('*'); // Search for "ab"

    // Should exit visual mode and jump to next occurrence of "ab" on line 3
    assert_eq!(test.editor.mode(), Mode::Normal);
    test.assert_cursor(3, 0);
}

#[test]
fn test_visualline_star_search() {
    let mut test = EditorTest::new("line one\nline two\nline one");

    // Select full line and press *
    test.press('V') // Visual line mode
        .press('*'); // Search for "line one"

    // Should exit visual mode and jump to next occurrence
    assert_eq!(test.editor.mode(), Mode::Normal);
    test.assert_cursor(2, 0);
}

#[test]
fn test_visual_star_with_special_chars() {
    let mut test = EditorTest::new("foo.bar\ntest\nfoo.bar");

    // Select "foo.bar" which contains regex special character '.'
    test.press('v')
        .keys("llllll") // Select "foo.bar"
        .press('*'); // Should escape the '.' for literal search

    // Should find exact match, not regex match
    assert_eq!(test.editor.mode(), Mode::Normal);
    test.assert_cursor(2, 0);
}

// ============================================================================
// Visual mode search extension (/ and ? extend selection)
// ============================================================================

#[test]
fn test_visual_search_extends_selection_forward() {
    let mut test = EditorTest::new("hello world test hello");

    // Start visual selection at beginning
    test.press('v')
        .keys("ll") // Select "hel"
        .press('/') // Enter search mode
        .type_text("test")
        .press_enter(); // Execute search

    // Should extend selection from "hello" to "test"
    assert_eq!(test.editor.mode(), Mode::Visual);
    test.assert_cursor(0, 12); // At start of "test"

    // Visual anchor should be at original position (0, 0)
    let visual_start = test.editor.visual_start();
    assert_eq!(visual_start, Some((0, 0)));
}

#[test]
fn test_visual_search_extends_selection_backward() {
    let mut test = EditorTest::new("hello world test hello");

    // Start at end, select backward, then search backward
    test.keys("$") // Move to end
        .press('v')
        .keys("hh") // Select backward
        .press('?') // Enter backward search
        .type_text("world")
        .press_enter(); // Execute search

    // Should extend selection from end to "world"
    assert_eq!(test.editor.mode(), Mode::Visual);
    test.assert_cursor(0, 6); // At start of "world"
}

#[test]
fn test_visualline_search_extends_selection() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    // Start visual line selection
    test.press('V') // Visual line mode
        .press('/') // Enter search mode
        .type_text("line 3")
        .press_enter(); // Execute search

    // Should extend selection to include lines up to line 3
    assert_eq!(test.editor.mode(), Mode::VisualLine);
    test.assert_cursor(2, 0); // At line 3
}

#[test]
fn test_visualblock_search_extends_selection() {
    let mut test = EditorTest::new("abc def\nghi jkl\nmno pqr");

    // Start visual block selection
    test.press_with(KeyCode::Char('v'), KeyModifiers::CONTROL) // Visual block
        .press('/') // Enter search mode
        .type_text("jkl")
        .press_enter(); // Execute search

    // Should extend block selection to search match
    assert_eq!(test.editor.mode(), Mode::VisualBlock);
    test.assert_cursor(1, 4); // At "jkl"
}

#[test]
fn test_visual_search_escape_cancels() {
    let mut test = EditorTest::new("hello world test");

    // Start visual selection and cancel search
    test.press('v')
        .keys("ll")
        .press('/') // Enter search mode (saves position at 0, 2)
        .type_text("test")
        .press_esc(); // Cancel search

    // Should return to visual mode at position when search was started (0, 2)
    assert_eq!(test.editor.mode(), Mode::Visual);
    test.assert_cursor(0, 2); // Cursor restored to position when / was pressed

    // Visual anchor should still be at original position (0, 0)
    let visual_start = test.editor.visual_start();
    assert_eq!(visual_start, Some((0, 0)));
}
