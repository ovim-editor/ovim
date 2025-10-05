use ovim::editor::{Editor, InputHandler};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Helper function to create a KeyEvent
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

/// Helper function to handle a key press
fn press(editor: &mut Editor, code: KeyCode) {
    InputHandler::handle_key_event(editor, key(code)).unwrap();
}

#[test]
fn test_goal_column_preserved_through_short_line() {
    // Test that cursor column is preserved when moving through a shorter line
    //
    // aaaaaaaaAaaa  <- Start at column 8 (marked A)
    // aaa           <- Move down (line too short, cursor at column 3)
    // aaaaaaaaBaa   <- Move down (should be at column 8, marked B)
    //         ^

    let mut editor = Editor::with_content("aaaaaaaaAaaa\naaa\naaaaaaaaBaa");

    // Move to column 8 on first line (the 'A')
    for _ in 0..8 {
        press(&mut editor, KeyCode::Char('l'));
    }

    assert_eq!(editor.buffer().cursor().line(), 0);
    assert_eq!(editor.buffer().cursor().col(), 8);

    // Move down to short line
    press(&mut editor, KeyCode::Char('j'));

    assert_eq!(editor.buffer().cursor().line(), 1);
    // Should be clamped to end of shorter line (column 2, last char of "aaa")
    assert_eq!(editor.buffer().cursor().col(), 2);

    // Move down to long line again
    press(&mut editor, KeyCode::Char('j'));

    assert_eq!(editor.buffer().cursor().line(), 2);
    // Should return to original goal column 8 (the 'B')
    assert_eq!(editor.buffer().cursor().col(), 8,
        "Cursor should return to goal column 8 when line is long enough");
}

#[test]
fn test_goal_column_preserved_through_multiple_short_lines() {
    // Test goal column preservation through multiple short lines
    //
    // aaaaaaaaaaa  <- Start at column 10
    // a            <- Short line
    // aa           <- Short line
    // aaa          <- Short line
    // aaaaaaaaaaaB <- Should return to column 10
    //           ^

    let mut editor = Editor::with_content("aaaaaaaaaaa\na\naa\naaa\naaaaaaaaaaaB");

    // Move to column 10 on first line
    for _ in 0..10 {
        press(&mut editor, KeyCode::Char('l'));
    }

    assert_eq!(editor.buffer().cursor().col(), 10);

    // Move down through short lines
    press(&mut editor, KeyCode::Char('j')); // Line 1: "a"
    assert_eq!(editor.buffer().cursor().line(), 1);
    assert_eq!(editor.buffer().cursor().col(), 0); // Clamped to col 0

    press(&mut editor, KeyCode::Char('j')); // Line 2: "aa"
    assert_eq!(editor.buffer().cursor().line(), 2);
    assert_eq!(editor.buffer().cursor().col(), 1); // Clamped to col 1

    press(&mut editor, KeyCode::Char('j')); // Line 3: "aaa"
    assert_eq!(editor.buffer().cursor().line(), 3);
    assert_eq!(editor.buffer().cursor().col(), 2); // Clamped to col 2

    press(&mut editor, KeyCode::Char('j')); // Line 4: "aaaaaaaaaaaB"
    assert_eq!(editor.buffer().cursor().line(), 4);
    assert_eq!(editor.buffer().cursor().col(), 10,
        "Cursor should return to goal column 10 after passing through multiple short lines");
}

#[test]
fn test_goal_column_up_movement() {
    // Test that goal column works in upward direction too
    //
    // aaaaaaaaBaa  <- Should end up at column 8 (marked B)
    // aaa          <- Short line
    // aaaaaaaaAaa  <- Start at column 8 (marked A)
    //         ^

    let mut editor = Editor::with_content("aaaaaaaaBaa\naaa\naaaaaaaaAaa");

    // Move to line 2 (third line)
    press(&mut editor, KeyCode::Char('j'));
    press(&mut editor, KeyCode::Char('j'));

    // Move to column 8
    for _ in 0..8 {
        press(&mut editor, KeyCode::Char('l'));
    }

    assert_eq!(editor.buffer().cursor().line(), 2);
    assert_eq!(editor.buffer().cursor().col(), 8);

    // Move up to short line
    press(&mut editor, KeyCode::Char('k'));

    assert_eq!(editor.buffer().cursor().line(), 1);
    assert_eq!(editor.buffer().cursor().col(), 2); // Clamped

    // Move up to long line
    press(&mut editor, KeyCode::Char('k'));

    assert_eq!(editor.buffer().cursor().line(), 0);
    assert_eq!(editor.buffer().cursor().col(), 8,
        "Cursor should return to goal column 8 when moving up");
}

#[test]
fn test_goal_column_reset_on_horizontal_movement() {
    // Test that goal column is reset when moving horizontally
    //
    // aaaaaaaaAaaa  <- Start at column 8
    // aaa           <- Move down, clamped
    //   ^           <- Move left to column 1
    // aaaaaaaaXaaa  <- Move down, should be at column 1 (marked X), not 8
    //  ^

    let mut editor = Editor::with_content("aaaaaaaaAaaa\naaa\naaaaaaaaXaaa");

    // Move to column 8 on first line
    for _ in 0..8 {
        press(&mut editor, KeyCode::Char('l'));
    }

    assert_eq!(editor.buffer().cursor().col(), 8);

    // Move down to short line
    press(&mut editor, KeyCode::Char('j'));
    assert_eq!(editor.buffer().cursor().line(), 1);
    assert_eq!(editor.buffer().cursor().col(), 2); // Clamped

    // Move left to column 1
    press(&mut editor, KeyCode::Char('h'));
    assert_eq!(editor.buffer().cursor().col(), 1);

    // Move down to long line - should use column 1, not original column 8
    press(&mut editor, KeyCode::Char('j'));
    assert_eq!(editor.buffer().cursor().line(), 2);
    assert_eq!(editor.buffer().cursor().col(), 1,
        "Cursor should be at column 1 (new goal column), not column 8 (old goal column)");
}

#[test]
fn test_goal_column_with_dollar_motion() {
    // Test that $ (end of line) sets goal column to "end of line"
    //
    // hello world$    <- Press $ to go to end
    // hi              <- Move down, should be at end ("i")
    // hello again$    <- Move down, should be at end ("n")

    let mut editor = Editor::with_content("hello world\nhi\nhello again");

    // Move to end of first line with $
    press(&mut editor, KeyCode::Char('$'));

    assert_eq!(editor.buffer().cursor().line(), 0);
    assert_eq!(editor.buffer().cursor().col(), 10); // Last char of "hello world"

    // Move down to shorter line
    press(&mut editor, KeyCode::Char('j'));

    assert_eq!(editor.buffer().cursor().line(), 1);
    assert_eq!(editor.buffer().cursor().col(), 1,
        "Should be at end of line (last char of 'hi')");

    // Move down to longer line
    press(&mut editor, KeyCode::Char('j'));

    assert_eq!(editor.buffer().cursor().line(), 2);
    assert_eq!(editor.buffer().cursor().col(), 10,
        "Should be at end of line (last char of 'hello again')");
}

#[test]
fn test_goal_column_with_zero_motion() {
    // Test that 0 (start of line) sets goal column to 0
    //
    //     indented line  <- Move to column 4
    // 0not indented      <- Press 0, then move down, should be at column 0
    // ^

    let mut editor = Editor::with_content("    indented line\nnot indented");

    // Move to column 4
    for _ in 0..4 {
        press(&mut editor, KeyCode::Char('l'));
    }

    assert_eq!(editor.buffer().cursor().col(), 4);

    // Press 0 to go to start of line
    press(&mut editor, KeyCode::Char('0'));

    assert_eq!(editor.buffer().cursor().col(), 0);

    // Move down
    press(&mut editor, KeyCode::Char('j'));

    assert_eq!(editor.buffer().cursor().line(), 1);
    assert_eq!(editor.buffer().cursor().col(), 0,
        "Should remain at column 0 after 0 motion");
}

#[test]
fn test_goal_column_empty_line() {
    // Test goal column behavior with empty lines
    //
    // aaaaaaaaX  <- Column 8 (the 'X')
    //            <- Empty line
    // aaaaaaaaBX <- Should be at column 8 (the 'X')
    //         ^

    let mut editor = Editor::with_content("aaaaaaaaX\n\naaaaaaaaBX");

    // Move to column 8
    for _ in 0..8 {
        press(&mut editor, KeyCode::Char('l'));
    }

    assert_eq!(editor.buffer().cursor().col(), 8);

    // Move down to empty line
    press(&mut editor, KeyCode::Char('j'));

    assert_eq!(editor.buffer().cursor().line(), 1);
    assert_eq!(editor.buffer().cursor().col(), 0); // Empty line

    // Move down to long line
    press(&mut editor, KeyCode::Char('j'));

    assert_eq!(editor.buffer().cursor().line(), 2);
    assert_eq!(editor.buffer().cursor().col(), 8,
        "Should return to goal column 8");
}
