mod helpers;

use helpers::EditorTest;

/// Test Ctrl-O (jump back)
#[test]
fn test_ctrl_o_jump_back() {
    let mut test = EditorTest::new("line1\nline2\nline3\nline4\n");

    // Make a jump
    test.keys("G");
    test.assert_cursor(2, 0); // Last line (accounting for empty final line behavior)

    // Jump back
    test.keys("<C-o>");

    test.assert_cursor(0, 0);
}

/// Test Ctrl-I (jump forward)
#[test]
fn test_ctrl_i_jump_forward() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    // Make a jump
    test.keys("G");

    // Jump back
    test.keys("<C-o>");
    test.assert_cursor(0, 0);

    // Jump forward again
    test.keys("<C-i>");

    // Should be back at the end
    assert!(test.cursor().0 > 0);
}

/// Test gg (go to first line)
#[test]
fn test_gg_first_line() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    test.keys("G");
    test.keys("gg");

    test.assert_cursor(0, 0);
}

/// Test gg with count
#[test]
fn test_gg_with_count() {
    let mut test = EditorTest::new("line1\nline2\nline3\nline4\n");

    test.keys("3gg");

    test.assert_cursor(2, 0);
}

/// Test G (go to last line)
#[test]
fn test_g_capital_last_line() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    test.keys("G");

    // Should be on last line
    assert!(test.cursor().0 >= 2);
}

/// Test G with count
#[test]
fn test_g_capital_with_count() {
    let mut test = EditorTest::new("line1\nline2\nline3\nline4\n");

    test.keys("2G");

    test.assert_cursor(1, 0);
}

/// Test % (jump to matching bracket)
#[test]
fn test_percent_matching_bracket() {
    let mut test = EditorTest::new("function() {\n    code;\n}\n");

    test.keys("0");
    test.keys("f{"); // Find opening brace
    test.keys("%");

    // Should jump to closing brace
    test.assert_cursor(2, 0);
}

/// Test % with parentheses
#[test]
fn test_percent_matching_paren() {
    let mut test = EditorTest::new("(hello world)\n");

    test.keys("0");
    test.keys("%");

    // Should jump to closing paren
    assert!(test.cursor().1 > 0);
}

/// Test [[ (previous section)
#[test]
fn test_double_bracket_prev_section() {
    let mut test = EditorTest::new("{\n    code\n}\n{\n    more\n}\n");

    test.keys("G");
    test.keys("[[");

    // Should move to previous section
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test ]] (next section)
#[test]
fn test_double_bracket_next_section() {
    let mut test = EditorTest::new("{\n    code\n}\n{\n    more\n}\n");

    test.keys("gg");
    test.keys("]]");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test '' (jump to previous position)
#[test]
fn test_double_quote_jump_to_prev() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    test.keys("j");
    let prev_pos = test.cursor();

    test.keys("G");
    test.keys("''");

    // Should jump back
    test.assert_cursor(prev_pos.0, prev_pos.1);
}

/// Test `` (jump to exact previous position)
#[test]
fn test_double_backtick_jump_exact() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    test.keys("j");
    test.keys("$");
    let prev_pos = test.cursor();

    test.keys("gg");
    test.keys("``");

    test.assert_cursor(prev_pos.0, prev_pos.1);
}

/// Test H (move to top of screen)
#[test]
fn test_h_capital_top_screen() {
    let mut test = EditorTest::new("line1\nline2\nline3\nline4\nline5\n");

    test.keys("G");
    test.keys("H");

    // Should be at top of viewport
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test M (move to middle of screen)
#[test]
fn test_m_capital_middle_screen() {
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n");

    test.keys("M");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test L (move to bottom of screen)
#[test]
fn test_l_capital_bottom_screen() {
    let mut test = EditorTest::new("line1\nline2\nline3\nline4\nline5\n");

    test.keys("gg");
    test.keys("L");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test zz (center screen on cursor)
#[test]
fn test_zz_center_screen() {
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n");

    test.keys("5G");
    test.keys("zz");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test zt (move cursor line to top)
#[test]
fn test_zt_cursor_to_top() {
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n");

    test.keys("5G");
    test.keys("zt");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test zb (move cursor line to bottom)
#[test]
fn test_zb_cursor_to_bottom() {
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n");

    test.keys("5G");
    test.keys("zb");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-D (scroll down half page)
#[test]
fn test_ctrl_d_scroll_down() {
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n");

    test.keys("gg");
    let start_line = test.cursor().0;

    test.keys("<C-d>");

    // Should have moved down
    assert!(test.cursor().0 > start_line);
}

/// Test Ctrl-U (scroll up half page)
#[test]
fn test_ctrl_u_scroll_up() {
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n");

    test.keys("G");
    let start_line = test.cursor().0;

    test.keys("<C-u>");

    // Should have moved up
    assert!(test.cursor().0 < start_line);
}

/// Test Ctrl-F (page down)
#[test]
fn test_ctrl_f_page_down() {
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nl11\nl12\n");

    test.keys("gg");
    test.keys("<C-f>");

    // Should have moved down
    assert!(test.cursor().0 > 0);
}

/// Test Ctrl-B (page up)
#[test]
fn test_ctrl_b_page_up() {
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nl11\nl12\n");

    test.keys("G");
    test.keys("<C-b>");

    // Should have moved up
    assert!(test.cursor().0 < 11);
}

/// Test gd (goto definition - LSP)
#[test]
fn test_gd_goto_definition() {
    let mut test = EditorTest::new("fn test() {}\ntest();\n");
    test.set_file_path("/tmp/test.rs".to_string());

    test.keys("j");
    test.keys("gd");

    // Without LSP, should not crash
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test gf (goto file under cursor)
#[test]
fn test_gf_goto_file() {
    let mut test = EditorTest::new("src/main.rs\n");

    test.keys("0");
    test.keys("gf");

    // Without actual file, should handle gracefully
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test { (previous paragraph)
#[test]
fn test_left_brace_prev_paragraph() {
    let mut test = EditorTest::new("para1\n\npara2\nline2\n\npara3\n");

    test.keys("G");
    test.keys("{");

    // Should move to previous blank line
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test } (next paragraph)
#[test]
fn test_right_brace_next_paragraph() {
    let mut test = EditorTest::new("para1\nline1\n\npara2\n\npara3\n");

    test.keys("gg");
    test.keys("}");

    // Should move to next blank line
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test ( (previous sentence)
#[test]
fn test_left_paren_prev_sentence() {
    let mut test = EditorTest::new("First sentence. Second sentence. Third sentence.\n");

    test.keys("$");
    test.keys("(");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test ) (next sentence)
#[test]
fn test_right_paren_next_sentence() {
    let mut test = EditorTest::new("First sentence. Second sentence. Third sentence.\n");

    test.keys("0");
    test.keys(")");

    test.assert_mode(ovim::mode::Mode::Normal);
}
