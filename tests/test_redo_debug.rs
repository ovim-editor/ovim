mod helpers;
use crossterm::event::{KeyCode, KeyModifiers};
use helpers::EditorTest;

#[test]
fn test_redo_debug() {
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

    test.press_with(KeyCode::Char('r'), KeyModifiers::CONTROL);
    println!(
        "After redo: {:?}, buffer: {}",
        test.cursor(),
        test.buffer_content()
    );
}
