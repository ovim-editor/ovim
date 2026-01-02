use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ovim::editor::{Editor, InputHandler};

/// Helper function to create a KeyEvent
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

/// Helper function to handle a key press
fn press(editor: &mut Editor, code: KeyCode) {
    InputHandler::handle_key_event(editor, key(code)).unwrap();
}

/// Helper to press a character key
fn press_char(editor: &mut Editor, c: char) {
    press(editor, KeyCode::Char(c));
}

/// Helper to get full buffer content as string
fn buffer_content(editor: &Editor) -> String {
    let mut content = String::new();
    for i in 0..editor.buffer().line_count() {
        if let Some(line) = editor.buffer().line(i) {
            content.push_str(&line);
        }
    }
    content
}

#[test]
fn test_o_middle_of_file() {
    // Test: 'o' on a line in the middle of file
    // Note: with_content adds trailing newline, so "line 1\nline 2\nline 3" becomes
    // "line 1\nline 2\nline 3\n" which is 4 lines (last line empty)
    let mut editor = Editor::with_content("line 1\nline 2\nline 3");

    // Press 'o' on line 0
    press_char(&mut editor, 'o');

    // Should be in insert mode
    assert_eq!(editor.mode(), ovim::mode::Mode::Insert);

    // Should have 5 lines now (4 original + 1 new)
    assert_eq!(editor.buffer().line_count(), 5);

    // Cursor should be on line 1 (the new line)
    assert_eq!(editor.buffer().cursor().line(), 1);
    assert_eq!(editor.buffer().cursor().col(), 0);

    // Lines should be correct
    assert_eq!(editor.buffer().line(0).unwrap(), "line 1\n");
    assert_eq!(editor.buffer().line(1).unwrap(), "\n");
    assert_eq!(editor.buffer().line(2).unwrap(), "line 2\n");
    assert_eq!(editor.buffer().line(3).unwrap(), "line 3\n");
    assert_eq!(editor.buffer().line(4).unwrap(), ""); // trailing empty line
}

#[test]
fn test_o_last_line_no_newline() {
    // Test: 'o' on the last line
    // Note: with_content adds trailing newline, so "line 1\nline 2" becomes
    // "line 1\nline 2\n" which is 3 lines (last line empty)
    let mut editor = Editor::with_content("line 1\nline 2");

    // Move to line 1 (line 2 is the empty trailing line due to normalization)
    press_char(&mut editor, 'j');
    assert_eq!(editor.buffer().cursor().line(), 1);

    // Press 'o'
    press_char(&mut editor, 'o');

    // Should be in insert mode
    assert_eq!(editor.mode(), ovim::mode::Mode::Insert);

    // Should have 4 lines (3 original + 1 new)
    assert_eq!(editor.buffer().line_count(), 4);

    // Cursor should be on line 2 (the new line)
    assert_eq!(editor.buffer().cursor().line(), 2);
    assert_eq!(editor.buffer().cursor().col(), 0);

    // Verify content
    assert_eq!(editor.buffer().line(0).unwrap(), "line 1\n");
    assert_eq!(editor.buffer().line(1).unwrap(), "line 2\n");
    assert_eq!(editor.buffer().line(2).unwrap(), "\n");
    assert_eq!(editor.buffer().line(3).unwrap(), ""); // trailing empty line
}

#[test]
fn test_o_with_indentation() {
    // Test: 'o' on an indented line copies indentation
    let mut editor = Editor::with_content("line 1\n    indented line\nline 3");

    // Move to indented line
    press_char(&mut editor, 'j');
    assert_eq!(editor.buffer().cursor().line(), 1);

    // Press 'o'
    press_char(&mut editor, 'o');

    // Should be in insert mode
    assert_eq!(editor.mode(), ovim::mode::Mode::Insert);

    // Cursor should be at column 4 (after the indentation)
    assert_eq!(editor.buffer().cursor().line(), 2);
    assert_eq!(editor.buffer().cursor().col(), 4);

    // New line should have indentation
    assert_eq!(editor.buffer().line(2).unwrap(), "    \n");
}

#[test]
fn test_o_type_text() {
    // Test: 'o' followed by typing text
    // Note: with_content adds trailing newline, so "line 1\nline 2" becomes
    // "line 1\nline 2\n" which is 3 lines
    let mut editor = Editor::with_content("line 1\nline 2");

    // Press 'o'
    press_char(&mut editor, 'o');

    // Type some text
    press_char(&mut editor, 'n');
    press_char(&mut editor, 'e');
    press_char(&mut editor, 'w');

    // Exit insert mode
    press(&mut editor, KeyCode::Esc);

    // Check content
    let content = buffer_content(&editor);
    assert!(
        content.contains("new"),
        "Content should contain 'new': {}",
        content
    );

    // Should have 4 lines (3 original + 1 new)
    assert_eq!(editor.buffer().line_count(), 4);
    assert_eq!(editor.buffer().line(0).unwrap(), "line 1\n");
    assert_eq!(editor.buffer().line(1).unwrap(), "new\n");
    assert_eq!(editor.buffer().line(2).unwrap(), "line 2\n");
    assert_eq!(editor.buffer().line(3).unwrap(), ""); // trailing empty line
}

#[test]
fn test_o_and_undo() {
    // Test: 'o' followed by typing and undo
    // Note: with_content adds a trailing newline, so we get 4 lines initially
    let mut editor = Editor::with_content("line 1\nline 2\nline 3");

    // Press 'o'
    press_char(&mut editor, 'o');

    // Type text
    press_char(&mut editor, 't');
    press_char(&mut editor, 'e');
    press_char(&mut editor, 's');
    press_char(&mut editor, 't');

    // Exit insert mode
    press(&mut editor, KeyCode::Esc);

    // Should have 5 lines (4 original + 1 new)
    assert_eq!(editor.buffer().line_count(), 5);
    assert_eq!(editor.buffer().line(1).unwrap(), "test\n");

    // Undo
    press_char(&mut editor, 'u');

    // Should be back to original 4 lines (with_content's trailing line)
    assert_eq!(editor.buffer().line_count(), 4);
    assert_eq!(editor.buffer().line(0).unwrap(), "line 1\n");
    assert_eq!(editor.buffer().line(1).unwrap(), "line 2\n");
    assert_eq!(editor.buffer().line(2).unwrap(), "line 3\n");
}

#[test]
fn test_o_on_empty_file() {
    // Test: 'o' on an empty file
    // Note: Empty content still normalizes (empty rope has 1 line, 'o' adds 2 more lines)
    let mut editor = Editor::with_content("");

    // Press 'o'
    press_char(&mut editor, 'o');

    // Should be in insert mode
    assert_eq!(editor.mode(), ovim::mode::Mode::Insert);

    // Should have 3 lines (empty string creates some initial state, 'o' adds new lines)
    assert_eq!(editor.buffer().line_count(), 3);

    // Cursor should be on line 1 (the new line)
    assert_eq!(editor.buffer().cursor().line(), 1);
}

#[test]
fn test_o_single_line_no_newline() {
    // Test: 'o' on a single line
    // Note: with_content("hello") → "hello\n" (2 lines: "hello\n" and "")
    let mut editor = Editor::with_content("hello");

    // Press 'o'
    press_char(&mut editor, 'o');

    // Should be in insert mode
    assert_eq!(editor.mode(), ovim::mode::Mode::Insert);

    // Should have 3 lines (2 original + 1 new)
    assert_eq!(editor.buffer().line_count(), 3);

    // Cursor should be on line 1 (the new line)
    assert_eq!(editor.buffer().cursor().line(), 1);
    assert_eq!(editor.buffer().cursor().col(), 0);

    // Line content
    assert_eq!(editor.buffer().line(0).unwrap(), "hello\n");
    assert_eq!(editor.buffer().line(1).unwrap(), "\n");
    assert_eq!(editor.buffer().line(2).unwrap(), ""); // trailing empty line
}

#[test]
fn test_o_multiple_times() {
    // Test: pressing 'o' multiple times
    // Note: with_content("start") → "start\n" (2 lines: "start\n" and "")
    let mut editor = Editor::with_content("start");

    // Press 'o' and add text
    press_char(&mut editor, 'o');
    press_char(&mut editor, 'l');
    press_char(&mut editor, '1');
    press(&mut editor, KeyCode::Esc);

    // Press 'o' again and add more text
    press_char(&mut editor, 'o');
    press_char(&mut editor, 'l');
    press_char(&mut editor, '2');
    press(&mut editor, KeyCode::Esc);

    // Should have 4 lines (2 original + 2 new)
    assert_eq!(editor.buffer().line_count(), 4);
    assert_eq!(editor.buffer().line(0).unwrap(), "start\n");
    assert_eq!(editor.buffer().line(1).unwrap(), "l1\n");
    assert_eq!(editor.buffer().line(2).unwrap(), "l2\n");
    assert_eq!(editor.buffer().line(3).unwrap(), ""); // trailing empty line
}

#[test]
fn test_o_preserves_tab_indentation() {
    // Test: 'o' preserves tab indentation
    let mut editor = Editor::with_content("line 1\n\ttabbed\nline 3");

    // Move to tabbed line
    press_char(&mut editor, 'j');

    // Press 'o'
    press_char(&mut editor, 'o');

    // New line should have tab indentation
    assert_eq!(editor.buffer().line(2).unwrap(), "\t\n");
    assert_eq!(editor.buffer().cursor().col(), 1); // After the tab
}

#[test]
fn test_o_mixed_indentation() {
    // Test: 'o' with mixed spaces and tabs (though not recommended in practice)
    let mut editor = Editor::with_content("start\n  \tmixed\nend");

    // Move to mixed indentation line
    press_char(&mut editor, 'j');

    // Press 'o'
    press_char(&mut editor, 'o');

    // New line should have the mixed indentation
    let new_line = editor.buffer().line(2).unwrap();
    assert!(
        new_line.starts_with("  \t"),
        "Line should start with '  \\t': {:?}",
        new_line
    );
}
