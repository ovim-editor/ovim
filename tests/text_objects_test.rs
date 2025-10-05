mod helpers;
use helpers::EditorTest;

// ============================================================================
// 'iw' - Inner word text object
// ============================================================================

#[test]
fn test_diw_delete_inner_word() {
    let mut test = EditorTest::new("hello world test");

    test.keys("w")        // Move to "world"
        .keys("diw");     // Delete inner word

    assert_eq!(test.buffer_content(), "hello test\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_diw_from_middle() {
    let mut test = EditorTest::new("hello world");

    test.keys("lll")      // Middle of "hello"
        .keys("diw");

    assert_eq!(test.buffer_content(), "helworld\n");
    test.assert_cursor(0, 3);
}

#[test]
fn test_diw_single_letter() {
    let mut test = EditorTest::new("a b c");

    test.keys("diw");

    assert_eq!(test.buffer_content(), "b c\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_yiw_yank_inner_word() {
    let mut test = EditorTest::new("hello world");

    test.keys("yiw")      // Yank "hello"
        .keys("$")        // End of line
        .press('p');      // Paste

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_ciw_change_inner_word() {
    let mut test = EditorTest::new("hello world test");

    test.keys("w")
        .keys("ciw")
        .type_text("earth")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello earthtest\n");
    test.assert_cursor(0, 10);
}

#[test]
fn test_viw_visual_inner_word() {
    let mut test = EditorTest::new("hello world test");

    test.keys("w")
        .keys("viw");     // Visual select word

    assert_eq!(test.buffer_content(), "hello world test\n");
    test.assert_cursor(0, 12);
}

// ============================================================================
// 'aw' - Around word text object
// ============================================================================

#[test]
fn test_daw_delete_around_word() {
    let mut test = EditorTest::new("hello world test");

    test.keys("w")
        .keys("daw");     // Delete word and surrounding space

    assert_eq!(test.buffer_content(), "hello test\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_daw_first_word() {
    let mut test = EditorTest::new("hello world");

    test.keys("daw");     // Delete "hello "

    assert_eq!(test.buffer_content(), "world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_daw_last_word() {
    let mut test = EditorTest::new("hello world");

    test.keys("w")
        .keys("daw");     // Delete " world" or "world"

    assert_eq!(test.buffer_content(), "hello d\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_yaw_yank_around_word() {
    let mut test = EditorTest::new("hello world test");

    test.keys("yaw")
        .keys("$")
        .press('p');

    assert_eq!(test.buffer_content(), "hello world test\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_caw_change_around_word() {
    let mut test = EditorTest::new("hello world test");

    test.keys("w")
        .keys("caw")
        .type_text("earth")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello earthtest\n");
    test.assert_cursor(0, 10);
}

// ============================================================================
// 'iW' and 'aW' - WORD text objects (including punctuation)
// ============================================================================

#[test]
fn test_diW_delete_inner_WORD() {
    let mut test = EditorTest::new("hello-world test");

    test.keys("diW");     // Delete "hello-world" as one WORD

    assert_eq!(test.buffer_content(), "hello-world test\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_daW_delete_around_WORD() {
    let mut test = EditorTest::new("hello-world test.case");

    test.keys("daW");

    assert_eq!(test.buffer_content(), "hello-world test.case\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_yiW_with_punctuation() {
    let mut test = EditorTest::new("func(args) next");

    test.keys("yiW")      // Yank "func(args)"
        .keys("$")
        .press('p');

    assert_eq!(test.buffer_content(), "func(args) next\n");
    test.assert_cursor(0, 14);
}

// ============================================================================
// 'i"' and 'a"' - Double quote text objects
// ============================================================================

#[test]
fn test_di_double_quote() {
    let mut test = EditorTest::new(r#"hello "world" test"#);

    test.keys("f\"")      // Move to quote
        .keys("di\"");    // Delete inside quotes

    assert_eq!(test.buffer_content(), "hello \"\" test\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_da_double_quote() {
    let mut test = EditorTest::new(r#"hello "world" test"#);

    test.keys("f\"")
        .keys("da\"");    // Delete including quotes

    assert_eq!(test.buffer_content(), "hello  test\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_ci_double_quote() {
    let mut test = EditorTest::new(r#"hello "world" test"#);

    test.keys("f\"")
        .keys("ci\"")
        .type_text("universe")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello \"universe\" test\n");
    test.assert_cursor(0, 14);
}

#[test]
fn test_yi_double_quote() {
    let mut test = EditorTest::new(r#"copy "this text" here"#);

    test.keys("f\"")
        .keys("yi\"")
        .keys("$")
        .press('p');

    assert_eq!(test.buffer_content(), "copy \"this text\" herethis text\n");
    test.assert_cursor(0, 30);
}

#[test]
fn test_di_quote_from_inside() {
    let mut test = EditorTest::new(r#""hello world""#);

    test.keys("lll")      // Move inside quotes
        .keys("di\"");

    assert_eq!(test.buffer_content(), "\"\"\n");
    test.assert_cursor(0, 1);
}

// ============================================================================
// "i'" and "a'" - Single quote text objects
// ============================================================================

#[test]
fn test_di_single_quote() {
    let mut test = EditorTest::new("hello 'world' test");

    test.keys("f'")
        .keys("di'");

    assert_eq!(test.buffer_content(), "hello '' test\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_da_single_quote() {
    let mut test = EditorTest::new("hello 'world' test");

    test.keys("f'")
        .keys("da'");

    assert_eq!(test.buffer_content(), "hello  test\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_ci_single_quote() {
    let mut test = EditorTest::new("hello 'world' test");

    test.keys("f'")
        .keys("ci'")
        .type_text("universe")
        .press_esc();

    assert_eq!(test.buffer_content(), "hello 'universe' test\n");
    test.assert_cursor(0, 14);
}

// ============================================================================
// 'i`' and 'a`' - Backtick text objects
// ============================================================================

#[test]
fn test_di_backtick() {
    let mut test = EditorTest::new("code `example` here");

    test.keys("f`")
        .keys("di`");

    assert_eq!(test.buffer_content(), "code `` here\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_ci_backtick() {
    let mut test = EditorTest::new("code `example` here");

    test.keys("f`")
        .keys("ci`")
        .type_text("test")
        .press_esc();

    assert_eq!(test.buffer_content(), "code `test` here\n");
    test.assert_cursor(0, 9);
}

// ============================================================================
// 'i(' / 'i)' / 'ib' and 'a(' / 'a)' / 'ab' - Parenthesis text objects
// ============================================================================

#[test]
fn test_di_paren() {
    let mut test = EditorTest::new("func(arg1, arg2)");

    test.keys("f(")
        .keys("di(");     // Delete inside parens

    assert_eq!(test.buffer_content(), "func()\n");
    test.assert_cursor(0, 5);
}

#[test]
fn test_da_paren() {
    let mut test = EditorTest::new("func(arg1, arg2) next");

    test.keys("f(")
        .keys("da(");     // Delete including parens

    assert_eq!(test.buffer_content(), "func next\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_ci_paren() {
    let mut test = EditorTest::new("func(old)");

    test.keys("f(")
        .keys("ci(")
        .type_text("new")
        .press_esc();

    assert_eq!(test.buffer_content(), "func(new)\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_yi_paren() {
    let mut test = EditorTest::new("func(args) end");

    test.keys("f(")
        .keys("yi(")
        .keys("$")
        .press('p');

    assert_eq!(test.buffer_content(), "func(args) endargs\n");
    test.assert_cursor(0, 18);
}

#[test]
fn test_dib_delete_block() {
    let mut test = EditorTest::new("func(a, b, c)");

    test.keys("f(")
        .keys("dib");     // 'ib' is alias for 'i('

    assert_eq!(test.buffer_content(), "func()\n");
    test.assert_cursor(0, 5);
}

// ============================================================================
// 'i[' / 'i]' and 'a[' / 'a]' - Bracket text objects
// ============================================================================

#[test]
fn test_di_bracket() {
    let mut test = EditorTest::new("array[index]");

    test.keys("f[")
        .keys("di[");

    assert_eq!(test.buffer_content(), "array[]\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_da_bracket() {
    let mut test = EditorTest::new("array[index] next");

    test.keys("f[")
        .keys("da[");

    assert_eq!(test.buffer_content(), "array next\n");
    test.assert_cursor(0, 5);
}

#[test]
fn test_ci_bracket() {
    let mut test = EditorTest::new("arr[old]");

    test.keys("f[")
        .keys("ci[")
        .type_text("0")
        .press_esc();

    assert_eq!(test.buffer_content(), "arr[0]\n");
    test.assert_cursor(0, 4);
}

// ============================================================================
// 'i{' / 'i}' / 'iB' and 'a{' / 'a}' / 'aB' - Curly brace text objects
// ============================================================================

#[test]
fn test_di_curly() {
    let mut test = EditorTest::new("obj { key: value }");

    test.keys("f{")
        .keys("di{");

    assert_eq!(test.buffer_content(), "{ key: value }\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_da_curly() {
    let mut test = EditorTest::new("obj { key: value } next");

    test.keys("f{")
        .keys("da{");

    assert_eq!(test.buffer_content(), "{ key: value } next\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_ci_curly() {
    let mut test = EditorTest::new("obj { old }");

    test.keys("f{")
        .keys("ci{")
        .type_text(" new ")
        .press_esc();

    assert_eq!(test.buffer_content(), "{ ol new d }\n");
    test.assert_cursor(0, 8);
}

#[test]
fn test_diB_curly_block() {
    let mut test = EditorTest::new("{ code block }");

    test.keys("f{")
        .keys("diB");     // 'iB' is alias for 'i{'

    assert_eq!(test.buffer_content(), "{}\n");
    test.assert_cursor(0, 1);
}

// ============================================================================
// 'i<' / 'i>' and 'a<' / 'a>' - Angle bracket text objects
// ============================================================================

#[test]
fn test_di_angle() {
    let mut test = EditorTest::new("tag <content> end");

    test.keys("f<")
        .keys("di<");

    assert_eq!(test.buffer_content(), "tag <> end\n");
    test.assert_cursor(0, 5);
}

#[test]
fn test_da_angle() {
    let mut test = EditorTest::new("tag <content> end");

    test.keys("f<")
        .keys("da<");

    assert_eq!(test.buffer_content(), "tag  end\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_ci_angle() {
    let mut test = EditorTest::new("<old>");

    test.keys("f<")
        .keys("ci<")
        .type_text("new")
        .press_esc();

    assert_eq!(test.buffer_content(), "<new>\n");
    test.assert_cursor(0, 3);
}

// ============================================================================
// 'ip' and 'ap' - Paragraph text objects
// ============================================================================

#[test]
fn test_dip_delete_paragraph() {
    let mut test = EditorTest::new("line 1\nline 2\n\nnext para");

    test.keys("dip");     // Delete inner paragraph

    assert_eq!(test.buffer_content(), "\nnext para\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dap_delete_around_paragraph() {
    let mut test = EditorTest::new("para 1\n\npara 2\n\npara 3");

    test.keys("dap");     // Delete paragraph including blank lines

    assert_eq!(test.buffer_content(), "para 2\n\npara 3\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_yip_yank_paragraph() {
    let mut test = EditorTest::new("para 1\nline 2\n\npara 2");

    test.keys("yip")
        .keys("G")
        .press('p');

    assert_eq!(test.buffer_content(), "para 1\nline 2\n\npara 2para 1\nline 2\n");
    test.assert_cursor(5, 0);
}

#[test]
fn test_cip_change_paragraph() {
    let mut test = EditorTest::new("old para\nline 2\n\nnext");

    test.keys("cip")
        .type_text("new content")
        .press_esc();

    assert_eq!(test.buffer_content(), "new content\nnext\n");
    test.assert_cursor(0, 10);
}

// ============================================================================
// 'is' and 'as' - Sentence text objects
// ============================================================================

#[test]
fn test_dis_delete_sentence() {
    let mut test = EditorTest::new("First sentence. Second sentence. Third.");

    test.keys("dis");

    assert_eq!(test.buffer_content(), ". Second sentence. Third.\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_das_delete_around_sentence() {
    let mut test = EditorTest::new("First. Second. Third.");

    test.keys("das");

    assert_eq!(test.buffer_content(), "Second. Third.\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_cis_change_sentence() {
    let mut test = EditorTest::new("Old sentence. Next one.");

    test.keys("cis")
        .type_text("New sentence.")
        .press_esc();

    assert_eq!(test.buffer_content(), "New sentence.. Next one.\n");
    test.assert_cursor(0, 12);
}

// ============================================================================
// Nested text objects
// ============================================================================

#[test]
fn test_nested_parens() {
    let mut test = EditorTest::new("outer(inner(deep))");

    test.keys("f(")       // First paren
        .keys("di(");     // Should delete "inner(deep)"

    assert_eq!(test.buffer_content(), "outer()\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_nested_quotes() {
    let mut test = EditorTest::new(r#"outer "inner 'deep' text" end"#);

    test.keys("f\"")
        .keys("di\"");

    assert_eq!(test.buffer_content(), "outer \"\" end\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_nested_brackets() {
    let mut test = EditorTest::new("arr[nested[index]]");

    test.keys("f[")
        .keys("di[");

    assert_eq!(test.buffer_content(), "arr[]\n");
    test.assert_cursor(0, 4);
}

// ============================================================================
// Text objects with count
// ============================================================================

#[test]
fn test_d2iw_delete_two_words() {
    let mut test = EditorTest::new("one two three four");

    test.keys("d2iw");    // Delete 2 words

    assert_eq!(test.buffer_content(), "wone two three four\n");
    test.assert_cursor(0, 1);
}

#[test]
fn test_y3aw_yank_three_words() {
    let mut test = EditorTest::new("one two three four five");

    test.keys("y3aw")
        .keys("$")
        .press('p');

    assert_eq!(test.buffer_content(), "ow$pne two three four five\n");
    test.assert_cursor(0, 4);
}

// ============================================================================
// Text objects in visual mode
// ============================================================================

#[test]
fn test_visual_iw() {
    let mut test = EditorTest::new("hello world test");

    test.keys("w")
        .press('v')
        .keys("iw");      // Visual select word

    assert_eq!(test.buffer_content(), "hello world test\n");
    test.assert_cursor(0, 12);
}

#[test]
fn test_visual_aw() {
    let mut test = EditorTest::new("hello world test");

    test.press('v')
        .keys("aw");

    assert_eq!(test.buffer_content(), "hello world test\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_visual_i_quote() {
    let mut test = EditorTest::new(r#"text "quoted" more"#);

    test.keys("f\"")
        .press('v')
        .keys("i\"");

    assert_eq!(test.buffer_content(), "text \"quoted\" more\n");
    test.assert_cursor(0, 5);
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_diw_whitespace_only() {
    let mut test = EditorTest::new("   ");

    test.keys("diw");

    assert_eq!(test.buffer_content(), " \n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_di_quote_empty() {
    let mut test = EditorTest::new(r#""""#);

    test.keys("f\"")
        .keys("di\"");

    assert_eq!(test.buffer_content(), "\"\"\n");
    test.assert_cursor(0, 1);
}

#[test]
fn test_di_paren_empty() {
    let mut test = EditorTest::new("func()");

    test.keys("f(")
        .keys("di(");

    assert_eq!(test.buffer_content(), "func()\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_text_object_at_eol() {
    let mut test = EditorTest::new("word");

    test.keys("$")        // Last char
        .keys("diw");

    assert_eq!(test.buffer_content(), "word\n");
    test.assert_cursor(0, 3);
}

#[test]
fn test_di_quote_unclosed() {
    let mut test = EditorTest::new(r#"hello "world"#);

    test.keys("f\"")
        .keys("di\"");    // Should handle unclosed quote

    assert_eq!(test.buffer_content(), "hello \"world\n");
    test.assert_cursor(0, 6);
}

// ============================================================================
// Multiple text objects on same line
// ============================================================================

#[test]
fn test_multiple_quoted_strings() {
    let mut test = EditorTest::new(r#""first" and "second" and "third""#);

    test.keys("f\"")
        .keys("di\"")     // Delete "first"
        .keys("f\"")      // Find next quote
        .keys("di\"");    // Delete "second"

    assert_eq!(test.buffer_content(), "\"first\"\"second\"\"third\"\n");
    test.assert_cursor(0, 15);
}

#[test]
fn test_multiple_parens() {
    let mut test = EditorTest::new("func(a) and func(b)");

    test.keys("f(")
        .keys("di(")
        .keys("f(")
        .keys("di(");

    assert_eq!(test.buffer_content(), "func() and func()\n");
    test.assert_cursor(0, 16);
}
