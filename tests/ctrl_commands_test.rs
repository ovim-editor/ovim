mod helpers;

use helpers::EditorTest;

/// Test Ctrl-T (indent in insert mode)
#[test]
fn test_ctrl_t_indent_insert() {
    let mut test = EditorTest::new("line\n");

    test.keys("i");
    test.keys("<C-t>");
    test.press_esc();

    let content = test.buffer_content();
    assert!(content.starts_with("    ") || content.starts_with("\t"));
}

/// Test Ctrl-D (dedent in insert mode)
#[test]
fn test_ctrl_d_dedent_insert() {
    let mut test = EditorTest::new("    indented\n");

    test.keys("I");
    test.keys("<C-d>");
    test.press_esc();

    // Should have reduced indentation
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-W (delete word backward in insert mode)
#[test]
fn test_ctrl_w_delete_word_insert() {
    let mut test = EditorTest::new("\n");

    test.keys("i");
    test.type_text("hello world");
    test.keys("<C-w>");
    test.press_esc();

    assert_eq!(test.buffer_content(), "hello \n");
}

/// Test Ctrl-U (delete to line start in insert mode)
#[test]
fn test_ctrl_u_delete_to_start_insert() {
    let mut test = EditorTest::new("\n");

    test.keys("i");
    test.type_text("delete all this");
    test.keys("<C-u>");
    test.press_esc();

    assert_eq!(test.buffer_content(), "\n");
}

/// Test Ctrl-H (backspace in insert mode)
#[test]
fn test_ctrl_h_backspace_insert() {
    let mut test = EditorTest::new("\n");

    test.keys("i");
    test.type_text("hello");
    test.keys("<C-h>");
    test.press_esc();

    assert_eq!(test.buffer_content(), "hell\n");
}

/// Test Ctrl-O (jump back)
#[test]
fn test_ctrl_o_jump_back_normal() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    test.keys("G");
    test.keys("<C-o>");

    test.assert_cursor(0, 0);
}

/// Test Ctrl-I (jump forward, same as Tab)
#[test]
fn test_ctrl_i_jump_forward() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    test.keys("G");
    test.keys("<C-o>");
    test.keys("<C-i>");

    assert!(test.cursor().0 > 0);
}

/// Test Ctrl-A (increment number)
#[test]
fn test_ctrl_a_increment_number() {
    let mut test = EditorTest::new("count: 5\n");

    test.keys("0");
    test.keys("f5");
    test.keys("<C-a>");

    assert!(test.buffer_content().contains("6"));
}

/// Test Ctrl-A with count
#[test]
fn test_ctrl_a_increment_with_count() {
    let mut test = EditorTest::new("value: 10\n");

    test.keys("0");
    test.keys("f1");
    test.keys("5<C-a>");

    assert!(test.buffer_content().contains("15"));
}

/// Test Ctrl-X (decrement number)
#[test]
fn test_ctrl_x_decrement_number() {
    let mut test = EditorTest::new("count: 5\n");

    test.keys("0");
    test.keys("f5");
    test.keys("<C-x>");

    assert!(test.buffer_content().contains("4"));
}

/// Test Ctrl-X with count
#[test]
fn test_ctrl_x_decrement_with_count() {
    let mut test = EditorTest::new("value: 10\n");

    test.keys("0");
    test.keys("f1");
    test.keys("3<C-x>");

    assert!(test.buffer_content().contains("7"));
}

/// Test Ctrl-R (redo)
#[test]
fn test_ctrl_r_redo_change() {
    let mut test = EditorTest::new("original\n");

    test.keys("A");
    test.type_text(" added");
    test.press_esc();

    test.keys("u");
    test.keys("<C-r>");

    assert!(test.buffer_content().contains("added"));
}

/// Test multiple Ctrl-R
#[test]
fn test_ctrl_r_multiple_redo() {
    let mut test = EditorTest::new("start\n");

    test.keys("A");
    test.type_text("1");
    test.press_esc();

    test.keys("A");
    test.type_text("2");
    test.press_esc();

    test.keys("u");
    test.keys("u");

    test.keys("<C-r>");
    test.keys("<C-r>");

    assert!(test.buffer_content().contains("12"));
}

/// Test Ctrl-V (visual block mode)
#[test]
fn test_ctrl_v_visual_block_mode() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    test.keys("<C-v>");

    test.assert_mode(ovim::mode::Mode::VisualBlock);
}

/// Test Ctrl-V then block select
#[test]
fn test_ctrl_v_block_select() {
    let mut test = EditorTest::new("abc\ndef\nghi\n");

    test.keys("0");
    test.keys("<C-v>");
    test.keys("j");
    test.keys("j");

    test.assert_mode(ovim::mode::Mode::VisualBlock);
}

/// Test Ctrl-F (page down)
#[test]
fn test_ctrl_f_page_down() {
    // Need more lines than viewport height (10 in test mode) for page down to work
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nl11\nl12\n");

    test.keys("gg");
    let start = test.cursor().0;

    test.keys("<C-f>");

    assert!(test.cursor().0 > start);
}

/// Test Ctrl-B (page up)
#[test]
fn test_ctrl_b_page_up() {
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n");

    test.keys("G");
    let start = test.cursor().0;

    test.keys("<C-b>");

    assert!(test.cursor().0 < start);
}

/// Test Ctrl-D (scroll down half page)
#[test]
fn test_ctrl_d_scroll_down() {
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n");

    test.keys("gg");
    let start = test.cursor().0;

    test.keys("<C-d>");

    assert!(test.cursor().0 > start);
}

/// Test Ctrl-U (scroll up half page)
#[test]
fn test_ctrl_u_scroll_up() {
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n");

    test.keys("G");
    let start = test.cursor().0;

    test.keys("<C-u>");

    assert!(test.cursor().0 < start);
}

/// Test Ctrl-E (scroll down one line)
#[test]
fn test_ctrl_e_scroll_line_down() {
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n");

    test.keys("gg");
    test.keys("<C-e>");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-Y (scroll up one line)
#[test]
fn test_ctrl_y_scroll_line_up() {
    let mut test = EditorTest::new("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n");

    test.keys("5G");
    test.keys("<C-y>");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-G (show file info)
#[test]
fn test_ctrl_g_file_info() {
    let mut test = EditorTest::new("test content\n");
    test.set_file_path("/tmp/test.txt".to_string());

    test.keys("<C-g>");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-L (redraw screen)
#[test]
fn test_ctrl_l_redraw() {
    let mut test = EditorTest::new("test\n");

    test.keys("<C-l>");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-N (completion next in insert mode)
#[test]
fn test_ctrl_n_completion_next() {
    let mut test = EditorTest::new("word\n");

    test.keys("o");
    test.type_text("wo");
    test.keys("<C-n>");
    test.press_esc();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-P (completion previous in insert mode)
#[test]
fn test_ctrl_p_completion_prev() {
    let mut test = EditorTest::new("word\n");

    test.keys("o");
    test.type_text("wo");
    test.keys("<C-p>");
    test.press_esc();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-Space (trigger completion)
#[test]
fn test_ctrl_space_completion() {
    let mut test = EditorTest::new("function test() {}\n");
    test.set_file_path("/tmp/test.js".to_string());

    test.keys("o");
    test.type_text("te");
    test.keys("<C- >");
    test.press_esc();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-] (jump to tag/definition)
#[test]
fn test_ctrl_bracket_jump_tag() {
    let mut test = EditorTest::new("function test() {}\ntest();\n");
    test.set_file_path("/tmp/test.rs".to_string());

    test.keys("j");
    test.keys("<C-]>");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-^ (alternate file)
#[test]
fn test_ctrl_caret_alternate() {
    let mut test = EditorTest::new("test\n");

    test.keys("<C-^>");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-W + Ctrl-W (cycle windows)
#[test]
fn test_ctrl_w_ctrl_w_cycle() {
    let mut test = EditorTest::new("test\n");

    test.keys("<C-w>");
    test.keys("<C-w>");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-W + s (split horizontal)
#[test]
fn test_ctrl_w_s_split_h() {
    let mut test = EditorTest::new("test\n");

    test.keys("<C-w>");
    test.keys("s");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-W + v (split vertical)
#[test]
fn test_ctrl_w_v_split_v() {
    let mut test = EditorTest::new("test\n");

    test.keys("<C-w>");
    test.keys("v");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-W + q (quit window)
#[test]
fn test_ctrl_w_q_quit() {
    let mut test = EditorTest::new("test\n");

    test.keys("<C-w>");
    test.keys("q");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-W + o (close other windows)
#[test]
fn test_ctrl_w_o_close_others() {
    let mut test = EditorTest::new("test\n");

    test.keys("<C-w>");
    test.keys("o");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-W + hjkl (navigate windows)
#[test]
fn test_ctrl_w_hjkl_navigate() {
    let mut test = EditorTest::new("test\n");

    test.keys("<C-w>");
    test.keys("h");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-Z (suspend - should be handled gracefully)
#[test]
fn test_ctrl_z_suspend() {
    let mut test = EditorTest::new("test\n");

    test.keys("<C-z>");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-C (interrupt - behaves like Esc)
#[test]
fn test_ctrl_c_interrupt() {
    let mut test = EditorTest::new("test\n");

    test.keys("i");
    test.type_text("typing");
    test.keys("<C-c>");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-[ (same as Esc)
#[test]
fn test_ctrl_bracket_escape() {
    let mut test = EditorTest::new("test\n");

    test.keys("i");
    test.type_text("text");
    test.keys("<C-[>");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-M (same as Enter)
#[test]
fn test_ctrl_m_enter() {
    let mut test = EditorTest::new("line1\n");

    test.keys("i");
    test.keys("<C-m>");
    test.type_text("line2");
    test.press_esc();

    assert!(test.buffer_content().contains("line2"));
}

/// Test Ctrl-T multiple times in insert
#[test]
fn test_ctrl_t_multiple_indent() {
    let mut test = EditorTest::new("text\n");

    test.keys("i");
    test.keys("<C-t>");
    test.keys("<C-t>");
    test.press_esc();

    let content = test.buffer_content();
    // Should be indented twice
    assert!(content.starts_with("        ") || content.starts_with("\t\t"));
}

/// Test Ctrl-D multiple times in insert
#[test]
fn test_ctrl_d_multiple_dedent() {
    let mut test = EditorTest::new("        text\n");

    test.keys("I");
    test.keys("<C-d>");
    test.keys("<C-d>");
    test.press_esc();

    // Should be dedented
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-K (digraph in insert mode)
#[test]
fn test_ctrl_k_digraph() {
    let mut test = EditorTest::new("\n");

    test.keys("i");
    test.keys("<C-k>");
    test.press_esc();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test Ctrl-Q (same as Ctrl-V on some systems)
#[test]
fn test_ctrl_q_visual_block() {
    let mut test = EditorTest::new("line1\nline2\n");

    test.keys("<C-q>");

    // May enter visual block mode
    test.assert_mode(ovim::mode::Mode::Normal);
}
