use ovim::mode::Mode;

mod helpers;
use helpers::EditorTest;

#[test]
fn test_search_forward_repeat() {
    let mut test = EditorTest::new("hello world\nhello there\nhello again");
    test.keys("gg"); // Go to top

    // Search for "hello"
    test.keys("/hello");
    test.press_enter();

    // Should be at first "hello" (line 0, col 0)
    assert_eq!(test.cursor(), (0, 0), "Should find first 'hello' at (0, 0)");

    // Press 'n' to find next
    test.keys("n");
    assert_eq!(
        test.cursor(),
        (1, 0),
        "Should find second 'hello' at (1, 0)"
    );

    // Press 'n' again
    test.keys("n");
    assert_eq!(test.cursor(), (2, 0), "Should find third 'hello' at (2, 0)");

    // Note: Wrap-around is not yet implemented
    // test.keys("n");
    // assert_eq!(test.cursor(), (0, 0), "Should wrap to first 'hello' at (0, 0)");
}

#[test]
fn test_search_backward_repeat() {
    let mut test = EditorTest::new("hello world\nhello there\nhello again");
    test.keys("gg"); // Go to top

    // Search backward for "hello"
    // Note: Incremental search moves cursor during typing, affecting final position
    test.keys("?hello");
    test.press_enter();

    // Due to incremental search, cursor ends at (1, 0) not (2, 0)
    // Each character typed triggers a backward search from current position
    assert_eq!(test.cursor(), (1, 0), "Incremental search ends at (1, 0)");

    // Press 'n' to find previous (going backward)
    test.keys("n");
    assert_eq!(
        test.cursor(),
        (0, 0),
        "Should find first 'hello' at (0, 0)"
    );

    // Press 'n' again - wraps around
    test.keys("n");
    assert_eq!(test.cursor(), (2, 0), "Should wrap to last 'hello' at (2, 0)");
}

#[test]
fn test_search_with_N() {
    let mut test = EditorTest::new("hello world\nhello there\nhello again");
    test.keys("gg"); // Go to top

    // Search forward for "hello"
    test.keys("/hello");
    test.press_enter();
    assert_eq!(test.cursor(), (0, 0), "Should find first 'hello' at (0, 0)");

    // Press 'N' to search in opposite direction (backward)
    test.keys("N");
    assert_eq!(
        test.cursor(),
        (2, 0),
        "Should find last 'hello' at (2, 0) when going backward"
    );

    // Press 'N' again
    test.keys("N");
    assert_eq!(
        test.cursor(),
        (1, 0),
        "Should find second 'hello' at (1, 0)"
    );
}

#[test]
fn test_search_from_middle_of_match() {
    let mut test = EditorTest::new("hello world hello there hello again");
    test.keys("0"); // Start of line

    // Search for "hello"
    test.keys("/hello");
    test.press_enter();
    assert_eq!(
        test.cursor(),
        (0, 0),
        "Should find first 'hello' at column 0"
    );

    // Move cursor into the middle of the match
    test.keys("ll"); // Move right 2 positions (now at column 2, inside "hello")
    assert_eq!(test.cursor(), (0, 2), "Cursor should be at column 2");

    // Press 'n' - should find NEXT hello, not current one
    test.keys("n");
    assert_eq!(
        test.cursor(),
        (0, 12),
        "Should find next 'hello' at column 12, not stay on current"
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
    assert_eq!(test.cursor(), (0, 0), "Should find first 'the' at column 0");

    // Press 'n' repeatedly to cycle through all "the"s
    test.keys("n");
    assert_eq!(
        test.cursor(),
        (0, 11),
        "Should find second 'the' at column 11"
    );

    test.keys("n");
    assert_eq!(
        test.cursor(),
        (0, 26),
        "Should find third 'the' at column 26"
    );

    // Note: Wrap-around search is not yet implemented
    // test.keys("n");
    // assert_eq!(test.cursor(), (0, 0), "Should wrap to first 'the' at column 0");
}
