use ovim_core::{KeyCode, KeyEvent, Modifiers};
use ovim::editor::{Editor, InputHandler};

/// Helper function to create a KeyEvent
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, Modifiers::NONE)
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
    // with_content adds trailing newline, so "line 1\nline 2\nline 3" becomes
    // "line 1\nline 2\nline 3\n" which is 3 lines in Vim-style counting
    // (Ropey counts 4 including phantom empty line, but line_count() adjusts)
    let mut editor = Editor::with_content("line 1\nline 2\nline 3");

    // Press 'o' on line 0
    press_char(&mut editor, 'o');

    // Should be in insert mode
    assert_eq!(editor.mode(), ovim::mode::Mode::Insert);

    // Should have 4 lines now (3 original + 1 new)
    assert_eq!(editor.buffer().line_count(), 4);

    // Cursor should be on line 1 (the new line)
    assert_eq!(editor.buffer().cursor().line(), 1);
    assert_eq!(editor.buffer().cursor().col(), 0);

    // Lines should be correct
    assert_eq!(editor.buffer().line(0).unwrap(), "line 1\n");
    assert_eq!(editor.buffer().line(1).unwrap(), "\n");
    assert_eq!(editor.buffer().line(2).unwrap(), "line 2\n");
    assert_eq!(editor.buffer().line(3).unwrap(), "line 3\n");
}

#[test]
fn test_o_last_line_no_newline() {
    // Test: 'o' on the last line
    // with_content adds trailing newline, so "line 1\nline 2" becomes
    // "line 1\nline 2\n" which is 2 lines in Vim-style counting
    let mut editor = Editor::with_content("line 1\nline 2");

    // Move to line 1 (the last actual line)
    press_char(&mut editor, 'j');
    assert_eq!(editor.buffer().cursor().line(), 1);

    // Press 'o'
    press_char(&mut editor, 'o');

    // Should be in insert mode
    assert_eq!(editor.mode(), ovim::mode::Mode::Insert);

    // Should have 3 lines (2 original + 1 new)
    assert_eq!(editor.buffer().line_count(), 3);

    // Cursor should be on line 2 (the new line)
    assert_eq!(editor.buffer().cursor().line(), 2);
    assert_eq!(editor.buffer().cursor().col(), 0);

    // Verify content
    assert_eq!(editor.buffer().line(0).unwrap(), "line 1\n");
    assert_eq!(editor.buffer().line(1).unwrap(), "line 2\n");
    assert_eq!(editor.buffer().line(2).unwrap(), "\n");
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
    // with_content adds trailing newline, so "line 1\nline 2" becomes
    // "line 1\nline 2\n" which is 2 lines in Vim-style counting
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

    // Should have 3 lines (2 original + 1 new)
    assert_eq!(editor.buffer().line_count(), 3);
    assert_eq!(editor.buffer().line(0).unwrap(), "line 1\n");
    assert_eq!(editor.buffer().line(1).unwrap(), "new\n");
    assert_eq!(editor.buffer().line(2).unwrap(), "line 2\n");
}

#[test]
fn test_o_and_undo() {
    // Test: 'o' followed by typing and undo
    // with_content adds a trailing newline, so "line 1\nline 2\nline 3" becomes
    // "line 1\nline 2\nline 3\n" which is 3 lines in Vim-style counting
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

    // Should have 4 lines (3 original + 1 new)
    assert_eq!(editor.buffer().line_count(), 4);
    assert_eq!(editor.buffer().line(1).unwrap(), "test\n");

    // Undo
    press_char(&mut editor, 'u');

    // Should be back to original 3 lines
    assert_eq!(editor.buffer().line_count(), 3);
    assert_eq!(editor.buffer().line(0).unwrap(), "line 1\n");
    assert_eq!(editor.buffer().line(1).unwrap(), "line 2\n");
    assert_eq!(editor.buffer().line(2).unwrap(), "line 3\n");
}

#[test]
fn test_o_on_empty_file() {
    // Test: 'o' on an empty file
    // Empty content ("") becomes "\n" (1 line with just newline in Vim-style counting)
    // After 'o', we add a new line, making it 2 lines
    let mut editor = Editor::with_content("");

    // Press 'o'
    press_char(&mut editor, 'o');

    // Should be in insert mode
    assert_eq!(editor.mode(), ovim::mode::Mode::Insert);

    // Should have 2 lines (1 original empty line + 1 new)
    assert_eq!(editor.buffer().line_count(), 2);

    // Cursor should be on line 1 (the new line)
    assert_eq!(editor.buffer().cursor().line(), 1);
}

#[test]
fn test_o_single_line_no_newline() {
    // Test: 'o' on a single line
    // with_content("hello") -> "hello\n" which is 1 line in Vim-style counting
    let mut editor = Editor::with_content("hello");

    // Press 'o'
    press_char(&mut editor, 'o');

    // Should be in insert mode
    assert_eq!(editor.mode(), ovim::mode::Mode::Insert);

    // Should have 2 lines (1 original + 1 new)
    assert_eq!(editor.buffer().line_count(), 2);

    // Cursor should be on line 1 (the new line)
    assert_eq!(editor.buffer().cursor().line(), 1);
    assert_eq!(editor.buffer().cursor().col(), 0);

    // Line content
    assert_eq!(editor.buffer().line(0).unwrap(), "hello\n");
    assert_eq!(editor.buffer().line(1).unwrap(), "\n");
}

#[test]
fn test_o_multiple_times() {
    // Test: pressing 'o' multiple times
    // with_content("start") -> "start\n" which is 1 line in Vim-style counting
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

    // Should have 3 lines (1 original + 2 new)
    assert_eq!(editor.buffer().line_count(), 3);
    assert_eq!(editor.buffer().line(0).unwrap(), "start\n");
    assert_eq!(editor.buffer().line(1).unwrap(), "l1\n");
    assert_eq!(editor.buffer().line(2).unwrap(), "l2\n");
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

// ============================================================================
// Redo cursor validation after o
// ============================================================================

#[test]
fn test_o_undo_redo_cursor_valid() {
    // Regression: redo of 'o' could place cursor past end of line because
    // cursor_after was captured in insert mode (one past last char).
    let mut editor = Editor::with_content("hello\nworld");

    // o, type text, Esc
    press_char(&mut editor, 'o');
    press_char(&mut editor, 'a');
    press_char(&mut editor, 'b');
    press_char(&mut editor, 'c');
    press(&mut editor, KeyCode::Esc);

    // Undo
    press_char(&mut editor, 'u');
    assert_eq!(editor.buffer().line_count(), 2);

    // Redo (Ctrl-R)
    InputHandler::handle_key_event(
        &mut editor,
        KeyEvent::new(KeyCode::Char('r'), Modifiers::CONTROL),
    )
    .unwrap();

    // After redo, cursor should be within valid line bounds (normal mode)
    let cursor_line = editor.buffer().cursor().line();
    let cursor_col = editor.buffer().cursor().col();
    let line = editor.buffer().line(cursor_line).unwrap();
    let line_len = line.trim_end_matches('\n').chars().count();
    assert!(
        cursor_col < line_len || line_len == 0,
        "Cursor col {} should be < line_len {} after redo (line: {:?})",
        cursor_col,
        line_len,
        line
    );
}

// ============================================================================
// Count not leaked through mode transitions
// ============================================================================

#[test]
fn test_count_not_leaked_through_o() {
    // Regression: typing '5o' would leak count=5 to next normal mode command.
    let mut editor = Editor::with_content("line 1\nline 2\nline 3\nline 4\nline 5");

    // Type 5, then o — count should be consumed/cleared
    press_char(&mut editor, '5');
    press_char(&mut editor, 'o');
    press(&mut editor, KeyCode::Esc);

    // Now type 'j' — should move 1 line, not 5
    let before_line = editor.buffer().cursor().line();
    press_char(&mut editor, 'j');
    let after_line = editor.buffer().cursor().line();

    assert_eq!(
        after_line - before_line,
        1,
        "j after o should move 1 line, not 5 (count leaked)"
    );
}
