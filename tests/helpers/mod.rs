mod viewport_assertions;

pub use viewport_assertions::ViewportAssertion;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ovim::editor::{Editor, InputHandler};
use ovim::mode::Mode;

/// Test helper that provides a fluent API for driving editor operations
/// and capturing snapshots of editor state
pub struct EditorTest {
    pub editor: Editor,
}

impl EditorTest {
    /// Create a new test with initial buffer content
    pub fn new(content: &str) -> Self {
        Self {
            editor: Editor::with_content(content),
        }
    }

    /// Create a new test with empty buffer
    pub fn empty() -> Self {
        Self::new("")
    }

    /// Press a character key
    pub fn press(&mut self, c: char) -> &mut Self {
        let event = KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty());
        InputHandler::handle_key_event(&mut self.editor, event).unwrap();
        self
    }

    /// Press a key with modifiers
    pub fn press_with(&mut self, code: KeyCode, modifiers: KeyModifiers) -> &mut Self {
        let event = KeyEvent::new(code, modifiers);
        InputHandler::handle_key_event(&mut self.editor, event).unwrap();
        self
    }

    /// Press Escape key
    pub fn press_esc(&mut self) -> &mut Self {
        self.press_key(KeyCode::Esc)
    }

    /// Press Enter key
    pub fn press_enter(&mut self) -> &mut Self {
        self.press_key(KeyCode::Enter)
    }

    /// Press Backspace key
    pub fn press_backspace(&mut self) -> &mut Self {
        self.press_key(KeyCode::Backspace)
    }

    /// Press any KeyCode
    pub fn press_key(&mut self, key: KeyCode) -> &mut Self {
        let event = KeyEvent::new(key, KeyModifiers::empty());
        InputHandler::handle_key_event(&mut self.editor, event).unwrap();
        self
    }

    /// Type multiple characters in sequence
    pub fn type_text(&mut self, text: &str) -> &mut Self {
        for c in text.chars() {
            self.press(c);
        }
        self
    }

    /// Execute a sequence of vim keys (simple parser for common operations)
    /// Examples: "gg", "dd", "yy", "3j", "dw", "ciw", "<C-a>", "<C-x>"
    pub fn keys(&mut self, keys: &str) -> &mut Self {
        let mut chars = keys.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '<' {
                // Look ahead to see if this is a special key notation
                // Peek through the characters to find either '>' or determine it's not a special key
                let mut lookahead = vec![];
                let mut found_close = false;

                // Collect characters until we find '>' or run out
                while let Some(&next_c) = chars.peek() {
                    lookahead.push(next_c);
                    chars.next(); // consume it
                    if next_c == '>' {
                        found_close = true;
                        break;
                    }
                }

                // If we found a closing '>' and have content, it's a special key
                if found_close && lookahead.len() > 1 {
                    let special_key: String = lookahead.iter().take(lookahead.len() - 1).collect();
                    // Handle the special key
                    match special_key.as_str() {
                        "Esc" => self.press_esc(),
                        "Enter" => self.press_enter(),
                        "Tab" => self.press_key(KeyCode::Tab),
                        "BS" | "Backspace" => self.press_backspace(),
                        "Space" => self.press(' '),
                        // Generic Ctrl+key support
                        key if key.starts_with("C-") => {
                            let char_part = &key[2..];
                            if char_part.len() == 1 {
                                let c = char_part.chars().next().unwrap().to_ascii_lowercase();
                                self.press_with(KeyCode::Char(c), KeyModifiers::CONTROL)
                            } else {
                                match char_part {
                                    "[" => self.press_with(KeyCode::Char('['), KeyModifiers::CONTROL),
                                    "]" => self.press_with(KeyCode::Char(']'), KeyModifiers::CONTROL),
                                    "^" => self.press_with(KeyCode::Char('^'), KeyModifiers::CONTROL),
                                    " " => self.press_with(KeyCode::Char(' '), KeyModifiers::CONTROL),
                                    _ => panic!("Unknown Ctrl key: <{}>", special_key),
                                }
                            }
                        }
                        _ => panic!("Unknown special key: <{}>", special_key),
                    };
                } else {
                    // Not a special key - press '<' and put back what we consumed
                    self.press('<');
                    for ch in lookahead {
                        if ch != '>' {  // Don't put back the '>' if we found one for empty special key
                            self.press(ch);
                        }
                    }
                }
            } else {
                self.press(c);
            }
        }
        self
    }

    /// Get the full buffer content as a string (including newlines)
    pub fn buffer_content(&self) -> String {
        let mut content = String::new();
        for i in 0..self.editor.buffer().line_count() {
            if let Some(line) = self.editor.buffer().line(i) {
                content.push_str(&line);
            }
        }
        // Ensure content always ends with a newline (Vim behavior)
        if !content.ends_with('\n') {
            content.push('\n');
        }
        content
    }

    /// Get a formatted snapshot of the complete editor state
    /// This includes: buffer content, cursor position, mode, and other state
    pub fn snapshot_state(&self) -> String {
        let cursor = self.editor.buffer().cursor();
        let mode = format!("{:?}", self.editor.mode());

        // Build visual representation with cursor indicator
        let mut lines = Vec::new();
        for i in 0..self.editor.buffer().line_count() {
            if let Some(line) = self.editor.buffer().line(i) {
                let line_display = line.trim_end_matches('\n');
                if i == cursor.line() {
                    // Show cursor position with a marker
                    let before = line_display.chars().take(cursor.col()).collect::<String>();
                    let at_cursor = line_display.chars().nth(cursor.col()).unwrap_or(' ');
                    let after = line_display
                        .chars()
                        .skip(cursor.col() + 1)
                        .collect::<String>();
                    lines.push(format!("{}[{}]{}", before, at_cursor, after));
                } else {
                    lines.push(line_display.to_string());
                }
            }
        }

        format!(
            "Mode: {}\nCursor: {}:{}\nLines: {}\n\nBuffer:\n{}",
            mode,
            cursor.line(),
            cursor.col(),
            self.editor.buffer().line_count(),
            lines.join("\n")
        )
    }

    /// Get a minimal snapshot with just buffer content (no cursor markers)
    pub fn snapshot_buffer(&self) -> String {
        self.buffer_content()
    }

    /// Get snapshot with buffer and cursor position (but no visual markers)
    pub fn snapshot_buffer_and_cursor(&self) -> String {
        let cursor = self.editor.buffer().cursor();
        format!(
            "Cursor: {}:{}\n\n{}",
            cursor.line(),
            cursor.col(),
            self.buffer_content()
        )
    }

    /// Assert cursor is at expected position
    pub fn assert_cursor(&self, line: usize, col: usize) {
        let cursor = self.editor.buffer().cursor();
        assert_eq!(
            (cursor.line(), cursor.col()),
            (line, col),
            "Expected cursor at {}:{}, got {}:{}",
            line,
            col,
            cursor.line(),
            cursor.col()
        );
    }

    /// Assert mode is as expected
    pub fn assert_mode(&self, mode: Mode) {
        assert_eq!(
            self.editor.mode(),
            mode,
            "Expected mode {:?}, got {:?}",
            mode,
            self.editor.mode()
        );
    }

    /// Assert line count
    pub fn assert_line_count(&self, count: usize) {
        assert_eq!(
            self.editor.buffer().line_count(),
            count,
            "Expected {} lines, got {}",
            count,
            self.editor.buffer().line_count()
        );
    }

    /// Assert specific line content
    pub fn assert_line(&self, line_idx: usize, expected: &str) {
        let actual = self.editor.buffer().line(line_idx).unwrap_or_default();
        assert_eq!(
            actual, expected,
            "Line {} mismatch:\nExpected: {:?}\nGot: {:?}",
            line_idx, expected, actual
        );
    }

    /// Get line count
    pub fn line_count(&self) -> usize {
        self.editor.buffer().line_count()
    }

    /// Get line content
    pub fn line(&self, idx: usize) -> Option<String> {
        self.editor.buffer().line(idx)
    }

    /// Get current mode
    pub fn mode(&self) -> Mode {
        self.editor.mode()
    }

    /// Get cursor position as (line, col)
    pub fn cursor(&self) -> (usize, usize) {
        let c = self.editor.buffer().cursor();
        (c.line(), c.col())
    }

    /// Load a file into the editor
    pub fn load_file(&mut self, path: &str) -> &mut Self {
        let _ = self.editor.load_file(path);
        self
    }

    /// Set file path without loading
    pub fn set_file_path(&mut self, path: String) -> &mut Self {
        self.editor.buffer_mut().set_file_path(path);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_editor_test_basic() {
        let mut test = EditorTest::new("hello\nworld");

        // Vim semantics: "hello\nworld" becomes "hello\nworld\n" internally, displayed as 2 lines
        test.assert_line_count(2);
        test.assert_cursor(0, 0);
        test.assert_mode(Mode::Normal);

        test.keys("j");
        test.assert_cursor(1, 0);
    }

    #[test]
    fn test_fluent_api() {
        let mut test = EditorTest::new("test");

        test.press('i').type_text("hello ").press_esc();

        assert!(test.buffer_content().contains("hello"));
    }
}
