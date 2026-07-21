//! File tree mode input handling
//!
//! Handles navigation and actions within the file tree sidebar.
//! j/k or arrows to navigate, Enter/o/l to open, h to collapse or go to parent,
//! x to collapse, r to refresh, gg/G for top/bottom. H/I toggle hidden and
//! ignored files, f filters loaded entries, and y/X/p copy, cut, and paste.
//! a adds a file, d deletes (with confirmation), and R renames.

use crate::editor::filetree::FileTreeAction;
use crate::editor::{Editor, SingleLineInput};
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
            editor.set_status_message(format!("Hidden files: {state}"));
        }
        KeyCode::Char('I') => {
            editor.file_tree_mut().toggle_ignored();
            let state = if editor.file_tree().show_ignored() {
                "shown"
            } else {
                "hidden"
            };
            editor.set_status_message(format!("Git-ignored files: {state}"));
        }
        // f - live filter loaded entries, F - clear filter
        KeyCode::Char('f') | KeyCode::Char('/') => {
            let input = SingleLineInput::new(editor.file_tree().filter());
            editor
                .file_tree_mut()
                .set_pending_action(FileTreeAction::Filter { input });
        }
        KeyCode::Char('F') => {
            editor.file_tree_mut().clear_filter();
            editor.set_status_message("File filter cleared");
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
                        input: SingleLineInput::default(),
                    });
            }
        }
        // d - delete selected file/directory (with confirmation)
        KeyCode::Char('d') => {
            if let Some(node) = editor.file_tree().selected_node() {
                if editor.file_tree().selected_is_root() {
                    editor.set_status_message("The explorer root cannot be deleted");
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
                    editor.set_status_message("The explorer root cannot be renamed");
                    return Ok(());
                }
                let original_path = node.path().to_path_buf();
                let name = node.name().to_string();
                editor
                    .file_tree_mut()
                    .set_pending_action(FileTreeAction::Rename {
                        input: SingleLineInput::new(name),
                        original_path,
                    });
            }
        }
        // y/X/p - copy, cut, and paste using an explorer-local clipboard
        KeyCode::Char('y') => match editor.file_tree_mut().copy_selected() {
            Ok(path) => editor.set_status_message(format!("Copied: {}", path.display())),
            Err(error) => editor.set_status_message(format!("Copy failed: {error}")),
        },
        KeyCode::Char('X') => match editor.file_tree_mut().cut_selected() {
            Ok(path) => editor.set_status_message(format!("Cut: {}", path.display())),
            Err(error) => editor.set_status_message(format!("Cut failed: {error}")),
        },
        KeyCode::Char('p') => match editor.file_tree_mut().paste_to_selected() {
            Ok(path) => editor.set_status_message(format!("Pasted: {}", path.display())),
            Err(error) => editor.set_status_message(format!("Paste failed: {error}")),
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
                clear_prompt(editor);
                match editor.file_tree_mut().delete_entry(&path) {
                    Ok(deleted) => {
                        editor.set_status_message(format!("Deleted: {}", deleted.display()))
                    }
                    Err(error) => editor.set_status_message(format!("Delete failed: {error}")),
                }
            }
            _ => {
                // Any other key cancels
                editor
                    .file_tree_mut()
                    .set_pending_action(FileTreeAction::None);
            }
        },

        FileTreeAction::Add { input } => handle_add_prompt(editor, input, key_event.code),
        FileTreeAction::Rename {
            input,
            original_path,
        } => handle_rename_prompt(editor, input, original_path, key_event.code),
        FileTreeAction::Filter { input } => handle_filter_prompt(editor, input, key_event.code),
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptEdit {
    Cancel,
    Submit,
    Changed,
    Unchanged,
}

fn edit_prompt(input: &mut SingleLineInput, key: KeyCode) -> PromptEdit {
    match key {
        KeyCode::Esc => PromptEdit::Cancel,
        KeyCode::Enter => PromptEdit::Submit,
        KeyCode::Backspace if input.is_empty() => PromptEdit::Cancel,
        KeyCode::Backspace => changed_if(input.backspace()),
        KeyCode::Left => changed_if(input.move_left()),
        KeyCode::Right => changed_if(input.move_right()),
        KeyCode::Char(character) => changed_if(input.insert(character)),
        _ => PromptEdit::Unchanged,
    }
}

fn changed_if(changed: bool) -> PromptEdit {
    if changed {
        PromptEdit::Changed
    } else {
        PromptEdit::Unchanged
    }
}

fn handle_add_prompt(editor: &mut Editor, mut input: SingleLineInput, key: KeyCode) {
    match edit_prompt(&mut input, key) {
        PromptEdit::Cancel => clear_prompt(editor),
        PromptEdit::Submit => {
            clear_prompt(editor);
            create_from_prompt(editor, input.text());
        }
        PromptEdit::Changed => editor
            .file_tree_mut()
            .set_pending_action(FileTreeAction::Add { input }),
        PromptEdit::Unchanged => {}
    }
}

fn create_from_prompt(editor: &mut Editor, input: &str) {
    if input.is_empty() {
        return;
    }
    match editor.file_tree_mut().create_entry(input) {
        Ok(created) => editor.set_status_message(format!("Created: {}", created.display())),
        Err(error) => editor.set_status_message(format!("Create failed: {error}")),
    }
}

fn handle_rename_prompt(
    editor: &mut Editor,
    mut input: SingleLineInput,
    original_path: std::path::PathBuf,
    key: KeyCode,
) {
    match edit_prompt(&mut input, key) {
        PromptEdit::Cancel => clear_prompt(editor),
        PromptEdit::Submit => {
            clear_prompt(editor);
            rename_from_prompt(editor, &original_path, input.text());
        }
        PromptEdit::Changed => editor
            .file_tree_mut()
            .set_pending_action(FileTreeAction::Rename {
                input,
                original_path,
            }),
        PromptEdit::Unchanged => {}
    }
}

fn rename_from_prompt(editor: &mut Editor, original_path: &std::path::Path, input: &str) {
    match editor.file_tree_mut().rename_entry(original_path, input) {
        Ok(Some(_)) => editor.set_status_message(format!(
            "Renamed: {} -> {}",
            original_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            input
        )),
        Ok(None) => {}
        Err(error) => editor.set_status_message(format!("Rename failed: {error}")),
    }
}

fn handle_filter_prompt(editor: &mut Editor, mut input: SingleLineInput, key: KeyCode) {
    match edit_prompt(&mut input, key) {
        PromptEdit::Cancel | PromptEdit::Submit => clear_prompt(editor),
        PromptEdit::Changed => {
            editor.file_tree_mut().set_filter(input.text().to_owned());
            editor
                .file_tree_mut()
                .set_pending_action(FileTreeAction::Filter { input });
        }
        PromptEdit::Unchanged => {}
    }
}

fn clear_prompt(editor: &mut Editor) {
    editor
        .file_tree_mut()
        .set_pending_action(FileTreeAction::None);
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
        assert!(editor.status_message().contains("cannot be deleted"));

        handle_filetree_mode(&mut editor, key('R')).unwrap();
        assert!(matches!(
            editor.file_tree().pending_action(),
            FileTreeAction::None
        ));
        assert!(editor.status_message().contains("cannot be renamed"));
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

    #[test]
    fn filter_prompt_edits_unicode_at_character_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = Editor::new();
        editor.open_directory(directory.path()).unwrap();

        handle_filetree_mode(&mut editor, key('f')).unwrap();
        handle_filetree_mode(&mut editor, key('é')).unwrap();
        handle_filetree_mode(&mut editor, key('x')).unwrap();
        handle_filetree_mode(&mut editor, KeyEvent::new(KeyCode::Left, Modifiers::NONE)).unwrap();
        handle_filetree_mode(
            &mut editor,
            KeyEvent::new(KeyCode::Backspace, Modifiers::NONE),
        )
        .unwrap();

        let FileTreeAction::Filter { input } = editor.file_tree().pending_action() else {
            panic!("filter prompt should remain active");
        };
        assert_eq!(input.text(), "x");
        assert_eq!(input.cursor(), 0);
        assert_eq!(editor.file_tree().filter(), "x");
    }
}
