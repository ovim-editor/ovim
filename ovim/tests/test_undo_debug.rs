mod helpers;
use crossterm::event::{KeyCode, KeyModifiers};
use helpers::EditorTest;

#[test]
fn test_undo_debug() {
    let mut test = EditorTest::new("value: 10");

    println!(
        "Initial: {:?}, buffer: {}",
        test.cursor(),
        test.buffer_content()
    );

    test.keys("w");
    println!(
        "After w: {:?}, buffer: {}",
        test.cursor(),
        test.buffer_content()
    );

    test.press_with(KeyCode::Char('a'), KeyModifiers::CONTROL);
    println!(
        "After Ctrl-A: {:?}, buffer: {}",
        test.cursor(),
        test.buffer_content()
    );

    test.press('u');
    println!(
        "After undo: {:?}, buffer: {}",
        test.cursor(),
        test.buffer_content()
    );

    assert_eq!(test.buffer_content(), "value: 10\n");
    println!("Buffer is correct!");

    // After undo, cursor returns to position before the operation (where w left us)
    test.assert_cursor(0, 5);
}
