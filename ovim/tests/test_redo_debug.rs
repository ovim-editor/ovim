mod helpers;
use helpers::EditorTest;
use ovim_core::{KeyCode, Modifiers};

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

    test.press_with(KeyCode::Char('a'), Modifiers::CONTROL);
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

    test.press_with(KeyCode::Char('r'), Modifiers::CONTROL);
    println!(
        "After redo: {:?}, buffer: {}",
        test.cursor(),
        test.buffer_content()
    );
}
