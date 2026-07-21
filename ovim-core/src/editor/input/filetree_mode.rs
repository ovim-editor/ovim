//! File tree mode input handling
//!
//! Handles navigation and actions within the file tree sidebar.
//! j/k or arrows to navigate, Enter/o/l to open, h to collapse or go to parent,
//! x to collapse, r to refresh, gg/G for top/bottom. H/I toggle hidden and
//! ignored files, f filters loaded entries, and y/X/p copy, cut, and paste.
//! a adds a file, d deletes (with confirmation), and R renames.

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
        // H/I - toggle hidden and git-ignored entries
        KeyCode::Char('H') => {
            editor.file_tree_mut().toggle_hidden();
            let state = if editor.file_tree().show_hidden() {
                "shown"
            } else {
                "hidden"
            };
            editor.set_lsp_status(format!("Hidden files: {state}"));
        }
        KeyCode::Char('I') => {
            editor.file_tree_mut().toggle_ignored();
            let state = if editor.file_tree().show_ignored() {
                "shown"
            } else {
                "hidden"
            };
            editor.set_lsp_status(format!("Git-ignored files: {state}"));
        }
        // f - live filter loaded entries, F - clear filter
        KeyCode::Char('f') | KeyCode::Char('/') => {
            let input = editor.file_tree().filter().to_string();
            let cursor = input.len();
            editor
                .file_tree_mut()
                .set_pending_action(FileTreeAction::Filter { input, cursor });
        }
        KeyCode::Char('F') => {
            editor.file_tree_mut().clear_filter();
            editor.set_lsp_status("File filter cleared".to_string());
        }
        // ? - contextual key reference
        KeyCode::Char('?') => {
            editor.file_tree_mut().toggle_help();
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
                if editor.file_tree().selected_is_root() {
                    editor.set_lsp_status("The explorer root cannot be deleted".to_string());
                    return Ok(());
                }
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
                if editor.file_tree().selected_is_root() {
                    editor.set_lsp_status("The explorer root cannot be renamed".to_string());
                    return Ok(());
                }
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
        // y/X/p - copy, cut, and paste using an explorer-local clipboard
        KeyCode::Char('y') => match editor.file_tree_mut().copy_selected() {
            Ok(path) => editor.set_lsp_status(format!("Copied: {}", path.display())),
            Err(error) => editor.set_lsp_status(format!("Copy failed: {error}")),
        },
        KeyCode::Char('X') => match editor.file_tree_mut().cut_selected() {
            Ok(path) => editor.set_lsp_status(format!("Cut: {}", path.display())),
            Err(error) => editor.set_lsp_status(format!("Cut failed: {error}")),
        },
        KeyCode::Char('p') => match editor.file_tree_mut().paste_to_selected() {
            Ok(path) => editor.set_lsp_status(format!("Pasted: {}", path.display())),
            Err(error) => editor.set_lsp_status(format!("Paste failed: {error}")),
        },
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
                let is_real_directory = std::fs::symlink_metadata(&path)
                    .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink());
                if is_real_directory {
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
                    let is_directory =
                        input.ends_with('/') || input.ends_with(std::path::MAIN_SEPARATOR);
                    match editor.file_tree().resolve_new_path(&input) {
                        Ok(new_path) if new_path.exists() => {
                            editor.set_lsp_status(format!(
                                "Create failed: {} already exists",
                                new_path.display()
                            ));
                        }
                        Ok(new_path) => {
                            let result = if is_directory {
                                std::fs::create_dir_all(&new_path)
                            } else {
                                new_path
                                    .parent()
                                    .map(std::fs::create_dir_all)
                                    .transpose()
                                    .and_then(|_| {
                                        std::fs::OpenOptions::new()
                                            .write(true)
                                            .create_new(true)
                                            .open(&new_path)
                                            .map(|_| ())
                                    })
                            };
                            match result {
                                Ok(()) => {
                                    editor
                                        .set_lsp_status(format!("Created: {}", new_path.display()));
                                    editor.file_tree_mut().refresh();
                                    editor.file_tree_mut().reveal_path(&new_path);
                                }
                                Err(error) => {
                                    editor.set_lsp_status(format!("Create failed: {error}"))
                                }
                            }
                        }
                        Err(error) => editor.set_lsp_status(format!("Create failed: {error}")),
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
                    match editor
                        .file_tree()
                        .resolve_rename_path(&original_path, &input)
                    {
                        Ok(new_path) if new_path != original_path && new_path.exists() => {
                            editor.set_lsp_status(format!(
                                "Rename failed: {} already exists",
                                new_path.display()
                            ));
                        }
                        Ok(new_path) if new_path == original_path => {}
                        Ok(new_path) => {
                            if let Err(error) = std::fs::rename(&original_path, &new_path) {
                                editor.set_lsp_status(format!("Rename failed: {error}"));
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
                                editor.file_tree_mut().reveal_path(&new_path);
                            }
                        }
                        Err(error) => editor.set_lsp_status(format!("Rename failed: {error}")),
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

        FileTreeAction::Filter { mut input, cursor } => match key_event.code {
            KeyCode::Esc => {
                editor
                    .file_tree_mut()
                    .set_pending_action(FileTreeAction::None);
            }
            KeyCode::Enter => {
                editor
                    .file_tree_mut()
                    .set_pending_action(FileTreeAction::None);
            }
            KeyCode::Backspace => {
                if cursor == 0 {
                    if input.is_empty() {
                        editor
                            .file_tree_mut()
                            .set_pending_action(FileTreeAction::None);
                    }
                } else {
                    let previous = input[..cursor]
                        .char_indices()
                        .next_back()
                        .map(|(index, _)| index)
                        .unwrap_or(0);
                    input.remove(previous);
                    update_filter(editor, input, previous);
                }
            }
            KeyCode::Left => {
                if cursor > 0 {
                    let previous = input[..cursor]
                        .char_indices()
                        .next_back()
                        .map(|(index, _)| index)
                        .unwrap_or(0);
                    editor
                        .file_tree_mut()
                        .set_pending_action(FileTreeAction::Filter {
                            input,
                            cursor: previous,
                        });
                }
            }
            KeyCode::Right => {
                if cursor < input.len() {
                    let next = input[cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(index, _)| cursor + index)
                        .unwrap_or(input.len());
                    editor
                        .file_tree_mut()
                        .set_pending_action(FileTreeAction::Filter {
                            input,
                            cursor: next,
                        });
                }
            }
            KeyCode::Char(character) => {
                input.insert(cursor, character);
                let next = cursor + character.len_utf8();
                update_filter(editor, input, next);
            }
            _ => {}
        },
    }

    Ok(())
}

fn update_filter(editor: &mut Editor, input: String, cursor: usize) {
    editor.file_tree_mut().set_filter(input.clone());
    editor
        .file_tree_mut()
        .set_pending_action(FileTreeAction::Filter { input, cursor });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Modifiers;

    fn key(character: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(character), Modifiers::NONE)
    }

    #[test]
    fn root_delete_and_rename_are_refused_before_showing_a_prompt() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = Editor::new();
        editor.open_directory(directory.path()).unwrap();

        handle_filetree_mode(&mut editor, key('d')).unwrap();
        assert!(matches!(
            editor.file_tree().pending_action(),
            FileTreeAction::None
        ));
        assert!(editor.lsp_status().contains("cannot be deleted"));

        handle_filetree_mode(&mut editor, key('R')).unwrap();
        assert!(matches!(
            editor.file_tree().pending_action(),
            FileTreeAction::None
        ));
        assert!(editor.lsp_status().contains("cannot be renamed"));
    }

    #[test]
    fn filter_prompt_updates_the_tree_as_text_is_typed() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("astro.config.mjs"), "config").unwrap();
        std::fs::write(directory.path().join("Cargo.toml"), "cargo").unwrap();
        let mut editor = Editor::new();
        editor.open_directory(directory.path()).unwrap();

        handle_filetree_mode(&mut editor, key('f')).unwrap();
        for character in "astro".chars() {
            handle_filetree_mode(&mut editor, key(character)).unwrap();
        }

        assert_eq!(editor.file_tree().filter(), "astro");
        assert!(editor
            .file_tree()
            .flattened()
            .iter()
            .any(|node| node.name() == "astro.config.mjs"));
        assert!(!editor
            .file_tree()
            .flattened()
            .iter()
            .any(|node| node.name() == "Cargo.toml"));
    }
}
