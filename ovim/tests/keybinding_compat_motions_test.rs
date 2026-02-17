mod helpers;

use helpers::EditorTest;

#[test]
fn h_m_l_move_within_viewport() {
    let content = (1..=30)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    let mut test = EditorTest::new(&content);
    test.editor.set_viewport_height(10);

    test.keys("H");
    assert_eq!(test.cursor().0, 0);

    test.keys("M");
    assert_eq!(test.cursor().0, 5);

    test.keys("L");
    assert_eq!(test.cursor().0, 9);
}

#[test]
fn g0_gdollar_gm_work_on_wrapped_screen_line() {
    let mut test = EditorTest::new("abcdefghijk\n");
    test.editor.options.wrap = true;
    test.editor.ensure_wrap_map(5);
    let start_col = 6;

    test.set_cursor(0, start_col);
    test.keys("g0");
    let g0_col = test.cursor().1;
    assert!(g0_col % 5 == 0);

    test.set_cursor(0, start_col);
    test.keys("g$");
    let g_end_col = test.cursor().1;
    assert!(g_end_col <= 10);
    assert!(g_end_col >= 4);

    test.set_cursor(0, start_col);
    test.keys("gm");
    let g_mid_col = test.cursor().1;
    assert!(g_mid_col <= 10);
    assert!(g_mid_col >= 2);
}

#[test]
fn gcaret_moves_to_first_non_blank_on_wrapped_screen_line() {
    let mut test = EditorTest::new("abcde   xyz\n");
    test.editor.options.wrap = true;
    test.editor.ensure_wrap_map(5);
    test.set_cursor(0, 6);

    test.keys("g^");
    assert_eq!(test.cursor(), (0, 8));
}

#[test]
fn gsemicolon_and_gcomma_navigate_change_list() {
    let mut test = EditorTest::new("a\nb\nc\n");

    test.keys("A1<Esc>");
    test.keys("jA2<Esc>");
    test.keys("jA3<Esc>");

    test.keys("g;");
    assert_eq!(test.cursor().0, 1);

    test.keys("g;");
    assert_eq!(test.cursor().0, 0);

    test.keys("g,");
    assert_eq!(test.cursor().0, 1);
}

#[test]
fn gquote_uses_linewise_mark_jump() {
    let mut test = EditorTest::new("alpha\n  beta\n");
    test.keys("2Gma");
    test.keys("1G");
    test.keys("g'a");

    assert_eq!(test.cursor().0, 1);
}
