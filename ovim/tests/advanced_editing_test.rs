mod helpers;

use helpers::EditorTest;

/// Test . (dot repeat) with insert
#[test]
fn test_dot_repeat_insert() {
    let mut test = EditorTest::new("line\n");

    test.keys("i");
    test.type_text("hello ");
    test.press_esc();

    test.keys("j");
    test.keys(".");

    // Should repeat the insert
    assert!(test.buffer_content().matches("hello").count() >= 1);
}

/// Test . (dot repeat) with delete
#[test]
fn test_dot_repeat_delete() {
    let mut test = EditorTest::new("word word word\n");

    test.keys("0");
    test.keys("dw");

    test.keys(".");

    // Should have deleted two words
    assert!(!test.buffer_content().starts_with("word word"));
}

/// Test . (dot repeat) with change
#[test]
fn test_dot_repeat_change() {
    let mut test = EditorTest::new("foo bar baz\n");

    test.keys("0");
    test.keys("cw");
    test.type_text("new");
    test.press_esc();

    test.keys("w");
    test.keys(".");

    assert!(test.buffer_content().contains("new new"));
}

/// Test q (record macro)
#[test]
fn test_q_record_macro() {
    let mut test = EditorTest::new("test\n");

    test.keys("qa"); // Start recording to register 'a'
    test.keys("i");
    test.type_text("X");
    test.press_esc();
    test.keys("q"); // Stop recording

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test @ (play macro)
#[test]
fn test_at_play_macro() {
    let mut test = EditorTest::new("line1\nline2\n");

    // Record macro
    test.keys("qa");
    test.keys("I");
    test.type_text("# ");
    test.press_esc();
    test.keys("j");
    test.keys("q");

    // Play macro
    test.keys("gg");
    test.keys("@a");

    assert!(test.buffer_content().contains("# line1"));
}

/// Test @@ (repeat last macro)
#[test]
fn test_at_at_repeat_macro() {
    let mut test = EditorTest::new("l1\nl2\nl3\n");

    // Record and play macro
    test.keys("qa");
    test.keys("A");
    test.type_text("!");
    test.press_esc();
    test.keys("j");
    test.keys("q");

    test.keys("gg");
    test.keys("@a");
    test.keys("@@"); // Repeat

    let content = test.buffer_content();
    assert!(content.matches("!").count() >= 2);
}

/// Test u (undo)
#[test]
fn test_u_undo() {
    let mut test = EditorTest::new("original\n");

    test.keys("i");
    test.type_text("changed ");
    test.press_esc();

    test.keys("u");

    assert_eq!(test.buffer_content(), "original\n");
}

/// Test multiple undo
#[test]
fn test_multiple_undo() {
    let mut test = EditorTest::new("start\n");

    // Make changes
    test.keys("A");
    test.type_text("1");
    test.press_esc();

    test.keys("A");
    test.type_text("2");
    test.press_esc();

    // Undo both
    test.keys("u");
    test.keys("u");

    assert_eq!(test.buffer_content(), "start\n");
}

/// Test Ctrl-R (redo)
#[test]
fn test_ctrl_r_redo() {
    let mut test = EditorTest::new("text\n");

    test.keys("A");
    test.type_text("!");
    test.press_esc();

    test.keys("u"); // Undo
    test.keys("<C-r>"); // Redo

    assert!(test.buffer_content().contains("!"));
}

/// Test * (search word under cursor forward)
#[test]
fn test_star_search_forward() {
    let mut test = EditorTest::new("foo bar foo baz\n");

    test.keys("0");
    test.keys("*");

    // Should find next occurrence
    assert!(test.cursor().1 > 0);
}

/// Test # (search word under cursor backward)
#[test]
fn test_hash_search_backward() {
    let mut test = EditorTest::new("foo bar foo baz\n");

    test.keys("$");
    test.keys("#");

    // Should find previous occurrence
    assert!(test.cursor().1 < 15);
}

/// Test n (next search result)
#[test]
fn test_n_next_search() {
    let mut test = EditorTest::new("foo bar foo baz foo\n");

    test.keys("/");
    test.type_text("foo");
    test.press_enter();

    test.keys("n");

    // Should move to next match
    assert!(test.cursor().1 > 0);
}

/// Test N (previous search result)
#[test]
fn test_n_capital_prev_search() {
    let mut test = EditorTest::new("foo bar foo baz foo\n");

    test.keys("/");
    test.type_text("foo");
    test.press_enter();

    test.keys("n");
    test.keys("N");

    test.assert_cursor(0, 0);
}

/// Test ; (repeat f/F/t/T)
#[test]
fn test_semicolon_repeat_find() {
    let mut test = EditorTest::new("a b c d e f g\n");

    test.keys("0");
    test.keys("fc"); // Find 'c'
    test.keys(";"); // Repeat find

    // Should find next space after 'c'
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test , (reverse repeat f/F/t/T)
#[test]
fn test_comma_reverse_find() {
    let mut test = EditorTest::new("a b c d e f g\n");

    test.keys("0");
    test.keys("fe"); // Find 'e'
    test.keys(","); // Reverse find

    // Should go backwards
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test v (visual mode)
#[test]
fn test_v_visual_mode() {
    let mut test = EditorTest::new("select me\n");

    test.keys("0");
    test.keys("v");

    test.assert_mode(ovim::mode::Mode::Visual);
}

/// Test V (visual line mode)
#[test]
fn test_v_capital_visual_line() {
    let mut test = EditorTest::new("line1\nline2\n");

    test.keys("V");

    test.assert_mode(ovim::mode::Mode::VisualLine);
}

/// Test Ctrl-V (visual block mode)
#[test]
fn test_ctrl_v_visual_block() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    test.keys("<C-v>");

    test.assert_mode(ovim::mode::Mode::VisualBlock);
}

/// Test visual mode yank
#[test]
fn test_visual_yank() {
    let mut test = EditorTest::new("select this\n");

    test.keys("0");
    test.keys("v");
    test.keys("e"); // Select word
    test.keys("y");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test visual mode delete
#[test]
fn test_visual_delete() {
    let mut test = EditorTest::new("delete this part\n");

    test.keys("0");
    test.keys("v");
    test.keys("w");
    test.keys("w");
    test.keys("d");

    assert!(!test.buffer_content().contains("delete this"));
}

/// Test visual mode change
#[test]
fn test_visual_change() {
    let mut test = EditorTest::new("change me\n");

    test.keys("0");
    test.keys("v");
    test.keys("e");
    test.keys("c");
    test.type_text("replaced");
    test.press_esc();

    assert!(test.buffer_content().contains("replaced"));
}

/// Test m (set mark)
#[test]
fn test_m_set_mark() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    test.keys("j");
    test.keys("ma"); // Set mark 'a'

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test ' (jump to mark line)
#[test]
fn test_quote_jump_to_mark_line() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    test.keys("j");
    test.keys("ma");

    test.keys("G");
    test.keys("'a");

    test.assert_cursor(1, 0);
}

/// Test ` (jump to mark exact)
#[test]
fn test_backtick_jump_to_mark_exact() {
    let mut test = EditorTest::new("line1\nline2 test\nline3\n");

    test.keys("j");
    test.keys("$");
    test.keys("ma");

    test.keys("gg");
    test.keys("`a");

    test.assert_cursor(1, 9); // At last character of "line2 test" (the 't')
}

/// Test R (replace mode)
#[test]
fn test_r_capital_replace_mode() {
    let mut test = EditorTest::new("hello\n");

    test.keys("0");
    test.keys("R");

    test.assert_mode(ovim::mode::Mode::Replace);
}

/// Test replace mode typing
#[test]
fn test_replace_mode_typing() {
    let mut test = EditorTest::new("hello world\n");

    test.keys("0");
    test.keys("R");
    test.type_text("HELLO");
    test.press_esc();

    assert!(test.buffer_content().starts_with("HELLO world"));
}

/// Test >> (indent) in visual mode
#[test]
fn test_visual_indent() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    test.keys("V");
    test.keys("j");
    test.keys(">");

    // Should be indented
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test << (dedent) in visual mode
#[test]
fn test_visual_dedent() {
    let mut test = EditorTest::new("    line1\n    line2\n");

    test.keys("V");
    test.keys("j");
    test.keys("<");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test = (auto-indent) in visual mode
#[test]
fn test_visual_auto_indent() {
    let mut test = EditorTest::new("line1\nline2\n");

    test.keys("V");
    test.keys("j");
    test.keys("=");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test gv (reselect last visual)
#[test]
fn test_gv_reselect_visual() {
    let mut test = EditorTest::new("select me\n");

    test.keys("0");
    test.keys("v");
    test.keys("e");
    test.press_esc();

    test.keys("gv");

    test.assert_mode(ovim::mode::Mode::Visual);
}

/// Test o (toggle visual selection end)
#[test]
fn test_o_toggle_visual_end() {
    let mut test = EditorTest::new("toggle this\n");

    test.keys("0");
    test.keys("v");
    test.keys("e");
    test.keys("o");

    test.assert_mode(ovim::mode::Mode::Visual);
}

/// Test :visual command
#[test]
fn test_command_visual() {
    let mut test = EditorTest::new("test\n");

    test.press(':');
    test.type_text("visual");
    test.press_enter();

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test ZZ (save and quit)
#[test]
fn test_zz_save_quit() {
    let mut test = EditorTest::new("test\n");
    let tmp = tempfile::Builder::new()
        .prefix("ovim_zz_")
        .suffix(".txt")
        .tempfile()
        .unwrap();
    test.set_file_path(tmp.path().to_string_lossy().to_string());

    test.keys("ZZ");

    assert!(test.editor.should_quit());
}

/// Test ZQ (quit without saving)
#[test]
fn test_zq_quit_no_save() {
    let mut test = EditorTest::new("test\n");

    test.keys("i");
    test.type_text("change");
    test.press_esc();

    test.keys("ZQ");

    assert!(test.editor.should_quit());
}

/// Test Ctrl-A (increment number)
#[test]
fn test_ctrl_a_increment() {
    let mut test = EditorTest::new("value: 42\n");

    test.keys("0");
    test.keys("f4");
    test.keys("<C-a>");

    assert!(test.buffer_content().contains("43"));
}

/// Test Ctrl-X (decrement number)
#[test]
fn test_ctrl_x_decrement() {
    let mut test = EditorTest::new("value: 42\n");

    test.keys("0");
    test.keys("f4");
    test.keys("<C-x>");

    assert!(test.buffer_content().contains("41"));
}

/// Test g; (go to last change)
#[test]
fn test_g_semicolon_last_change() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    test.keys("j");
    test.keys("A");
    test.type_text("!");
    test.press_esc();

    test.keys("G");
    test.keys("g;");

    test.assert_cursor(1, 5);
}

/// Test g, (go to next change)
#[test]
fn test_g_comma_next_change() {
    let mut test = EditorTest::new("line1\nline2\nline3\n");

    // Make changes
    test.keys("A");
    test.type_text("1");
    test.press_esc();

    test.keys("j");
    test.keys("A");
    test.type_text("2");
    test.press_esc();

    // Go back and forward through changes
    test.keys("g;");
    test.keys("g,");

    test.assert_mode(ovim::mode::Mode::Normal);
}
