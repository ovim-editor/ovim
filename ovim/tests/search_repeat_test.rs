#![allow(non_snake_case)]

mod helpers;
use helpers::EditorTest;

#[test]
fn test_search_forward_repeat() {
    let mut test = EditorTest::new("hello world\nhello there\nhello again");
    test.keys("gg"); // Go to top

    // Search for "hello"
    test.keys("/hello");
    test.press_enter();

    // nvim --clean: the match under the cursor is skipped
    assert_eq!(
        test.cursor(),
        (1, 0),
        "Should find second 'hello' at (1, 0)"
    );

    // Press 'n' to find next
    test.keys("n");
    assert_eq!(test.cursor(), (2, 0), "Should find third 'hello' at (2, 0)");

    // Press 'n' again: wraps around
    test.keys("n");
    assert_eq!(
        test.cursor(),
        (0, 0),
        "Should wrap to first 'hello' at (0, 0)"
    );
}

#[test]
fn test_search_backward_repeat() {
    let mut test = EditorTest::new("hello world\nhello there\nhello again");
    test.keys("gg"); // Go to top

    // Search backward for "hello"
    // nvim --clean: the preview of each typed character searches from where the
    // search began, so the result is the last 'hello' (wrapping from the top).
    test.keys("?hello");
    test.press_enter();
    assert_eq!(test.cursor(), (2, 0), "Should wrap to the last 'hello'");

    // Press 'n' to find previous (going backward)
    test.keys("n");
    assert_eq!(test.cursor(), (1, 0), "Should find the middle 'hello'");

    // Press 'n' again
    test.keys("n");
    assert_eq!(test.cursor(), (0, 0), "Should find first 'hello' at (0, 0)");
}

#[test]
fn test_search_with_N() {
    let mut test = EditorTest::new("hello world\nhello there\nhello again");
    test.keys("gg"); // Go to top

    // Search forward for "hello"
    test.keys("/hello");
    test.press_enter();
    // nvim --clean: /hello from (0,0) skips the match under the cursor.
    assert_eq!(
        test.cursor(),
        (1, 0),
        "Should find second 'hello' at (1, 0)"
    );

    // Press 'N' to search in opposite direction (backward)
    test.keys("N");
    assert_eq!(test.cursor(), (0, 0), "N goes backward to (0, 0)");

    // Press 'N' again
    test.keys("N");
    assert_eq!(
        test.cursor(),
        (2, 0),
        "N wraps backward to the last 'hello' at (2, 0)"
    );
}

#[test]
fn test_search_from_middle_of_match() {
    let mut test = EditorTest::new("hello world hello there hello again");
    test.keys("0"); // Start of line

    // Search for "hello"
    test.keys("/hello");
    test.press_enter();
    // nvim --clean: the match under the cursor is skipped, so this lands on column 12
    assert_eq!(
        test.cursor(),
        (0, 12),
        "Should find second 'hello' at column 12"
    );

    // Move cursor into the middle of the match
    test.keys("ll"); // Move right 2 positions (now at column 14, inside "hello")
    assert_eq!(test.cursor(), (0, 14), "Cursor should be at column 14");

    // Press 'n' - should find NEXT hello, not current one
    test.keys("n");
    assert_eq!(
        test.cursor(),
        (0, 24),
        "Should find next 'hello' at column 24, not stay on current"
    );
}

#[test]
fn test_search_no_matches() {
    let mut test = EditorTest::new("hello world");
    test.keys("gg");

    // Search for something that doesn't exist
    test.keys("/xyz");
    test.press_enter();

    // Cursor should stay at current position
    assert_eq!(
        test.cursor(),
        (0, 0),
        "Cursor should not move when pattern not found"
    );

    // Press 'n' - should still not move
    test.keys("n");
    assert_eq!(test.cursor(), (0, 0), "Cursor should still not move");
}

#[test]
fn test_search_multiple_on_same_line() {
    let mut test = EditorTest::new("the cat in the hat sat on the mat");
    test.keys("0");

    // Search for "the"
    test.keys("/the");
    test.press_enter();
    // nvim --clean: the match under the cursor (column 0) is skipped
    assert_eq!(test.cursor(), (0, 11), "Should find 'the' at column 11");

    // Press 'n' repeatedly to cycle through all "the"s
    test.keys("n");
    assert_eq!(test.cursor(), (0, 26), "Should find 'the' at column 26");

    test.keys("n");
    assert_eq!(
        test.cursor(),
        (0, 0),
        "Should wrap to first 'the' at column 0"
    );
}
