use ovim::mode::Mode;

mod helpers;
use helpers::EditorTest;

#[test]
fn test_find_forward_repeat() {
    let mut test = EditorTest::new("hello there hello world hello again");
    test.keys("0"); // Start of line
    // Position 0='h', 7='h' in 'there', 12='h' in second 'hello', 24='h' in third 'hello'

    // Find 'h' from position 0 - should find 'h' in 'there' at position 7
    test.keys("fh");
    assert_eq!(test.cursor(), (0, 7), "'h' in 'there' should be at column 7");

    // Repeat find - should find 'h' in second 'hello'
    test.keys(";");
    assert_eq!(test.cursor(), (0, 12), "'h' in second 'hello' should be at column 12");

    // Repeat again - should find 'h' in third 'hello'
    test.keys(";");
    assert_eq!(test.cursor(), (0, 24), "'h' in third 'hello' should be at column 24");
}

#[test]
fn test_find_backward_repeat() {
    let mut test = EditorTest::new("hello there hello world hello again");
    test.keys("$"); // End of line
    // Position 0='h', 7='h' in 'there', 12='h' in second 'hello', 24='h' in third 'hello'

    // Find 'h' backward from end - should find 'h' at position 24
    test.keys("Fh");
    assert_eq!(test.cursor(), (0, 24), "'h' at position 24");

    // Repeat backward - should find 'h' at position 12
    test.keys(";");
    assert_eq!(test.cursor(), (0, 12), "'h' at position 12");

    // Repeat backward again - should find 'h' at position 7
    test.keys(";");
    assert_eq!(test.cursor(), (0, 7), "'h' at position 7");

    // One more - should find 'h' at position 0
    test.keys(";");
    assert_eq!(test.cursor(), (0, 0), "'h' at position 0");
}

#[test]
fn test_find_repeat_opposite_direction() {
    let mut test = EditorTest::new("hello there hello world hello again");
    test.keys("0"); // Start of line
    // Position 0='h', 7='h' in 'there', 12='h' in second 'hello', 24='h' in third 'hello'

    // Find 'h' forward twice to get to position 12
    test.keys("fh"); // Should go to position 7
    test.keys(";");  // Should go to position 12
    assert_eq!(test.cursor(), (0, 12), "Should be at 'h' at position 12");

    // , reverses direction (backward), should find 'h' at position 7
    test.keys(",");
    assert_eq!(test.cursor(), (0, 7), "Should go back to 'h' at position 7");

    // , continues backward, should find 'h' at position 0
    test.keys(",");
    assert_eq!(test.cursor(), (0, 0), "Should continue backward to 'h' at position 0");

    // , continues backward, should not find anything (stay at 0)
    test.keys(",");
    assert_eq!(test.cursor(), (0, 0), "Should stay at position 0 (no more 'h' backward)");
}

#[test]
fn test_till_forward_repeat() {
    let mut test = EditorTest::new("abcde fghij klmno");
    test.keys("0"); // Start of line
    // Positions: a=0, b=1, c=2, d=3, e=4, space=5, f=6, g=7, h=8, i=9, j=10, space=11, k=12, l=13, m=14, n=15, o=16

    // Till 'e' from position 0 - should stop at position 3 (one before 'e' at 4)
    test.keys("te");
    assert_eq!(test.cursor(), (0, 3), "Should be one before 'e' at column 3");

    // Repeat till 'o' - find next 'o' at position 16, stop at 15
    test.keys("to");
    assert_eq!(test.cursor(), (0, 15), "Should be one before 'o' at column 15");

    // Repeat - should not find another 'o'
    test.keys(";");
    assert_eq!(test.cursor(), (0, 15), "Should stay at column 15 (no more 'o')");
}

#[test]
fn test_till_backward_repeat() {
    let mut test = EditorTest::new("abcde fghij klmno");
    test.keys("$"); // End of line
    // Positions: a=0, b=1, c=2, d=3, e=4, space=5, f=6, g=7, h=8, i=9, j=10, space=11, k=12, l=13, m=14, n=15, o=16

    // Till 'e' backward from end - 'e' is at position 4, stop at 5 (one after)
    test.keys("Te");
    assert_eq!(test.cursor(), (0, 5), "Should be one after 'e' at column 5");

    // Till 'a' backward - 'a' is at position 0, stop at 1 (one after)
    test.keys("Ta");
    assert_eq!(test.cursor(), (0, 1), "Should be one after 'a' at column 1");

    // Repeat - should not find another 'a'
    test.keys(";");
    assert_eq!(test.cursor(), (0, 1), "Should stay at column 1 (no more 'a')");
}

#[test]
fn test_find_no_match() {
    let mut test = EditorTest::new("hello world");
    test.keys("0");

    // Find 'z' which doesn't exist
    test.keys("fz");
    assert_eq!(test.cursor(), (0, 0), "Cursor should not move when character not found");

    // Repeat should also do nothing (no last find to repeat)
    test.keys(";");
    assert_eq!(test.cursor(), (0, 0), "Repeat should also not move");
}

#[test]
fn test_find_with_count() {
    let mut test = EditorTest::new("hello there hello world hello again");
    test.keys("0");

    // Find second 'h' with count
    test.keys("2fh");
    assert_eq!(test.cursor(), (0, 12), "2fh should find second 'h'");

    // Repeat should find next 'h'
    test.keys(";");
    assert_eq!(test.cursor(), (0, 24), "Repeat should find third 'h'");
}

#[test]
fn test_find_at_current_position() {
    let mut test = EditorTest::new("hello world");
    test.keys("0");

    // We're at 'h' position 0
    assert_eq!(test.cursor(), (0, 0));

    // Find 'h' should stay at current position (we're already on it)
    test.keys("fh");
    assert_eq!(test.cursor(), (0, 0), "Should stay on current 'h'");

    // Repeat should not find another 'h' (there isn't one)
    test.keys(";");
    assert_eq!(test.cursor(), (0, 0), "Should stay put as no more 'h' found");
}
