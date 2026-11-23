mod helpers;
use helpers::EditorTest;

#[test]
fn test_incremental_search() {
    let mut test = EditorTest::new("hello world\nfoo bar\nhello again\n");

    // nvim --clean: `/hel<CR>` from (0,0) lands on (2,0) -- the match under the
    // cursor is skipped, and the preview of each typed character starts from the
    // same place.
    test.keys("/");

    // Type 'h' - the next 'h' after the cursor is the one on line 2
    test.keys("h");
    assert_eq!(test.cursor(), (2, 0));

    // Type 'e' - still the same match
    test.keys("e");
    assert_eq!(test.cursor(), (2, 0));

    // Type 'l' - still the same match
    test.keys("l");
    assert_eq!(test.cursor(), (2, 0));

    // Press Enter to confirm
    test.keys("<Enter>");
    assert_eq!(test.cursor(), (2, 0)); // Stays at match

    // nvim --clean: n wraps around to the first 'hel'
    test.keys("n");
    assert_eq!(test.cursor(), (0, 0));
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
