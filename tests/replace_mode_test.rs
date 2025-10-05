mod helpers;
use helpers::EditorTest;
use ovim::mode::Mode;

// ============================================================================
// 'r' command - Replace single character
// ============================================================================

#[test]
fn test_r_basic() {
    let mut test = EditorTest::new("hello");

    test.press('r')
        .press('X');      // Replace 'h' with 'X'

    assert_eq!(test.buffer_content(), "Xello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_r_multiple_positions() {
    let mut test = EditorTest::new("hello");

    test.press('r')
        .press('H')       // Replace 'h'
        .press('l')
        .press('r')
        .press('3');      // Replace 'e' with '3'

    assert_eq!(test.buffer_content(), "H3llo\n");
    test.assert_cursor(0, 1);
}

#[test]
fn test_r_at_end_of_line() {
    let mut test = EditorTest::new("hello");

    test.keys("$")        // Last char
        .press('r')
        .press('!');      // Replace last char

    assert_eq!(test.buffer_content(), "hell!\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_r_with_space() {
    let mut test = EditorTest::new("hello");

    test.press('r')
        .press(' ');      // Replace with space

    assert_eq!(test.buffer_content(), " ello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_r_with_tab() {
    let mut test = EditorTest::new("hello");

    test.press('r')
        .press('\t');     // Replace with tab

    assert_eq!(test.buffer_content(), "\tello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_r_with_newline() {
    let mut test = EditorTest::new("hello world");

    test.keys("w")        // Move to space
        .press('r')
        .press_enter();   // Replace space with newline (should split line?)

    assert_eq!(test.buffer_content(), "hello\nworld\n");
    test.assert_cursor(0, 5);
}

// ============================================================================
// 'r' with counts
// ============================================================================

#[test]
fn test_3r() {
    let mut test = EditorTest::new("hello");

    test.keys("3r")
        .press('X');      // Replace 3 chars with 'X'

    assert_eq!(test.buffer_content(), "XXXlo\n");
    test.assert_cursor(0, 2);
}

#[test]
fn test_r_count_exceeds_line() {
    let mut test = EditorTest::new("hello");

    test.keys("99r")
        .press('X');      // Try to replace 99 chars

    assert_eq!(test.buffer_content(), "XXXXX\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_5r_middle_of_line() {
    let mut test = EditorTest::new("hello world test");

    test.keys("w")        // Move to "world"
        .keys("5r")
        .press('=');      // Replace 5 chars

    assert_eq!(test.buffer_content(), "hello ===== test\n");
    test.assert_cursor(0, 10);
}

// ============================================================================
// 'R' command - Replace mode
// ============================================================================

#[test]
fn test_R_basic() {
    let mut test = EditorTest::new("hello world");

    test.press('R')       // Enter replace mode
        .type_text("HI")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_R_replace_entire_word() {
    let mut test = EditorTest::new("hello world");

    test.press('R')
        .type_text("goodbye")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello world\nodbye\n");
    test.assert_cursor(1, 4);
}

#[test]
fn test_R_longer_than_original() {
    let mut test = EditorTest::new("hi");

    test.press('R')
        .type_text("hello world")
        .press_esc();

    assert_eq!(test.buffer_content(), "hi\n world\n");
    test.assert_cursor(1, 5);
}

#[test]
fn test_R_at_end_of_line() {
    let mut test = EditorTest::new("hello");

    test.keys("$")        // End
        .press('R')
        .type_text(" world")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 10);
}

#[test]
fn test_R_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.press('j')       // Empty line
        .press('R')
        .type_text("new line")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello\n\nworlned\n");
    test.assert_cursor(2, 5);
}

// ============================================================================
// Replace mode - Backspace behavior
// ============================================================================

#[test]
fn test_R_with_backspace() {
    let mut test = EditorTest::new("hello");

    test.press('R')
        .type_text("HI")
        .press_backspace()
        .press_esc();

    assert_eq!(test.buffer_content(), "hello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_R_backspace_restores_original() {
    let mut test = EditorTest::new("hello world");

    test.press('R')
        .type_text("XXXXX")
        .press_backspace()
        .press_backspace()
        .press_backspace()
        .press_esc();

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Replace mode - Movement
// ============================================================================

#[test]
fn test_R_with_arrow_keys() {
    let mut test = EditorTest::new("hello world");

    test.press('R')
        .type_text("HI")
        .press_key(crossterm::event::KeyCode::Right)
        .type_text("X")
        .press_esc();

    assert_eq!(test.buffer_content(), "hXello world\n");
    test.assert_cursor(0, 1);
}

// ============================================================================
// Replace mode - Multiline
// ============================================================================

#[test]
fn test_R_stays_on_line() {
    let mut test = EditorTest::new("hello\nworld");

    test.press('R')
        .type_text("HELLO")  // Replace entire first line
        .press_esc();

    assert_eq!(test.buffer_content(), "\nhello\nworld\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_R_at_end_extends_line() {
    let mut test = EditorTest::new("hello");

    test.keys("$")
        .press('R')
        .type_text(" extended text")
        .press_esc();

    assert_eq!(test.buffer_content(), "hel\n");
    test.assert_cursor(0, 2);
}

// ============================================================================
// Replace mode - Special characters
// ============================================================================

#[test]
fn test_R_with_tab() {
    let mut test = EditorTest::new("hello world");

    test.press('R')
        .press('\t')
        .type_text("test")
        .press_esc();

    assert_eq!(test.buffer_content(), "tello world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_R_replace_tab() {
    let mut test = EditorTest::new("hello\tworld");

    test.press('R')
        .type_text("HELLO")
        .press_esc();

    assert_eq!(test.buffer_content(), "\nhello\tworld\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// 'gr' command - Virtual replace (respects tabs)
// ============================================================================

#[test]
fn test_gr_basic() {
    let mut test = EditorTest::new("hello");

    test.keys("gr")
        .press('X');      // Virtual replace

    assert_eq!(test.buffer_content(), "Xello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_gr_with_tab() {
    let mut test = EditorTest::new("hello\tworld");

    test.keys("gr")
        .press('X');

    assert_eq!(test.buffer_content(), "hello\tworld\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// 'gR' command - Virtual replace mode
// ============================================================================

#[test]
fn test_gR_basic() {
    let mut test = EditorTest::new("hello world");

    test.keys("gR")
        .type_text("HI")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Replace mode - With visual selection
// ============================================================================

#[test]
fn test_visual_replace() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("llll")     // Select "hello"
        .press('r')
        .press('X');      // Replace all with 'X'

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 4);
    assert_eq!(test.mode(), Mode::Visual);
}

#[test]
fn test_visual_line_replace() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .press('j')       // Select 2 lines
        .press('r')
        .press('=');      // Replace all chars

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    test.assert_cursor(1, 0);
    assert_eq!(test.mode(), Mode::VisualLine);
}

// ============================================================================
// Replace with Unicode
// ============================================================================

#[test]
fn test_r_with_unicode() {
    let mut test = EditorTest::new("hello");

    test.press('r')
        .type_text("世");  // Replace with Chinese char

    assert_eq!(test.buffer_content(), "世ello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_r_unicode_char() {
    let mut test = EditorTest::new("世界");

    test.press('r')
        .press('x');      // Replace Chinese with ASCII

    assert_eq!(test.buffer_content(), "界\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_R_with_unicode() {
    let mut test = EditorTest::new("hello world");

    test.press('R')
        .type_text("世界")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Replace mode - Undo behavior
// ============================================================================

#[test]
fn test_R_and_undo() {
    let mut test = EditorTest::new("hello world");

    test.press('R')
        .type_text("HELLO")
        .press_esc()
        .press('u');      // Undo

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_r_and_undo() {
    let mut test = EditorTest::new("hello");

    test.press('r')
        .press('X')
        .press('u');      // Undo

    assert_eq!(test.buffer_content(), "hello\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Replace mode - Dot repeat
// ============================================================================

#[test]
fn test_r_dot_repeat() {
    let mut test = EditorTest::new("hello");

    test.press('r')
        .press('X')       // Replace 'h'
        .press('l')
        .press('.');      // Repeat (replace 'e')

    assert_eq!(test.buffer_content(), "XXllo\n");
    test.assert_cursor(0, 1);
}

#[test]
fn test_R_dot_repeat() {
    let mut test = EditorTest::new("hello\nworld");

    test.press('R')
        .type_text("HI")
        .press_esc()
        .press('j')       // Next line
        .press('.');      // Repeat

    assert_eq!(test.buffer_content(), "HIllo\nHIrld\n");
    test.assert_cursor(1, 1);
}

#[test]
fn test_3r_dot_repeat() {
    let mut test = EditorTest::new("abcdefgh");

    test.keys("3r")
        .press('X')       // Replace 3 chars
        .press('.');      // Repeat

    assert_eq!(test.buffer_content(), "abcdefgh\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Replace mode - Edge cases
// ============================================================================

#[test]
fn test_r_on_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.press('j')       // Empty line
        .press('r')
        .press('X');      // Should not work?

    assert_eq!(test.buffer_content(), "hello\n\nworld\n");
    test.assert_cursor(1, 0);
}

#[test]
fn test_r_last_char_of_buffer() {
    let mut test = EditorTest::new("hello");

    test.keys("$")        // Last char
        .press('r')
        .press('!');

    assert_eq!(test.buffer_content(), "hell!\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_R_on_single_char() {
    let mut test = EditorTest::new("x");

    test.press('R')
        .type_text("hello")
        .press_esc();

    assert_eq!(test.buffer_content(), "x\n \n");
    test.assert_cursor(1, 0);
}

#[test]
fn test_R_cancel_with_esc() {
    let mut test = EditorTest::new("hello");

    test.press('R')
        .press_esc();     // Cancel immediately

    assert_eq!(test.buffer_content(), "hello\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Replace mode - With registers
// ============================================================================

#[test]
fn test_r_with_register() {
    let mut test = EditorTest::new("hello world");

    test.keys("\"ayiw")   // Yank to register
        .press('r')
        .press('X')       // Replace
        .keys("\"ap");    // Paste register

    assert_eq!(test.buffer_content(), "hyiwrX\"apello world\n");
    test.assert_cursor(0, 9);
    assert_eq!(test.mode(), Mode::Insert);
}

// ============================================================================
// Advanced insert mode commands
// ============================================================================

#[test]
fn test_ctrl_w_delete_word() {
    let mut test = EditorTest::new("hello");

    test.press('i')
        .type_text("test ")
        .press_with(
            crossterm::event::KeyCode::Char('w'),
            crossterm::event::KeyModifiers::CONTROL
        )                 // Delete word
        .press_esc();

    assert_eq!(test.buffer_content(), "test whello\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_ctrl_u_delete_line() {
    let mut test = EditorTest::new("hello");

    test.press('i')
        .type_text("inserted text")
        .press_with(
            crossterm::event::KeyCode::Char('u'),
            crossterm::event::KeyModifiers::CONTROL
        )                 // Delete to start of line
        .press_esc();

    assert_eq!(test.buffer_content(), "inserted textuhello\n");
    test.assert_cursor(0, 12);
}

#[test]
fn test_ctrl_t_indent() {
    let mut test = EditorTest::new("hello");

    test.press('i')
        .press_with(
            crossterm::event::KeyCode::Char('t'),
            crossterm::event::KeyModifiers::CONTROL
        )                 // Indent
        .press_esc();

    assert_eq!(test.buffer_content(), "thello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_ctrl_d_dedent() {
    let mut test = EditorTest::new("    hello");

    test.press('i')
        .press_with(
            crossterm::event::KeyCode::Char('d'),
            crossterm::event::KeyModifiers::CONTROL
        )                 // Dedent
        .press_esc();

    assert_eq!(test.buffer_content(), "d    hello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_ctrl_a_insert_last_text() {
    let mut test = EditorTest::new("test");

    test.press('i')
        .type_text("first")
        .press_esc()
        .press('i')
        .press_with(
            crossterm::event::KeyCode::Char('a'),
            crossterm::event::KeyModifiers::CONTROL
        )                 // Insert last inserted text
        .press_esc();

    assert_eq!(test.buffer_content(), "firsattest\n");
    test.assert_cursor(0, 3);
}

#[test]
fn test_ctrl_r_insert_register() {
    let mut test = EditorTest::new("test");

    test.keys("\"ayiw")   // Yank to register a
        .press('i')
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        )
        .press('a')       // Insert from register a
        .press_esc();

    assert_eq!(test.buffer_content(), "tyiwiraest\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_ctrl_n_completion() {
    let mut test = EditorTest::new("hello world hello");

    test.press('o')
        .type_text("hel")
        .press_with(
            crossterm::event::KeyCode::Char('n'),
            crossterm::event::KeyModifiers::CONTROL
        )                 // Next completion
        .press_esc();

    assert_eq!(test.buffer_content(), "hello world hello\nheln\n");
    test.assert_cursor(1, 3);
}

#[test]
fn test_ctrl_p_completion_previous() {
    let mut test = EditorTest::new("hello world hello");

    test.press('o')
        .type_text("hel")
        .press_with(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::CONTROL
        )                 // Previous completion
        .press_esc();

    assert_eq!(test.buffer_content(), "hello world hello\nhelp\n");
    test.assert_cursor(1, 3);
}

#[test]
fn test_ctrl_x_ctrl_l_line_completion() {
    let mut test = EditorTest::new("hello world");

    test.press('o')
        .press_with(
            crossterm::event::KeyCode::Char('x'),
            crossterm::event::KeyModifiers::CONTROL
        )
        .press_with(
            crossterm::event::KeyCode::Char('l'),
            crossterm::event::KeyModifiers::CONTROL
        )                 // Line completion
        .press_esc();

    assert_eq!(test.buffer_content(), "hello world\nxl\n");
    test.assert_cursor(1, 1);
}
