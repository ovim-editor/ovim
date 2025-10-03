mod helpers;
use helpers::EditorTest;
use insta::assert_snapshot;

// ============================================================================
// '>' command - Indent operator
// ============================================================================

#[test]
fn test_shift_right_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys(">>");      // Indent current line

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_shift_right_with_count() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("3>>");     // Indent 3 lines

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_shift_right_already_indented() {
    let mut test = EditorTest::new("    already indented\nplain line");

    test.keys(">>");      // Indent more

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// '>' with motions
// ============================================================================

#[test]
fn test_shift_right_j() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys(">j");      // Indent current and next line

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_shift_right_4j() {
    let mut test = EditorTest::new("a\nb\nc\nd\ne\nf");

    test.keys(">4j");     // Indent 5 lines (current + 4 down)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_shift_right_k() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("jj")       // Move to line 2
        .keys(">k");      // Indent line 2 and line 1

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_shift_right_2k() {
    let mut test = EditorTest::new("a\nb\nc\nd\ne");

    test.keys("G")        // Go to last line
        .keys(">2k");     // Indent 3 lines (current + 2 up)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_shift_right_G() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4\nline 5");

    test.keys("jj")       // Line 2
        .keys(">G");      // Indent from line 2 to end

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_shift_right_gg() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("G")        // Last line
        .keys(">gg");     // Indent from last to first

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// '<' command - Dedent operator
// ============================================================================

#[test]
fn test_shift_left_line() {
    let mut test = EditorTest::new("    indented line\n    another");

    test.keys("<<");      // Dedent current line

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_shift_left_with_count() {
    let mut test = EditorTest::new("    line 1\n    line 2\n    line 3");

    test.keys("3<<");     // Dedent 3 lines

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_shift_left_no_indent() {
    let mut test = EditorTest::new("no indent");

    test.keys("<<");      // Should do nothing

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_shift_left_partial_indent() {
    let mut test = EditorTest::new("  two spaces");

    test.keys("<<");      // Remove indent (might go to 0 or stay at some minimum)

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// '<' with motions
// ============================================================================

#[test]
fn test_shift_left_j() {
    let mut test = EditorTest::new("    line 1\n    line 2\n    line 3");

    test.keys("<j");      // Dedent current and next

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_shift_left_3j() {
    let mut test = EditorTest::new("    a\n    b\n    c\n    d\n    e");

    test.keys("<3j");     // Dedent 4 lines

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_shift_left_k() {
    let mut test = EditorTest::new("    line 1\n    line 2\n    line 3");

    test.keys("j")
        .keys("<k");      // Dedent current and previous

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_shift_left_G() {
    let mut test = EditorTest::new("    a\n    b\n    c\n    d");

    test.keys("j")
        .keys("<G");      // Dedent from line 1 to end

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Visual mode indenting
// ============================================================================

#[test]
fn test_visual_line_indent() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')       // Visual line
        .keys("jj")       // Select 3 lines
        .press('>');      // Indent

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_visual_line_dedent() {
    let mut test = EditorTest::new("    line 1\n    line 2\n    line 3");

    test.press('V')
        .keys("j")
        .press('<');      // Dedent

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_visual_char_indent() {
    let mut test = EditorTest::new("hello\nworld\ntest");

    test.press('v')       // Character visual
        .keys("jj")
        .press('>');      // Should indent affected lines

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_visual_reselect_indent() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .keys("j")
        .press('>')       // Indent once
        .keys("gv")       // Reselect
        .press('>');      // Indent again

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Multiple indents
// ============================================================================

#[test]
fn test_multiple_indent() {
    let mut test = EditorTest::new("line 1");

    test.keys(">>")       // Indent
        .keys(">>")       // Indent again
        .keys(">>");      // And again

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_indent_then_dedent() {
    let mut test = EditorTest::new("line 1");

    test.keys(">>")       // Indent
        .keys("<<");      // Dedent

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_indent_dedent_cycle() {
    let mut test = EditorTest::new("    indented");

    test.keys("<<")       // Dedent
        .keys(">>")       // Indent
        .keys(">>")       // Indent more
        .keys("<<");      // Dedent

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Indent with text objects (if supported)
// ============================================================================

#[test]
fn test_indent_paragraph() {
    let mut test = EditorTest::new("para line 1\npara line 2\n\nnext para");

    test.keys(">ip");     // Indent inner paragraph

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_indent_around_paragraph() {
    let mut test = EditorTest::new("para 1\npara 1 cont\n\npara 2");

    test.keys(">ap");     // Indent around paragraph

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Indent and undo/redo
// ============================================================================

#[test]
fn test_indent_and_undo() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys(">>")       // Indent
        .press('u');      // Undo

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_multiple_indent_undo() {
    let mut test = EditorTest::new("line 1");

    test.keys(">>")
        .keys(">>")
        .keys(">>")
        .press('u')       // Undo last indent
        .press('u')       // Undo another
        .press('u');      // Undo first

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_indent_undo_redo() {
    let mut test = EditorTest::new("line 1");

    test.keys(">>")
        .press('u')       // Undo
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        );                // Redo

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Indent with different content
// ============================================================================

#[test]
fn test_indent_mixed_content() {
    let mut test = EditorTest::new("    indented\nno indent\n        more indent");

    test.keys(">G");      // Indent all

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_indent_empty_line() {
    let mut test = EditorTest::new("line 1\n\nline 3");

    test.keys("j")        // Move to empty line
        .keys(">>");      // Indent empty line

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_indent_whitespace_only() {
    let mut test = EditorTest::new("    ");

    test.keys(">>");      // Indent whitespace

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_indent_single_line_file() {
    let mut test = EditorTest::new("only line");

    test.keys(">>");

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_indent_at_eof() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.keys("G")        // Last line
        .keys(">>")
        .keys(">j");      // Try to indent beyond EOF

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dedent_beyond_zero() {
    let mut test = EditorTest::new("no indent");

    test.keys("<<")       // Dedent
        .keys("<<")       // Dedent again (should stay at 0)
        .keys("<<");      // And again

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Indent with counts and motions combined
// ============================================================================

#[test]
fn test_2_shift_right_3j() {
    let mut test = EditorTest::new("a\nb\nc\nd\ne\nf");

    test.keys("2>3j");    // Count 2, indent, motion 3j

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_3_shift_right_2k() {
    let mut test = EditorTest::new("a\nb\nc\nd\ne");

    test.keys("G")
        .keys("3>2k");    // Count 3, indent, motion 2k

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Auto-indent behavior
// ============================================================================

#[test]
fn test_equal_equal_auto_indent() {
    let mut test = EditorTest::new("if true {\ncode\n}");

    test.keys("j")        // Move to "code"
        .keys("==");      // Auto-indent line

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_equal_motion() {
    let mut test = EditorTest::new("{\nline1\nline2\n}");

    test.keys("j")
        .keys("=j");      // Auto-indent current and next

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_equal_G() {
    let mut test = EditorTest::new("{\nline1\nline2\nline3\n}");

    test.keys("j")
        .keys("=G");      // Auto-indent to end

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Indent with tabs vs spaces
// ============================================================================

#[test]
fn test_indent_creates_spaces() {
    let mut test = EditorTest::new("line");

    test.keys(">>");      // Should create spaces (or tabs based on settings)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_indent_with_existing_tabs() {
    let mut test = EditorTest::new("\tline");

    test.keys(">>");      // Indent tabbed line

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dedent_tabs() {
    let mut test = EditorTest::new("\t\tline");

    test.keys("<<");      // Dedent tabs

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Dot repeat with indent
// ============================================================================

#[test]
fn test_indent_dot_repeat() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys(">>")       // Indent
        .keys("j")        // Move down
        .press('.')       // Repeat
        .keys("j")
        .press('.');      // Repeat again

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dedent_dot_repeat() {
    let mut test = EditorTest::new("    line 1\n    line 2\n    line 3");

    test.keys("<<")
        .keys("j")
        .press('.')
        .keys("j")
        .press('.');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Indent in insert mode (if Ctrl-T/Ctrl-D supported)
// ============================================================================

#[test]
fn test_ctrl_t_indent_in_insert() {
    let mut test = EditorTest::new("line");

    test.press('i')
        .press_with(
            crossterm::event::KeyCode::Char('t'),
            crossterm::event::KeyModifiers::CONTROL
        )                 // Indent in insert mode
        .press_esc();

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_ctrl_d_dedent_in_insert() {
    let mut test = EditorTest::new("    line");

    test.press('i')
        .press_with(
            crossterm::event::KeyCode::Char('d'),
            crossterm::event::KeyModifiers::CONTROL
        )                 // Dedent in insert mode
        .press_esc();

    assert_snapshot!(test.snapshot_state());
}
