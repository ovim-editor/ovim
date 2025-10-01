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
fn test_delete_last_line_with_dd() {
    // Test deleting the last line moves cursor up
    let mut editor = Editor::with_content("line 1\nline 2\nline 3");

    // Move to last line
    press(&mut editor, KeyCode::Char('G'));

    // Delete last line
    press(&mut editor, KeyCode::Char('d'));
    press(&mut editor, KeyCode::Char('d'));

    // Should have 2 lines now
    assert_eq!(editor.buffer().line_count(), 2);

    // Cursor should be on line 1 (second line, 0-indexed)
    assert_eq!(editor.buffer().cursor().line(), 1);
}

#[test]
fn test_delete_multiple_lines_at_end() {
    // Test deleting multiple lines at end of file
    let mut editor = Editor::with_content("line 1\nline 2\nline 3\nline 4");

    // Move to line 3 (0-indexed: line 2)
    press(&mut editor, KeyCode::Char('j'));
    press(&mut editor, KeyCode::Char('j'));

    // Delete 2 lines (should delete line 3 and 4)
    press(&mut editor, KeyCode::Char('2'));
    press(&mut editor, KeyCode::Char('d'));
    press(&mut editor, KeyCode::Char('d'));

    // Should have 2 lines now
    assert_eq!(editor.buffer().line_count(), 2);

    // Cursor should be on line 1 (0-indexed)
    assert_eq!(editor.buffer().cursor().line(), 1);
}

#[test]
fn test_x_at_end_of_line() {
    // Test x at end of line clamps cursor
    let mut editor = Editor::with_content("hello");

    // Move to last character
    press(&mut editor, KeyCode::Char('$'));

    // Delete last character
    press(&mut editor, KeyCode::Char('x'));

    // Buffer should be "hell"
    assert_eq!(editor.buffer().line(0).unwrap(), "hell");

    // Cursor should be at col 3 (last char)
    assert_eq!(editor.buffer().cursor().col(), 3);
}

#[test]
fn test_x_delete_all_chars_on_line() {
    // Test deleting all characters on a line
    let mut editor = Editor::with_content("abc");

    // Delete all 3 characters
    press(&mut editor, KeyCode::Char('3'));
    press(&mut editor, KeyCode::Char('x'));

    // Buffer should be empty
    assert_eq!(editor.buffer().line(0).unwrap(), "");

    // Cursor should be at col 0
    assert_eq!(editor.buffer().cursor().col(), 0);
}

#[test]
fn test_dw_at_end_of_line() {
    // Test dw at end of line doesn't cross newline
    let mut editor = Editor::with_content("hello world\nline 2");

    // Move to "world"
    for _ in 0..6 {
        press(&mut editor, KeyCode::Char('l'));
    }

    // Delete word
    press(&mut editor, KeyCode::Char('d'));
    press(&mut editor, KeyCode::Char('w'));

    // Should have deleted "world" but not crossed newline
    let line = editor.buffer().line(0).unwrap();
    assert!(line.starts_with("hello "));
    assert!(!line.contains("world"));

    // Line 2 should still exist
    assert_eq!(editor.buffer().line_count(), 2);
}

#[test]
fn test_empty_file() {
    // Test operations on empty file don't crash
    let mut editor = Editor::with_content("");

    // Try various operations
    press(&mut editor, KeyCode::Char('d'));
    press(&mut editor, KeyCode::Char('d'));

    press(&mut editor, KeyCode::Char('x'));

    press(&mut editor, KeyCode::Char('d'));
    press(&mut editor, KeyCode::Char('w'));

    // Should still have empty buffer
    assert_eq!(editor.buffer().line_count(), 1);
    assert_eq!(editor.buffer().line(0).unwrap(), "");

    // Cursor should be at 0,0
    assert_eq!(editor.buffer().cursor().line(), 0);
    assert_eq!(editor.buffer().cursor().col(), 0);
}

#[test]
fn test_single_line_delete() {
    // Test deleting the only line in the buffer
    let mut editor = Editor::with_content("only line");

    // Delete the line
    press(&mut editor, KeyCode::Char('d'));
    press(&mut editor, KeyCode::Char('d'));

    // Should have empty buffer
    assert_eq!(editor.buffer().line_count(), 1);
    assert_eq!(editor.buffer().line(0).unwrap(), "");

    // Cursor should be at 0,0
    assert_eq!(editor.buffer().cursor().line(), 0);
    assert_eq!(editor.buffer().cursor().col(), 0);
}

#[test]
fn test_d_at_end_of_line() {
    // Test D (delete to end of line) when at last character
    let mut editor = Editor::with_content("hello world");

    // Move to end ($ puts cursor on last char, the 'd')
    press(&mut editor, KeyCode::Char('$'));

    // Delete to end of line (deletes the 'd')
    press(&mut editor, KeyCode::Char('D'));

    // Buffer should have "hello worl" (last char deleted)
    assert_eq!(editor.buffer().line(0).unwrap(), "hello worl");

    // Cursor should be clamped to the new last character
    assert_eq!(editor.buffer().cursor().col(), 9);
}

#[test]
fn test_diw_deletes_word() {
    // Test diw (delete inner word) deletes the word
    let mut editor = Editor::with_content("hello world");

    // Delete inner word at cursor (should delete "hello")
    press(&mut editor, KeyCode::Char('d'));
    press(&mut editor, KeyCode::Char('i'));
    press(&mut editor, KeyCode::Char('w'));

    // "hello" should be deleted, leaving " world" or something similar
    let line = editor.buffer().line(0).unwrap();
    assert!(!line.contains("hello"), "Expected 'hello' to be deleted, got: {}", line);

    // Cursor should be at start of deletion
    assert_eq!(editor.buffer().cursor().line(), 0);
}

#[test]
fn test_cw_at_end_of_line() {
    // Test cw (change word) at end of line
    let mut editor = Editor::with_content("hello world");

    // Move to "world"
    for _ in 0..6 {
        press(&mut editor, KeyCode::Char('l'));
    }

    // Change word
    press(&mut editor, KeyCode::Char('c'));
    press(&mut editor, KeyCode::Char('w'));

    // Should be in insert mode
    assert_eq!(editor.mode(), ovim::mode::Mode::Insert);

    // Buffer should have "hello " (word deleted)
    let line = editor.buffer().line(0).unwrap();
    assert!(line.starts_with("hello "));
    assert!(!line.contains("world"));
}
