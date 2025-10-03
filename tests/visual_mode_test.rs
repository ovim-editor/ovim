mod helpers;
use helpers::EditorTest;
use insta::assert_snapshot;

// ============================================================================
// 'v' command - Character-wise visual mode
// ============================================================================

#[test]
fn test_v_basic_selection() {
    let mut test = EditorTest::new("hello world");

    test.press('v')       // Enter visual mode
        .keys("lll");     // Select 4 chars (h, e, l, l)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_v_delete_selection() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("lll")      // Select "hell"
        .press('d');      // Delete selection

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_v_yank_selection() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("llll")     // Select "hello"
        .press('y')       // Yank
        .press_esc()
        .keys("$")        // Move to end
        .press('p');      // Paste

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_v_select_word() {
    let mut test = EditorTest::new("hello world test");

    test.press('v')
        .keys("e");       // Select to end of word

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_v_across_lines() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('v')
        .keys("jjj");     // Select across multiple lines

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_v_backward_selection() {
    let mut test = EditorTest::new("hello world");

    test.keys("$")        // Go to end
        .press('v')
        .keys("hhh");     // Select backward

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_v_change_selection() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("llll")     // Select "hello"
        .press('c')       // Change
        .type_text("goodbye")
        .press_esc();

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_v_escape_cancels() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("lll")      // Make selection
        .press_esc();     // Cancel

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// 'V' command - Line-wise visual mode
// ============================================================================

#[test]
fn test_V_basic_selection() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V');      // Enter visual line mode

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_V_multiple_lines() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('V')
        .keys("jj");      // Select 3 lines

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_V_delete_lines() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('V')
        .keys("jj")       // Select 3 lines
        .press('d');      // Delete

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_V_yank_paste_lines() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .press('j')       // Select 2 lines
        .press('y')       // Yank
        .press_esc()
        .press('G')       // Go to last line
        .press('p');      // Paste

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_V_from_middle_of_line() {
    let mut test = EditorTest::new("hello world\ntest line");

    test.keys("w")        // Move to "world"
        .press('V');      // Should select entire line

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_V_select_all() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .keys("G");       // Select to last line

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_V_indent() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .press('j')       // Select 2 lines
        .press('>');      // Indent (if implemented)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_V_backward_selection() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("G")        // Go to last line
        .press('V')
        .keys("kk");      // Select upward

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Visual mode with operators
// ============================================================================

#[test]
fn test_v_delete_word() {
    let mut test = EditorTest::new("hello world test");

    test.press('v')
        .keys("e")        // Select word
        .press('d');      // Delete

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_v_change_word() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("e")        // Select "hello"
        .press('c')       // Change
        .type_text("goodbye")
        .press_esc();

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_V_delete_and_undo() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .press('j')       // Select 2 lines
        .press('d')       // Delete
        .press('u');      // Undo

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_v_yank_and_replace() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("llll")     // Select "hello"
        .press('y')       // Yank
        .press_esc()
        .keys("w")        // Move to "world"
        .press('v')
        .keys("llll")     // Select "world"
        .press('p');      // Paste (should replace selection)

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Visual mode edge cases
// ============================================================================

#[test]
fn test_v_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.press('j')       // Move to empty line
        .press('v')
        .press('j');      // Select to next line

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_v_single_char() {
    let mut test = EditorTest::new("x");

    test.press('v');      // Select single char

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_v_end_of_line() {
    let mut test = EditorTest::new("hello");

    test.keys("$")        // Move to end
        .press('v');      // Select last char

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_V_single_line() {
    let mut test = EditorTest::new("only line");

    test.press('V');      // Select only line

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_v_select_entire_file() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("gg")       // Go to top
        .press('v')
        .keys("G");       // Select to end

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Visual mode with motions
// ============================================================================

#[test]
fn test_v_with_w_motion() {
    let mut test = EditorTest::new("hello world test");

    test.press('v')
        .press('w');      // Select word forward

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_v_with_dollar() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("$");       // Select to end of line

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_v_with_zero() {
    let mut test = EditorTest::new("hello world");

    test.keys("$")        // Go to end
        .press('v')
        .keys("0");       // Select to beginning

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_V_with_gg() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("G")        // Go to last line
        .press('V')
        .keys("gg");      // Select to first line

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Switch between visual modes
// ============================================================================

#[test]
fn test_v_to_V_switch() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('v')       // Character visual
        .press('V');      // Switch to line visual

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_V_to_v_switch() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('V')       // Line visual
        .press('v');      // Switch to character visual

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Visual mode with count
// ============================================================================

#[test]
fn test_v_with_count() {
    let mut test = EditorTest::new("hello world test");

    test.press('v')
        .keys("3l");      // Select 4 chars (including current)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_V_with_count() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('V')
        .keys("3j");      // Select 4 lines

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Visual mode with search
// ============================================================================

#[test]
fn test_v_to_search_result() {
    let mut test = EditorTest::new("hello world hello");

    test.press('v')
        .press('/');      // Start search in visual mode

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Visual mode reselect with gv
// ============================================================================

#[test]
fn test_gv_reselect() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("lll")      // Select
        .press_esc()      // Exit visual mode
        .keys("gv");      // Reselect last selection

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_gv_after_delete() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .press('d')       // Delete line
        .keys("gv");      // Reselect (might not work after delete)

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Visual mode with indented text
// ============================================================================

#[test]
fn test_V_indented_lines() {
    let mut test = EditorTest::new("    line 1\n    line 2\n    line 3");

    test.press('V')
        .press('j')       // Select 2 indented lines
        .press('d');      // Delete

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_v_select_indentation() {
    let mut test = EditorTest::new("    indented line");

    test.press('v')
        .keys("lll");     // Select spaces

    assert_snapshot!(test.snapshot_state());
}
