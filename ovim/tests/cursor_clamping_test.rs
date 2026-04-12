use ovim::editor::{Editor, InputHandler};
use ovim::unicode::GraphemeCol;
use ovim_core::buffer::Buffer;
use ovim_core::{KeyCode, KeyEvent, Modifiers};

/// Helper function to create a KeyEvent
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, Modifiers::NONE)
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

    // Should have 2 lines now (with_content adds trailing newline, Vim semantics: 3 lines initially)
    assert_eq!(editor.buffer().line_count(), 2);

    // Cursor should be on line 1 (we deleted line 3, so cursor moves up to line 2, 0-indexed: line 1)
    // Actually dd at end moves cursor up, but with trailing newline behavior it ends at index 1
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

    // Should have 2 lines now (with_content adds trailing newline, Vim semantics: 4 lines initially)
    assert_eq!(editor.buffer().line_count(), 2);

    // Cursor should be on line 1 (we deleted lines 3-4, cursor moves up)
    assert_eq!(editor.buffer().cursor().line(), 1);
}

#[test]
fn test_x_at_end_of_line() {
    // Test x at end of line clamps cursor
    // Note: with_content adds trailing newline, so "hello" becomes "hello\n"
    let mut editor = Editor::with_content("hello");

    // Move to last character
    press(&mut editor, KeyCode::Char('$'));

    // Delete last character
    press(&mut editor, KeyCode::Char('x'));

    // Buffer should be "hell\n" (with trailing newline from normalization)
    assert_eq!(editor.buffer().line(0).unwrap(), "hell\n");

    // Cursor should be at col 3 (last char)
    assert_eq!(editor.buffer().cursor().col(), GraphemeCol(3));
}

#[test]
fn test_x_delete_all_chars_on_line() {
    // Test deleting all characters on a line
    // Note: with_content adds trailing newline, so "abc" becomes "abc\n"
    let mut editor = Editor::with_content("abc");

    // Delete all 3 characters
    press(&mut editor, KeyCode::Char('3'));
    press(&mut editor, KeyCode::Char('x'));

    // Buffer should just have newline (from normalization)
    assert_eq!(editor.buffer().line(0).unwrap(), "\n");

    // Cursor should be at col 0
    assert_eq!(editor.buffer().cursor().col(), GraphemeCol::ZERO);
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

    // Line 2 should still exist (Vim semantics: 2 lines)
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
    assert_eq!(editor.buffer().cursor().col(), GraphemeCol::ZERO);
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
    assert_eq!(editor.buffer().cursor().col(), GraphemeCol::ZERO);
}

#[test]
fn test_d_at_end_of_line() {
    // Test D (delete to end of line) when at last character
    let mut editor = Editor::with_content("hello world");

    // Move to end ($ puts cursor on last char, the 'd')
    press(&mut editor, KeyCode::Char('$'));

    // Delete to end of line (deletes the 'd')
    press(&mut editor, KeyCode::Char('D'));

    // Buffer should have "hello worl\n" (last char deleted, with trailing newline from normalization)
    assert_eq!(editor.buffer().line(0).unwrap(), "hello worl\n");

    // Cursor should be clamped to the new last character
    assert_eq!(editor.buffer().cursor().col(), GraphemeCol(9));
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
    assert!(
        !line.contains("hello"),
        "Expected 'hello' to be deleted, got: {}",
        line
    );

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

// ============================================================================
// Grapheme-aware cursor clamping
// ============================================================================

#[test]
fn test_clamp_cursor_col_ascii() {
    // Basic ASCII: 5 chars, cursor at col 10 should clamp to col 4
    let mut buf = Buffer::new_from_str("hello\n");
    buf.cursor_mut().set_col(GraphemeCol(10));
    buf.clamp_cursor_col();
    assert_eq!(buf.cursor().col(), GraphemeCol(4)); // last char index
}

#[test]
fn test_clamp_cursor_col_emoji() {
    // "a😀b" = 3 graphemes, but 😀 is 1 char (in Rust's char, it's actually
    // multiple code points for some emojis, but basic emoji is 1 code point).
    // With a ZWJ family emoji: "a👨‍👩‍👧‍👦b" = 3 graphemes, 7+ code points
    let mut buf = Buffer::new_from_str("a👨‍👩‍👧‍👦b\n");
    // grapheme count of "a👨‍👩‍👧‍👦b" = 3
    // If cursor is at col 5 (past grapheme count), should clamp to 2 (last grapheme)
    buf.cursor_mut().set_col(GraphemeCol(5));
    buf.clamp_cursor_col();
    assert_eq!(buf.cursor().col(), GraphemeCol(2)); // last grapheme index
}

#[test]
fn test_clamp_cursor_col_combining_chars() {
    // "e\u{0301}" (e + combining acute) = 1 grapheme, 2 code points
    // "ae\u{0301}b" = 3 graphemes
    let mut buf = Buffer::new_from_str("ae\u{0301}b\n");
    buf.cursor_mut().set_col(GraphemeCol(5));
    buf.clamp_cursor_col();
    assert_eq!(buf.cursor().col(), GraphemeCol(2)); // last grapheme index
}

#[test]
fn test_clamp_cursor_col_flag_emoji() {
    // "🇺🇸" = 1 grapheme (regional indicator pair), 2 code points
    // "a🇺🇸b" = 3 graphemes
    let mut buf = Buffer::new_from_str("a🇺🇸b\n");
    buf.cursor_mut().set_col(GraphemeCol(5));
    buf.clamp_cursor_col();
    assert_eq!(buf.cursor().col(), GraphemeCol(2));
}

#[test]
fn test_clamp_cursor_col_within_bounds_unchanged() {
    // Cursor within valid range should not be moved
    let mut buf = Buffer::new_from_str("a👨‍👩‍👧‍👦b\n");
    buf.cursor_mut().set_col(GraphemeCol(1)); // on the emoji grapheme
    buf.clamp_cursor_col();
    assert_eq!(buf.cursor().col(), GraphemeCol(1)); // unchanged
}

#[test]
fn test_clamp_cursor_col_empty_line() {
    let mut buf = Buffer::new_from_str("\n");
    buf.cursor_mut().set_col(GraphemeCol(3));
    buf.clamp_cursor_col();
    assert_eq!(buf.cursor().col(), GraphemeCol::ZERO);
}

#[test]
fn test_clamp_cursor_col_at_zero_stays() {
    // col 0 should never be clamped (the `col > 0` guard)
    let mut buf = Buffer::new_from_str("hello\n");
    buf.cursor_mut().set_col(GraphemeCol::ZERO);
    buf.clamp_cursor_col();
    assert_eq!(buf.cursor().col(), GraphemeCol::ZERO);
}

#[test]
fn test_dedent_clamps_cursor_with_emoji() {
    // After dedent, if cursor was past the new (shorter) line, it should clamp
    // using grapheme count, not char count.
    // Line: "    a👨‍👩‍👧‍👦b" (4 spaces + 3 graphemes = 7 graphemes total)
    // After dedent (remove 4 spaces): "a👨‍👩‍👧‍👦b" (3 graphemes)
    // Cursor at col 6 should clamp to 2 (last grapheme of "a👨‍👩‍👧‍👦b")
    let mut editor = Editor::with_content("    a👨‍👩‍👧‍👦b");

    // Move cursor to col 6
    for _ in 0..6 {
        press(&mut editor, KeyCode::Char('l'));
    }

    // Dedent
    press(&mut editor, KeyCode::Char('<'));
    press(&mut editor, KeyCode::Char('<'));

    // Line should now be "a👨‍👩‍👧‍👦b\n"
    let line = editor.buffer().line(0).unwrap();
    assert_eq!(
        line.trim_end_matches('\n'),
        "a👨\u{200d}👩\u{200d}👧\u{200d}👦b"
    );

    // Cursor should be clamped to valid grapheme position
    let col = editor.buffer().cursor().col().0;
    assert!(
        col <= 2,
        "cursor col {} should be <= 2 (last grapheme index)",
        col
    );
}
