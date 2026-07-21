use crate::{KeyCode, KeyEvent};
use anyhow::Result;

use crate::editor::Editor;

pub fn handle_lsp_manager_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    // If filter is focused, handle text input
    if editor
        .lsp_manager_panel()
        .map(|p| p.filter_focused)
        .unwrap_or(false)
    {
        return handle_filter_input(editor, key_event);
    }

    match key_event.code {
        KeyCode::Char('q') | KeyCode::Esc => {
            editor.close_lsp_manager();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(panel) = editor.lsp_manager_panel_mut() {
                panel.move_down();
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(panel) = editor.lsp_manager_panel_mut() {
                panel.move_up();
            }
        }
        KeyCode::Char('g') => {
            if let Some(panel) = editor.lsp_manager_panel_mut() {
                panel.jump_to_top();
            }
        }
        KeyCode::Char('G') => {
            if let Some(panel) = editor.lsp_manager_panel_mut() {
                panel.jump_to_bottom();
            }
        }
        KeyCode::Char('K') => {
            if let Some(panel) = editor.lsp_manager_panel_mut() {
                panel.show_detail = !panel.show_detail;
            }
        }
        KeyCode::Char('/') => {
            if let Some(panel) = editor.lsp_manager_panel_mut() {
                panel.filter_focused = true;
            }
        }
        KeyCode::Char('i') | KeyCode::Enter => {
            // Install selected entry (Phase 3)
            handle_install(editor);
        }
        KeyCode::Char('x') => {
            // Uninstall selected entry (Phase 3)
            handle_uninstall(editor);
        }
        KeyCode::Char('u') => {
            // Update/reinstall selected entry (Phase 3)
            handle_update(editor);
        }
        _ => {}
    }
    Ok(())
}

fn handle_filter_input(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        KeyCode::Esc => {
            if let Some(panel) = editor.lsp_manager_panel_mut() {
                panel.filter_focused = false;
            }
        }
        KeyCode::Enter => {
            if let Some(panel) = editor.lsp_manager_panel_mut() {
                panel.filter_focused = false;
                // Select first filtered result
                let filtered = panel.filtered_entries();
                if let Some((idx, _)) = filtered.first() {
                    panel.selected_index = *idx;
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(panel) = editor.lsp_manager_panel_mut() {
                if panel.filter_input_mut().backspace() {
                    panel.jump_to_top();
                }
            }
        }
        KeyCode::Delete => {
            if let Some(panel) = editor.lsp_manager_panel_mut() {
                if panel.filter_input_mut().delete() {
                    panel.jump_to_top();
                }
            }
        }
        KeyCode::Left => {
            if let Some(panel) = editor.lsp_manager_panel_mut() {
                panel.filter_input_mut().move_left();
            }
        }
        KeyCode::Right => {
            if let Some(panel) = editor.lsp_manager_panel_mut() {
                panel.filter_input_mut().move_right();
            }
        }
        KeyCode::Home => {
            if let Some(panel) = editor.lsp_manager_panel_mut() {
                panel.filter_input_mut().move_home();
            }
        }
        KeyCode::End => {
            if let Some(panel) = editor.lsp_manager_panel_mut() {
                panel.filter_input_mut().move_end();
            }
        }
        KeyCode::Char(c) => {
            if let Some(panel) = editor.lsp_manager_panel_mut() {
                if panel.filter_input_mut().insert(c) {
                    panel.jump_to_top();
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_install(editor: &mut Editor) {
    let lang_id = editor
        .lsp_manager_panel()
        .and_then(|p| p.selected_entry())
        .map(|e| e.language_id.clone());

    if let Some(lang_id) = lang_id {
        editor.request_lsp_install(&lang_id);
    }
}

fn handle_uninstall(editor: &mut Editor) {
    let lang_id = editor
        .lsp_manager_panel()
        .and_then(|p| p.selected_entry())
        .map(|e| e.language_id.clone());

    if let Some(lang_id) = lang_id {
        editor.request_lsp_uninstall(&lang_id);
    }
}

fn handle_update(editor: &mut Editor) {
    // Update = reinstall
    let lang_id = editor
        .lsp_manager_panel()
        .and_then(|p| p.selected_entry())
        .map(|e| e.language_id.clone());

    if let Some(lang_id) = lang_id {
        editor.request_lsp_install(&lang_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Modifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, Modifiers::NONE)
    }

    fn editor_with_filter_focused() -> Editor {
        let mut editor = Editor::new();
        editor.open_lsp_manager();
        editor.lsp_manager_panel_mut().unwrap().filter_focused = true;
        editor
    }

    #[test]
    fn filter_supports_unicode_safe_mid_string_edits() {
        let mut editor = editor_with_filter_focused();
        for character in "a🙂z".chars() {
            handle_lsp_manager_mode(&mut editor, key(KeyCode::Char(character))).unwrap();
        }

        handle_lsp_manager_mode(&mut editor, key(KeyCode::Left)).unwrap();
        handle_lsp_manager_mode(&mut editor, key(KeyCode::Backspace)).unwrap();
        handle_lsp_manager_mode(&mut editor, key(KeyCode::Char('é'))).unwrap();
        handle_lsp_manager_mode(&mut editor, key(KeyCode::Delete)).unwrap();

        let panel = editor.lsp_manager_panel().unwrap();
        assert_eq!(panel.filter_query(), "aé");
        assert_eq!(panel.filter_cursor(), 3);
    }

    #[test]
    fn home_end_and_delete_edit_the_filter_at_the_cursor() {
        let mut editor = editor_with_filter_focused();
        for character in "rust".chars() {
            handle_lsp_manager_mode(&mut editor, key(KeyCode::Char(character))).unwrap();
        }

        handle_lsp_manager_mode(&mut editor, key(KeyCode::Home)).unwrap();
        handle_lsp_manager_mode(&mut editor, key(KeyCode::Delete)).unwrap();
        handle_lsp_manager_mode(&mut editor, key(KeyCode::End)).unwrap();
        handle_lsp_manager_mode(&mut editor, key(KeyCode::Char('y'))).unwrap();

        let panel = editor.lsp_manager_panel().unwrap();
        assert_eq!(panel.filter_query(), "usty");
        assert_eq!(panel.filter_cursor(), 4);
    }
}
