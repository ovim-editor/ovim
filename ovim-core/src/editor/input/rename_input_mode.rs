use crate::editor::Editor;
use crate::mode::Mode;
use anyhow::Result;
use crate::{KeyCode, KeyEvent, Modifiers};

pub fn handle_rename_input_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        KeyCode::Char(ch) => {
            if key_event.modifiers.contains(Modifiers::CONTROL) && ch == 'a' {
                // Select all: move cursor to end (effectively selects all text for overwrite)
                let len = editor.rename_buffer().len();
                editor.set_rename_cursor(len);
                return Ok(());
            }
            // Insert character at cursor position
            let pos = editor.rename_cursor();
            let mut buf = editor.rename_buffer().to_string();
            buf.insert(pos, ch);
            let new_pos = pos + ch.len_utf8();
            editor.set_rename_buffer(buf);
            editor.set_rename_cursor(new_pos);
        }
        KeyCode::Backspace => {
            let pos = editor.rename_cursor();
            if pos == 0 {
                // Empty backspace at start cancels
                if editor.rename_buffer().is_empty() {
                    editor.set_mode(Mode::Normal);
                }
                return Ok(());
            }
            let mut buf = editor.rename_buffer().to_string();
            // Find previous char boundary
            let prev = buf[..pos]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            buf.remove(prev);
            editor.set_rename_buffer(buf);
            editor.set_rename_cursor(prev);
        }
        KeyCode::Delete => {
            let pos = editor.rename_cursor();
            let buf_len = editor.rename_buffer().len();
            if pos < buf_len {
                let mut buf = editor.rename_buffer().to_string();
                buf.remove(pos);
                editor.set_rename_buffer(buf);
            }
        }
        KeyCode::Left => {
            let pos = editor.rename_cursor();
            if pos > 0 {
                let buf = editor.rename_buffer();
                let prev = buf[..pos]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                editor.set_rename_cursor(prev);
            }
        }
        KeyCode::Right => {
            let pos = editor.rename_cursor();
            let buf = editor.rename_buffer();
            if pos < buf.len() {
                let next = buf[pos..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| pos + i)
                    .unwrap_or(buf.len());
                editor.set_rename_cursor(next);
            }
        }
        KeyCode::Home => {
            editor.set_rename_cursor(0);
        }
        KeyCode::End => {
            let len = editor.rename_buffer().len();
            editor.set_rename_cursor(len);
        }
        KeyCode::Enter => {
            let new_name = editor.rename_buffer().to_string();
            if !new_name.is_empty() {
                editor.request_rename(new_name);
            }
            editor.set_mode(Mode::Normal);
        }
        KeyCode::Esc => {
            editor.set_mode(Mode::Normal);
        }
        _ => {}
    }
    Ok(())
}
