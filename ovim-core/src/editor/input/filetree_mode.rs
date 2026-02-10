//! File tree mode input handling
//!
//! Handles navigation and actions within the file tree sidebar.
//! j/k or arrows to navigate, Enter/o/l to open, h to collapse or go to parent,
//! x to collapse, r to refresh, gg/G for top/bottom.
//! a to add file, d to delete (with confirm), R to rename.

use crate::editor::filetree::FileTreeAction;
use crate::editor::Editor;
use crate::mode::Mode;
use crate::{KeyCode, KeyEvent};
use anyhow::Result;

/// Handles input in FileTree mode
pub fn handle_filetree_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    // If there's a pending action, route to the prompt handler
    if !matches!(editor.file_tree().pending_action(), FileTreeAction::None) {
        return handle_prompt_input(editor, key_event);
    }

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
        // a - add new file
        KeyCode::Char('a') => {
            if editor.file_tree().selected_parent_dir().is_some() {
                editor
                    .file_tree_mut()
                    .set_pending_action(FileTreeAction::Add {
                        input: String::new(),
                        cursor: 0,
                    });
            }
        }
        // d - delete selected file/directory (with confirmation)
        KeyCode::Char('d') => {
            if let Some(node) = editor.file_tree().selected_node() {
                let path = node.path().to_path_buf();
                let name = node.name().to_string();
                editor
                    .file_tree_mut()
                    .set_pending_action(FileTreeAction::DeleteConfirm { path, name });
            }
        }
        // R - rename selected file/directory
        KeyCode::Char('R') => {
            if let Some(node) = editor.file_tree().selected_node() {
                let original_path = node.path().to_path_buf();
                let name = node.name().to_string();
                let cursor = name.len();
                editor
                    .file_tree_mut()
                    .set_pending_action(FileTreeAction::Rename {
                        input: name,
                        cursor,
                        original_path,
                    });
            }
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

/// Handles input while a prompt is active (add/rename text input, delete confirmation)
fn handle_prompt_input(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    // Clone the action to avoid borrow issues
    let action = editor.file_tree().pending_action().clone();

    match action {
        FileTreeAction::None => unreachable!(),

        FileTreeAction::DeleteConfirm { path, .. } => match key_event.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                editor
                    .file_tree_mut()
                    .set_pending_action(FileTreeAction::None);
                // Perform the delete
                if path.is_dir() {
                    if let Err(e) = std::fs::remove_dir_all(&path) {
                        editor.set_lsp_status(format!("Delete failed: {e}"));
                    } else {
                        editor.set_lsp_status(format!("Deleted: {}", path.display()));
                        editor.file_tree_mut().refresh();
                    }
                } else if let Err(e) = std::fs::remove_file(&path) {
                    editor.set_lsp_status(format!("Delete failed: {e}"));
                } else {
                    editor.set_lsp_status(format!("Deleted: {}", path.display()));
                    editor.file_tree_mut().refresh();
                }
            }
            _ => {
                // Any other key cancels
                editor
                    .file_tree_mut()
                    .set_pending_action(FileTreeAction::None);
            }
        },

        FileTreeAction::Add { mut input, cursor } => match key_event.code {
            KeyCode::Esc => {
                editor
                    .file_tree_mut()
                    .set_pending_action(FileTreeAction::None);
            }
            KeyCode::Enter => {
                editor
                    .file_tree_mut()
                    .set_pending_action(FileTreeAction::None);
                if !input.is_empty() {
                    if let Some(parent) = editor.file_tree().selected_parent_dir() {
                        let new_path = parent.join(&input);
                        if input.ends_with('/') || input.ends_with(std::path::MAIN_SEPARATOR) {
                            // Create directory
                            if let Err(e) = std::fs::create_dir_all(&new_path) {
                                editor.set_lsp_status(format!("Create dir failed: {e}"));
                            } else {
                                editor.set_lsp_status(format!(
                                    "Created: {}",
                                    new_path.display()
                                ));
                                editor.file_tree_mut().refresh();
                            }
                        } else {
                            // Create file (and parent dirs if needed)
                            if let Some(parent_dir) = new_path.parent() {
                                let _ = std::fs::create_dir_all(parent_dir);
                            }
                            if let Err(e) = std::fs::File::create(&new_path) {
                                editor.set_lsp_status(format!("Create file failed: {e}"));
                            } else {
                                editor.set_lsp_status(format!(
                                    "Created: {}",
                                    new_path.display()
                                ));
                                editor.file_tree_mut().refresh();
                            }
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                if cursor == 0 {
                    if input.is_empty() {
                        editor
                            .file_tree_mut()
                            .set_pending_action(FileTreeAction::None);
                    }
                } else {
                    let prev = input[..cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    input.remove(prev);
                    editor
                        .file_tree_mut()
                        .set_pending_action(FileTreeAction::Add {
                            input,
                            cursor: prev,
                        });
                }
            }
            KeyCode::Left => {
                if cursor > 0 {
                    let prev = input[..cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    editor
                        .file_tree_mut()
                        .set_pending_action(FileTreeAction::Add {
                            input,
                            cursor: prev,
                        });
                }
            }
            KeyCode::Right => {
                if cursor < input.len() {
                    let next = input[cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| cursor + i)
                        .unwrap_or(input.len());
                    editor
                        .file_tree_mut()
                        .set_pending_action(FileTreeAction::Add {
                            input,
                            cursor: next,
                        });
                }
            }
            KeyCode::Char(ch) => {
                input.insert(cursor, ch);
                let new_cursor = cursor + ch.len_utf8();
                editor
                    .file_tree_mut()
                    .set_pending_action(FileTreeAction::Add {
                        input,
                        cursor: new_cursor,
                    });
            }
            _ => {}
        },

        FileTreeAction::Rename {
            mut input,
            cursor,
            original_path,
        } => match key_event.code {
            KeyCode::Esc => {
                editor
                    .file_tree_mut()
                    .set_pending_action(FileTreeAction::None);
            }
            KeyCode::Enter => {
                editor
                    .file_tree_mut()
                    .set_pending_action(FileTreeAction::None);
                if !input.is_empty() {
                    if let Some(parent) = original_path.parent() {
                        let new_path = parent.join(&input);
                        if let Err(e) = std::fs::rename(&original_path, &new_path) {
                            editor.set_lsp_status(format!("Rename failed: {e}"));
                        } else {
                            editor.set_lsp_status(format!(
                                "Renamed: {} -> {}",
                                original_path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy(),
                                input
                            ));
                            editor.file_tree_mut().refresh();
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                if cursor == 0 {
                    if input.is_empty() {
                        editor
                            .file_tree_mut()
                            .set_pending_action(FileTreeAction::None);
                    }
                } else {
                    let prev = input[..cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    input.remove(prev);
                    editor
                        .file_tree_mut()
                        .set_pending_action(FileTreeAction::Rename {
                            input,
                            cursor: prev,
                            original_path,
                        });
                }
            }
            KeyCode::Left => {
                if cursor > 0 {
                    let prev = input[..cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    editor
                        .file_tree_mut()
                        .set_pending_action(FileTreeAction::Rename {
                            input,
                            cursor: prev,
                            original_path,
                        });
                }
            }
            KeyCode::Right => {
                if cursor < input.len() {
                    let next = input[cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| cursor + i)
                        .unwrap_or(input.len());
                    editor
                        .file_tree_mut()
                        .set_pending_action(FileTreeAction::Rename {
                            input,
                            cursor: next,
                            original_path,
                        });
                }
            }
            KeyCode::Char(ch) => {
                input.insert(cursor, ch);
                let new_cursor = cursor + ch.len_utf8();
                editor
                    .file_tree_mut()
                    .set_pending_action(FileTreeAction::Rename {
                        input,
                        cursor: new_cursor,
                        original_path,
                    });
            }
            _ => {}
        },
    }

    Ok(())
}
