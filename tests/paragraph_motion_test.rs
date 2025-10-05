use ovim::mode::Mode;

mod helpers;
use helpers::EditorTest;

#[test]
fn test_paragraph_forward() {
    let mut test = EditorTest::new("line 1\nline 2\n\nline 4\nline 5");
    test.keys("gg"); // Go to top

    // From line 0, } should move to line 2 (blank line)
    test.keys("}");
    assert_eq!(test.cursor(), (2, 0), "Should move to blank line at (2, 0)");

    // From line 2, } should move to end (or next paragraph)
    test.keys("}");
    // Depending on implementation, could be (4, 0) or end of buffer
}

#[test]
fn test_paragraph_backward() {
    let mut test = EditorTest::new("line 1\nline 2\n\nline 4\nline 5");
    test.keys("G"); // Go to last line (line 4)

    // From line 4, { should move to line 2 (blank line) or line 3?
    test.keys("{");
    // Need to check what the expected behavior is
    let pos = test.cursor();
    println!("After {{ from last line: {:?}", pos);
}

#[test]
fn test_paragraph_backward_from_end() {
    // This is the test case from the bug report
    let mut test = EditorTest::new("paragraph 1 line 1\nparagraph 1 line 2\n\nparagraph 2 line 1\nparagraph 2 line 2");

    // Go to last line
    test.keys("G");
    let end_pos = test.cursor();
    println!("End position: {:?}", end_pos);

    // Move backward with {
    test.keys("{");
    let pos = test.cursor();
    println!("After {{: {:?}", pos);

    // Expected: should be at start of previous paragraph (line 2, the blank line, or line 3?)
    // The bug report says: Expected (2, 0), Actual (3, 0)
    // This suggests it should go to the blank line separator
}

#[test]
fn test_multiple_paragraphs() {
    let mut test = EditorTest::new("para1 line1\npara1 line2\n\npara2 line1\npara2 line2\n\npara3 line1");

    test.keys("G"); // Last line (para3 line1) - line 6

    // First { - should go to blank line before para3 (line 5)
    test.keys("{");
    println!("First {{: {:?}", test.cursor());

    // Second { - should go to blank line before para2 (line 2)
    test.keys("{");
    println!("Second {{: {:?}", test.cursor());

    // Third { - should go to start of file (line 0)
    test.keys("{");
    println!("Third {{: {:?}", test.cursor());
}
