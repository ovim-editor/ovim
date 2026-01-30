use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

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
                panel.filter_query.pop();
            }
        }
        KeyCode::Char(c) => {
            if let Some(panel) = editor.lsp_manager_panel_mut() {
                panel.filter_query.push(c);
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_install(editor: &mut Editor) {
    let entry_info = editor
        .lsp_manager_panel()
        .and_then(|p| p.selected_entry())
        .map(|e| (e.language_id.clone(), e.has_auto_install, e.install_hint.clone()));

    if let Some((_lang_id, has_auto, hint)) = entry_info {
        if has_auto {
            // Phase 3: trigger auto-install
            editor.set_lsp_status("Install triggered (not yet implemented)".to_string());
        } else if let Some(hint) = hint {
            editor.set_lsp_status(hint);
        } else {
            editor.set_lsp_status("No install method available".to_string());
        }
    }
}

fn handle_uninstall(editor: &mut Editor) {
    let entry_info = editor
        .lsp_manager_panel()
        .and_then(|p| p.selected_entry())
        .map(|e| e.language_name.clone());

    if let Some(name) = entry_info {
        editor.set_lsp_status(format!("Uninstall {name} (not yet implemented)"));
    }
}

fn handle_update(editor: &mut Editor) {
    let entry_info = editor
        .lsp_manager_panel()
        .and_then(|p| p.selected_entry())
        .map(|e| e.language_name.clone());

    if let Some(name) = entry_info {
        editor.set_lsp_status(format!("Update {name} (not yet implemented)"));
    }
}
