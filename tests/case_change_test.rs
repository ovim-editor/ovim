use ovim::mode::Mode;

mod helpers;
use helpers::EditorTest;

#[test]
fn test_lowercase_inner_word() {
    let mut test = EditorTest::new("HELLO world");
    test.keys("0"); // Start at 'H'

    // Apply gu (lowercase) to inner word
    test.keys("guiw");

    // Should convert "HELLO" to "hello"
    let line = test.line(0).unwrap();
    println!("After guiw: '{}'", line);
    assert!(line.contains("hello"), "Should convert HELLO to hello, got: {}", line);
}

#[test]
fn test_uppercase_inner_word() {
    let mut test = EditorTest::new("hello WORLD");
    test.keys("0"); // Start at 'h'

    // Apply gU (uppercase) to inner word
    test.keys("gUiw");

    // Should convert "hello" to "HELLO"
    let line = test.line(0).unwrap();
    println!("After gUiw: '{}'", line);
    assert!(line.contains("HELLO WORLD"), "Should convert hello to HELLO, got: {}", line);
}

#[test]
fn test_lowercase_with_motion() {
    let mut test = EditorTest::new("HELLO WORLD");
    test.keys("0");

    // Apply gu with word motion
    test.keys("guw");

    let line = test.line(0).unwrap();
    println!("After guw: '{}'", line);
    assert!(line.starts_with("hello"), "Should convert first word to lowercase, got: {}", line);
}

#[test]
fn test_uppercase_line() {
    let mut test = EditorTest::new("hello world");
    test.keys("0");

    // Apply gU to entire line
    test.keys("gU$");

    let line = test.line(0).unwrap();
    println!("After gU$: '{}'", line);
    assert!(line.contains("HELLO WORLD"), "Should convert line to uppercase, got: {}", line);
}

#[test]
fn test_case_toggle() {
    let mut test = EditorTest::new("HeLLo WoRLd");
    test.keys("0");

    // Toggle case with ~
    test.keys("~");
    let pos1 = test.cursor();
    println!("After first ~, cursor: {:?}", pos1);

    test.keys("~");
    let pos2 = test.cursor();
    println!("After second ~, cursor: {:?}", pos2);

    let line = test.line(0).unwrap();
    println!("After ~~: '{}'", line);
    // ~ toggles case and moves cursor forward
}
