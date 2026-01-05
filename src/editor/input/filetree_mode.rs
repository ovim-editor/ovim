//! File tree mode input handling
//!
//! Handles navigation and actions within the file tree sidebar.
//! j/k or arrows to navigate, Enter/o/l to open, x/h to collapse, r to refresh.

use crate::editor::Editor;
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

/// Handles input in FileTree mode
pub fn handle_filetree_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        // Esc or q - close file tree
        KeyCode::Esc | KeyCode::Char('q') => {
            editor.toggle_file_tree();
        }
        // j or Down - move selection down
        KeyCode::Char('j') | KeyCode::Down => {
            editor.file_tree_mut().select_next();
        }
        // k or Up - move selection up
        KeyCode::Char('k') | KeyCode::Up => {
            editor.file_tree_mut().select_previous();
        }
        // Enter or o - open file or toggle directory
        KeyCode::Enter | KeyCode::Char('o') => {
            editor.open_file_from_tree();
        }
        // x or h - collapse directory
        KeyCode::Char('x') | KeyCode::Char('h') => {
            // Only collapse if it's an expanded directory
            if let Some(node) = editor.file_tree().selected_node() {
                if node.is_dir() && node.is_expanded() {
                    editor.file_tree_mut().toggle_selected();
                }
            }
        }
        // l - expand directory or open file
        KeyCode::Char('l') => {
            editor.open_file_from_tree();
        }
        // r - refresh file tree
        KeyCode::Char('r') => {
            editor.file_tree_mut().refresh();
        }
        // Tab - switch focus to buffer
        KeyCode::Tab => {
            editor.set_mode(Mode::Normal);
        }
        _ => {
            // Ignore other keys
        }
    }
    Ok(())
}
