use crate::editor::Editor;
use crate::mode::Mode;
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;

pub fn handle_rename_input_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        KeyCode::Char('a') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.rename_input_mut().move_end();
        }
        KeyCode::Char(character) => {
            editor.rename_input_mut().insert(character);
        }
        KeyCode::Backspace => {
            let input = editor.rename_input_mut();
            if !input.backspace() && input.is_empty() {
                editor.set_mode(Mode::Normal);
            }
        }
        KeyCode::Delete => {
            editor.rename_input_mut().delete();
        }
        KeyCode::Left => {
            editor.rename_input_mut().move_left();
        }
        KeyCode::Right => {
            editor.rename_input_mut().move_right();
        }
        KeyCode::Home => {
            editor.rename_input_mut().move_home();
        }
        KeyCode::End => {
            editor.rename_input_mut().move_end();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, Modifiers::NONE)
    }

    #[test]
    fn edits_unicode_without_invalidating_the_cursor() {
        let mut editor = Editor::new();
        editor.set_rename_buffer("a🙂z".to_owned());
        editor.set_mode(Mode::RenameInput);

        handle_rename_input_mode(&mut editor, key(KeyCode::Left)).unwrap();
        handle_rename_input_mode(&mut editor, key(KeyCode::Backspace)).unwrap();
        handle_rename_input_mode(&mut editor, key(KeyCode::Char('é'))).unwrap();
        handle_rename_input_mode(&mut editor, key(KeyCode::Delete)).unwrap();

        assert_eq!(editor.rename_buffer(), "aé");
        assert_eq!(editor.rename_cursor(), 3);
        assert_eq!(editor.mode(), Mode::RenameInput);
    }

    #[test]
    fn backspace_only_cancels_when_the_input_is_empty() {
        let mut editor = Editor::new();
        editor.set_rename_buffer("name".to_owned());
        editor.set_mode(Mode::RenameInput);

        handle_rename_input_mode(&mut editor, key(KeyCode::Home)).unwrap();
        handle_rename_input_mode(&mut editor, key(KeyCode::Backspace)).unwrap();
        assert_eq!(editor.mode(), Mode::RenameInput);

        editor.set_rename_buffer(String::new());
        handle_rename_input_mode(&mut editor, key(KeyCode::Backspace)).unwrap();
        assert_eq!(editor.mode(), Mode::Normal);
    }
}
