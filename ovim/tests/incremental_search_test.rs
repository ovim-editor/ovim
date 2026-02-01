mod helpers;
use helpers::EditorTest;

#[test]
fn test_incremental_search() {
    let mut test = EditorTest::new("hello world\nfoo bar\nhello again\n");

    // Start search
    test.keys("/");

    // Type 'h' - should jump to first 'hello'
    test.keys("h");
    eprintln!("After /h: cursor {:?}", test.cursor());
    assert_eq!(test.cursor(), (0, 0)); // First 'hello' at line 0, col 0

    // Type 'e' - should still match 'hello'
    test.keys("e");
    eprintln!("After /he: cursor {:?}", test.cursor());
    assert_eq!(test.cursor(), (0, 0)); // Still first 'hello'

    // Type 'l' - should still match 'hello'
    test.keys("l");
    eprintln!("After /hel: cursor {:?}", test.cursor());
    assert_eq!(test.cursor(), (0, 0)); // Still first 'hello'

    // Press Enter to confirm
    test.keys("<Enter>");
    eprintln!("After Enter: cursor {:?}", test.cursor());
    assert_eq!(test.cursor(), (0, 0)); // Stays at match

    // Press n to find next
    test.keys("n");
    eprintln!("After n: cursor {:?}", test.cursor());
    assert_eq!(test.cursor(), (2, 0)); // Next 'hello' on line 2
}

#[test]
fn test_incremental_search_no_match() {
    let mut test = EditorTest::new("hello world\n");

    // Start search
    test.keys("/");

    // Type pattern with no match
    test.keys("xyz");

    // Cursor should stay at original position (0,0)
    eprintln!("After /xyz: cursor {:?}", test.cursor());
    assert_eq!(test.cursor(), (0, 0));

    // Esc should exit
    test.press_esc();
    assert_eq!(test.mode(), ovim::mode::Mode::Normal);
}

#[test]
fn test_incremental_search_backspace() {
    let mut test = EditorTest::new("hello world\nfoo bar\n");

    // Start at line 1
    test.keys("j");
    assert_eq!(test.cursor(), (1, 0));

    // Start search
    test.keys("/");

    // Type 'foo' - should jump to 'foo'
    test.keys("foo");
    eprintln!("After /foo: cursor {:?}", test.cursor());
    assert_eq!(test.cursor(), (1, 0)); // 'foo' at line 1, col 0

    // Backspace to 'fo'
    test.press_key(ovim_core::KeyCode::Backspace);
    eprintln!("After backspace to /fo: cursor {:?}", test.cursor());
    assert_eq!(test.cursor(), (1, 0)); // Still matches 'foo'

    // Backspace to 'f'
    test.press_key(ovim_core::KeyCode::Backspace);
    eprintln!("After backspace to /f: cursor {:?}", test.cursor());
    assert_eq!(test.cursor(), (1, 0)); // Still matches 'foo'

    // Press Enter
    test.keys("<Enter>");
    assert_eq!(test.cursor(), (1, 0));
}
