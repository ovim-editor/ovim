mod helpers;
use helpers::EditorTest;
use ovim::mode::Mode;

#[test]
fn test_delete_inside_braces() {
    let mut test = EditorTest::new("{hello world}");

    // Delete inside braces - should leave just {}
    test.keys("di{");

    assert_eq!(test.buffer_content(), "{}\n");
}

#[test]
fn test_delete_around_braces() {
    let mut test = EditorTest::new("{hello world}");

    // Delete around braces - should delete everything including braces
    test.keys("da{");

    assert_eq!(test.buffer_content(), "\n");
}

#[test]
fn test_delete_inside_braces_with_cursor_inside() {
    let mut test = EditorTest::new("{hello world}");

    // Move cursor inside the braces
    test.keys("5ldi{");

    assert_eq!(test.buffer_content(), "{}\n");
}

#[test]
fn test_delete_inside_nested_braces() {
    let mut test = EditorTest::new("{outer {inner} text}");

    // Move to the inner braces and delete
    test.keys("8ldi{");

    assert_eq!(test.buffer_content(), "{outer {} text}\n");
}

#[test]
fn test_change_inside_braces() {
    let mut test = EditorTest::new("{hello}");

    // Change inside braces should delete content and enter insert mode
    test.keys("ci{");

    assert_eq!(test.buffer_content(), "{}\n");
    assert_eq!(test.editor.mode(), Mode::Insert);
}

#[test]
fn test_yank_inside_braces() {
    let mut test = EditorTest::new("{hello}");

    // Yank inside braces
    test.keys("yi{");

    // Content should be unchanged
    assert_eq!(test.buffer_content(), "{hello}\n");

    // But we should be able to paste it
    test.keys("p");
    assert_eq!(test.buffer_content(), "{hellohello}\n");
}

#[test]
fn test_di_braces_vs_d_braces_different() {
    // Test that di{ (delete inside braces) is different from d{ (delete to prev paragraph)

    let mut test1 = EditorTest::new("{hello}");
    test1.keys("di{");
    assert_eq!(
        test1.buffer_content(),
        "{}\n",
        "di{{ should delete inside braces"
    );

    // For d{, we need a multi-line setup to test paragraph motion
    let mut test2 = EditorTest::new("line1\n\n{hello}");
    test2.keys("Gd{"); // Go to last line, then d{
                       // d{ should delete to previous paragraph, not inside braces
                       // The exact behavior depends on paragraph definition, but it shouldn't be empty braces
    let result = test2.buffer_content();
    assert_ne!(
        result, "{}\n",
        "d{{ should not result in empty braces (paragraph motion, not text object)"
    );
}

#[test]
fn test_delete_inside_brackets() {
    let mut test = EditorTest::new("[hello world]");

    test.keys("di[");

    assert_eq!(test.buffer_content(), "[]\n");
}

#[test]
fn test_delete_inside_parens() {
    let mut test = EditorTest::new("(hello world)");

    test.keys("di(");

    assert_eq!(test.buffer_content(), "()\n");
}

#[test]
fn test_delete_inside_quotes() {
    let mut test = EditorTest::new("\"hello world\"");

    // Move into the quotes first
    test.keys("ldi\"");

    assert_eq!(test.buffer_content(), "\"\"\n");
}
