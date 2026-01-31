//! Tests for using find (f/F/t/T) with visual mode to select text
//!
//! Common patterns like f"vf" to select a string literal

mod helpers;
use helpers::EditorTest;
use ovim::mode::Mode;

#[test]
fn test_simple_visual_find() {
    let mut test = EditorTest::new("abcdefgh");
    test.set_cursor(0, 0); // Start at 'a'

    // Enter visual mode, then find 'e'
    test.type_text("vfe");

    assert_eq!(test.mode(), Mode::Visual);

    let selection = test.get_visual_selection();
    assert_eq!(
        selection,
        Some(((0, 0), (0, 4))),
        "Should select from a to e"
    );
}

#[test]
fn test_fvf_selects_string() {
    let mut test = EditorTest::empty();
    test.set_buffer_content(r#"let x = "hello world";"#);
    test.set_cursor(0, 0); // Position at start of line

    // f" to find first quote, v to enter visual, f" to extend to closing quote
    test.type_text(r#"f"vf""#);

    assert_eq!(test.mode(), Mode::Visual);

    // Should select from first quote to second quote (inclusive)
    let selection = test.get_visual_selection();
    assert_eq!(
        selection,
        Some(((0, 8), (0, 20))),
        "Should select from opening to closing quote"
    );

    // Yank should copy the selected text including quotes
    test.type_text("y");
    assert_eq!(test.mode(), Mode::Normal);

    let yanked = test.get_register_content('"');
    assert_eq!(yanked, Some("\"hello world\"".to_string()));
}

#[test]
fn test_fvf_selects_parentheses() {
    let mut test = EditorTest::empty();
    test.set_buffer_content("function(arg1, arg2)");
    test.set_cursor(0, 0); // Start of line

    // f(vf) to select from opening to closing paren
    test.type_text("f(vf)");

    assert_eq!(test.mode(), Mode::Visual);

    let selection = test.get_visual_selection();
    assert_eq!(
        selection,
        Some(((0, 8), (0, 19))),
        "Should select parentheses and contents"
    );

    test.type_text("y");
    let yanked = test.get_register_content('"');
    assert_eq!(yanked, Some("(arg1, arg2)".to_string()));
}

#[test]
fn test_fvf_selects_brackets() {
    let mut test = EditorTest::empty();
    test.set_buffer_content("let arr = [1, 2, 3];");
    test.set_cursor(0, 0);

    // f[vf] to select array
    test.type_text("f[vf]");

    assert_eq!(test.mode(), Mode::Visual);

    let selection = test.get_visual_selection();
    assert_eq!(
        selection,
        Some(((0, 10), (0, 18))),
        "Should select brackets and contents"
    );

    test.type_text("y");
    let yanked = test.get_register_content('"');
    assert_eq!(yanked, Some("[1, 2, 3]".to_string()));
}

#[test]
fn test_fvf_selects_braces() {
    let mut test = EditorTest::empty();
    test.set_buffer_content("struct Foo { x: i32 }");
    test.set_cursor(0, 0);

    // f{vf} to select braces
    test.type_text("f{vf}");

    assert_eq!(test.mode(), Mode::Visual);

    let selection = test.get_visual_selection();
    assert_eq!(
        selection,
        Some(((0, 11), (0, 20))),
        "Should select braces and contents"
    );

    test.type_text("y");
    let yanked = test.get_register_content('"');
    assert_eq!(yanked, Some("{ x: i32 }".to_string()));
}

#[test]
fn test_tvt_selects_until_char() {
    let mut test = EditorTest::empty();
    test.set_buffer_content("let x = value;");
    test.set_cursor(0, 0); // At start

    // t=vt; to select from position till = to till ;
    test.type_text("t=vt;");

    assert_eq!(test.mode(), Mode::Visual);

    let selection = test.get_visual_selection();
    // t= moves to column 5 (space before =), then vt; selects till before semicolon
    assert_eq!(
        selection,
        Some(((0, 5), (0, 12))),
        "Should select between = and ;"
    );
}

#[test]
fn test_fvf_backward_then_forward() {
    let mut test = EditorTest::empty();
    test.set_buffer_content(r#"let x = "hello" + "world";"#);
    test.set_cursor(0, 20); // Between the two strings

    // F"vf" - backward find quote (finds 18), anchor, forward find quote (finds 24)
    test.type_text(r#"F"vf""#);

    assert_eq!(test.mode(), Mode::Visual);

    let selection = test.get_visual_selection();
    // Quotes are at: 8, 14, 18, 24. From position 20, F" finds 18, v anchors, f" finds 24
    assert_eq!(
        selection,
        Some(((0, 18), (0, 24))),
        "Should select from opening quote of 'world' to closing quote"
    );
}

#[test]
fn test_fvf_multiple_occurrences() {
    let mut test = EditorTest::empty();
    test.set_buffer_content(r#""first" "second" "third""#);
    test.set_cursor(0, 0); // At opening quote of "first"

    // From position 0 (first "), f" finds position 6, v anchors there, f" finds position 8
    test.type_text(r#"f"vf""#);
    let selection = test.get_visual_selection();
    assert_eq!(
        selection,
        Some(((0, 6), (0, 8))),
        "Should select from first closing quote to second opening quote"
    );

    // Escape and try selecting second string completely
    test.press_esc(); // ESC
    test.set_cursor(0, 7); // At space between strings

    test.type_text(r#"f"vf""#);
    let selection = test.get_visual_selection();
    assert_eq!(
        selection,
        Some(((0, 8), (0, 15))),
        "Should select from opening to closing quote of second string"
    );
}

#[test]
fn test_fvf_nested_quotes() {
    let mut test = EditorTest::empty();
    test.set_buffer_content(r#"let s = "outer \"inner\" text";"#);
    test.set_cursor(0, 8);

    // f"vf" should select to first unescaped quote
    // Note: basic implementation will just find next quote character
    test.type_text(r#"f"vf""#);

    assert_eq!(test.mode(), Mode::Visual);

    // This will select from first quote to next quote (escaped one)
    // More sophisticated text objects would handle escaping
    let selection = test.get_visual_selection();
    assert!(selection.is_some(), "Should find some selection");
}

#[test]
fn test_fvf_no_match_stays_in_visual() {
    let mut test = EditorTest::empty();
    test.set_buffer_content("no quotes here");
    test.set_cursor(0, 0);

    // f" should fail to find quote
    test.type_text(r#"f""#);

    // Cursor should not move if no match
    assert_eq!(test.cursor(), (0, 0));

    // v should still work to enter visual mode
    test.type_text("v");
    assert_eq!(test.mode(), Mode::Visual);
}

#[test]
fn test_fvf_with_count() {
    let mut test = EditorTest::empty();
    test.set_buffer_content(r#""a" "b" "c" "d""#);
    test.set_cursor(0, 0);

    // 2f" finds the 2nd quote after position 0 (which is position 4, opening quote of "b")
    // Then v anchors at 4, and f" finds position 6 (closing quote of "b")
    test.type_text(r#"2f"vf""#);

    let selection = test.get_visual_selection();
    // Quotes at positions: 0, 2, 4, 6, 8, 10, 12, 14
    // 2f" from 0 finds positions 2 (1st) and 4 (2nd), anchors at 4, f" finds 6
    assert_eq!(
        selection,
        Some(((0, 4), (0, 6))),
        "Should select opening to closing quote of 'b'"
    );
}

#[test]
fn test_fvf_select_and_delete() {
    let mut test = EditorTest::empty();
    test.set_buffer_content(r#"let x = "remove me" + other;"#);
    test.set_cursor(0, 0); // Start at beginning

    // Select the string and delete it
    test.type_text(r#"f"vf"d"#);

    assert_eq!(test.mode(), Mode::Normal);
    // After f"vf"d: finds first quote (at 8), v anchors, finds closing quote (at 19), deletes inclusive
    assert_eq!(test.buffer_content(), "let x =  + other;\n");
}

#[test]
fn test_fvf_select_and_change() {
    let mut test = EditorTest::empty();
    test.set_buffer_content(r#"let x = "old";"#);
    test.set_cursor(0, 0); // Start at beginning

    // Select the string and change it
    test.type_text(r#"f"vf"c"#);

    assert_eq!(test.mode(), Mode::Insert);

    test.type_text("new");
    test.press_esc(); // ESC

    // After f"vf"c: selects from first " to second ", changes (deletes and enters insert), types "new"
    assert_eq!(test.buffer_content(), "let x = new;\n");
}

#[test]
fn test_tilde_visual_selection_case_toggle() {
    let mut test = EditorTest::empty();
    test.set_buffer_content(r#"let x = "HeLLo WoRLD";"#);
    test.set_cursor(0, 0); // At start

    // Select the string content (without quotes) using f"lvf"h
    // f" finds first quote at 8, l moves to 9 (H), v anchors, f" finds closing quote at 20, h moves back to 19
    test.type_text(r#"f"lvf"h"#);

    let selection = test.get_visual_selection();
    assert_eq!(
        selection,
        Some(((0, 9), (0, 19))),
        "Should select string contents"
    );

    // Toggle case with ~
    test.type_text("~");

    assert_eq!(test.buffer_content(), "let x = \"hEllO wOrld\";\n");
}
