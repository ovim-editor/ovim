//! Dashboard mode input handling
//!
//! Handles the start screen dashboard with menu navigation and shortcuts.
//! j/k navigate menu, Enter selects, or press shortcut key directly.

use crate::editor::{Editor, Picker};
use crate::mode::Mode;
use crate::dashboard::MENU_ITEMS;
use anyhow::Result;
use crate::{KeyCode, KeyEvent};

/// Handles input in Dashboard mode
/// j/k navigate menu, Enter selects, or press shortcut key directly
pub fn handle_dashboard_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    let current = editor.dashboard_selected();
    let menu_count = MENU_ITEMS.len();

    match key_event.code {
        // Navigation
        KeyCode::Char('j') | KeyCode::Down => {
            let next = (current + 1) % menu_count;
            editor.set_dashboard_selected(next);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let next = if current == 0 {
                menu_count - 1
            } else {
                current - 1
            };
            editor.set_dashboard_selected(next);
        }

        // Select current item
        KeyCode::Enter => {
            execute_dashboard_action(editor, current)?;
        }

        // Direct shortcuts - use execute_dashboard_action for consistency
        KeyCode::Char('e') => {
            execute_dashboard_action(editor, 0)?; // New File
        }
        KeyCode::Char('f') => {
            execute_dashboard_action(editor, 1)?; // Find File
        }
        KeyCode::Char('r') => {
            execute_dashboard_action(editor, 2)?; // Recent Files
        }
        KeyCode::Char('g') => {
            execute_dashboard_action(editor, 3)?; // Find Word
        }
        KeyCode::Char('c') => {
            execute_dashboard_action(editor, 4)?; // Configuration
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            execute_dashboard_action(editor, 5)?; // Quit
        }

        // Any other key exits dashboard to normal mode
        KeyCode::Char(':') => {
            // Enter command mode
            editor.set_mode(Mode::Command);
        }
        KeyCode::Char('/') => {
            // Enter search mode
            editor.set_mode(Mode::Search);
            editor.set_search_forward(true);
        }

        _ => {
            // Ignore other keys
        }
    }
    Ok(())
}

/// Execute the action for the selected dashboard menu item
fn execute_dashboard_action(editor: &mut Editor, index: usize) -> Result<()> {
    match index {
        0 => {
            // New File - exit to normal mode
            editor.set_mode(Mode::Normal);
        }
        1 => {
            // Find File
            let (base_dir, preferred_dir) = editor.picker_dirs();
            let picker = Picker::new_file_finder(base_dir, preferred_dir);
            editor.set_picker(picker);
            editor.set_mode(Mode::Picker);
            editor.mark_picker_selection_changed();
        }
        2 => {
            // Recent Files - for now, use file finder (TODO: add recent files picker)
            let (base_dir, preferred_dir) = editor.picker_dirs();
            let picker = Picker::new_file_finder(base_dir, preferred_dir);
            editor.set_picker(picker);
            editor.set_mode(Mode::Picker);
            editor.mark_picker_selection_changed();
        }
        3 => {
            // Find Word (grep)
            let (base_dir, preferred_dir) = editor.picker_dirs();
            let picker = Picker::new_live_grep(base_dir, preferred_dir);
            editor.set_picker(picker);
            editor.set_mode(Mode::Picker);
        }
        4 => {
            // Configuration
            editor.set_mode(Mode::Normal);
            let config_path = dirs::config_dir()
                .map(|p| p.join("ovim").join("init.lua"))
                .unwrap_or_else(|| std::path::PathBuf::from("~/.config/ovim/init.lua"));
            let _ = editor.load_file(&config_path);
        }
        5 => {
            // Quit
            editor.quit();
        }
        _ => {}
    }
    Ok(())
}
