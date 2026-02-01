//! File tree mode input handling
//!
//! Handles navigation and actions within the file tree sidebar.
//! j/k or arrows to navigate, Enter/o/l to open, h to collapse or go to parent,
//! x to collapse, r to refresh, gg/G for top/bottom.

use crate::editor::Editor;
use crate::mode::Mode;
use anyhow::Result;
use ovim_core::{KeyCode, KeyEvent};

/// Handles input in FileTree mode
pub fn handle_filetree_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    // Handle pending 'g' for gg command
    if editor.file_tree().pending_g() {
        editor.file_tree_mut().set_pending_g(false);
        match key_event.code {
            KeyCode::Char('g') => {
                // gg - go to top of tree
                editor.file_tree_mut().select_first();
            }
            _ => {
                // Invalid second key after g, ignore
            }
        }
        return Ok(());
    }

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
        // h - collapse expanded dir, or navigate to parent
        KeyCode::Char('h') => {
            let should_navigate_parent = if let Some(node) = editor.file_tree().selected_node() {
                // If it's an expanded directory, collapse it
                if node.is_dir() && node.is_expanded() {
                    false // will toggle_selected instead
                } else {
                    true // navigate to parent
                }
            } else {
                false
            };

            if should_navigate_parent {
                editor.file_tree_mut().navigate_to_parent();
            } else {
                editor.file_tree_mut().toggle_selected();
            }
        }
        // x - collapse directory (only collapses, never navigates)
        KeyCode::Char('x') => {
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
        // g - first key of gg (go to top)
        KeyCode::Char('g') => {
            editor.file_tree_mut().set_pending_g(true);
        }
        // G - go to bottom of tree
        KeyCode::Char('G') => {
            editor.file_tree_mut().select_last();
        }
        // Tab - switch focus to buffer (keep tree open)
        KeyCode::Tab => {
            editor.set_mode(Mode::Normal);
        }
        _ => {
            // Ignore other keys
        }
    }
    Ok(())
}
