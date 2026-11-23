use crate::editor::path_completion::extract_path_from_command;
use crate::editor::{Editor, Mode};
use crate::{KeyCode, KeyEvent};
use anyhow::Result;

/// Handles input in Command mode
pub fn handle_command_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    if let Some(text) = super::helpers::take_prompt_register_text(editor, &key_event) {
        editor.insert_into_command_line(&text);
        update_path_completion(editor);
        return Ok(());
    }
    if let Some(control) = super::helpers::prompt_control(&key_event) {
        use super::helpers::PromptControl;
        match control {
            PromptControl::Cancel => {
                return handle_command_mode(
                    editor,
                    KeyEvent::new(KeyCode::Esc, crate::Modifiers::NONE),
                );
            }
            PromptControl::Backspace => {
                return handle_command_mode(
                    editor,
                    KeyEvent::new(KeyCode::Backspace, crate::Modifiers::NONE),
                );
            }
            PromptControl::DeleteToStart => {
                editor.delete_command_line_to_start();
                update_path_completion(editor);
            }
            PromptControl::DeleteWord => {
                editor.delete_command_line_word();
                update_path_completion(editor);
            }
            PromptControl::Home => editor.move_command_cursor_home(),
            PromptControl::End => editor.move_command_cursor_end(),
            PromptControl::InsertRegister => editor.editing.pending_register_insert = true,
            PromptControl::Ignore => {}
        }
        return Ok(());
    }
    match key_event.code {
        KeyCode::Char(ch) => {
            editor.append_to_command_line(ch);
            update_path_completion(editor);
        }
        KeyCode::Backspace => {
            if editor.command_line().is_empty() {
                editor.path_completion_mut().hide();
                editor.set_mode(Mode::Normal);
            } else {
                editor.backspace_command_line();
                update_path_completion(editor);
            }
        }
        KeyCode::Delete => {
            editor.delete_command_line_char();
            update_path_completion(editor);
        }
        KeyCode::Tab => {
            handle_tab_completion(editor, false);
        }
        KeyCode::BackTab => {
            handle_tab_completion(editor, true);
        }
        KeyCode::Up => {
            if editor.path_completion().is_visible() {
                editor.path_completion_mut().select_previous();
                accept_selected_into_command_line(editor);
            } else {
                editor.history_prev();
            }
        }
        KeyCode::Down => {
            if editor.path_completion().is_visible() {
                editor.path_completion_mut().select_next();
                accept_selected_into_command_line(editor);
            } else {
                editor.history_next();
            }
        }
        KeyCode::Left => {
            editor.move_command_cursor_left();
        }
        KeyCode::Right => {
            editor.move_command_cursor_right();
        }
        KeyCode::Home => {
            editor.move_command_cursor_home();
        }
        KeyCode::End => {
            editor.move_command_cursor_end();
        }
        KeyCode::Enter => {
            if editor.path_completion().is_visible() {
                if editor.path_completion().selected_is_dir() {
                    // Directory: accept into command line, refresh completions, don't execute.
                    if let Some(new_path) = editor.path_completion().accept() {
                        let cmd = editor.command_line().to_string();
                        if let Some(path_portion) = extract_path_from_command(&cmd) {
                            let prefix_len = cmd.len() - path_portion.len();
                            let new_cmd = format!("{}{}", &cmd[..prefix_len], new_path);
                            editor.set_command_line(&new_cmd);
                            let cwd = std::env::current_dir().unwrap_or_default();
                            editor.path_completion_mut().update(&new_path, &cwd);
                        }
                    }
                } else {
                    // File: accept into command line, then execute.
                    if let Some(new_path) = editor.path_completion().accept() {
                        let cmd = editor.command_line().to_string();
                        if let Some(path_portion) = extract_path_from_command(&cmd) {
                            let prefix_len = cmd.len() - path_portion.len();
                            let new_cmd = format!("{}{}", &cmd[..prefix_len], new_path);
                            editor.set_command_line(&new_cmd);
                        }
                    }
                    editor.path_completion_mut().hide();
                    editor.add_command_to_history();
                    execute_command(editor)?;
                    editor.clear_command_line();
                    if editor.mode() == Mode::Command {
                        editor.set_mode(Mode::Normal);
                    }
                }
            } else {
                editor.add_command_to_history();
                execute_command(editor)?;
                editor.clear_command_line();
                if editor.mode() == Mode::Command {
                    editor.set_mode(Mode::Normal);
                }
            }
        }
        KeyCode::Esc => {
            editor.path_completion_mut().hide();
            editor.clear_command_line();
            editor.set_mode(Mode::Normal);
        }
        _ => {}
    }
    Ok(())
}

/// Updates the path completion popup based on current command line content.
fn update_path_completion(editor: &mut Editor) {
    let cmd = editor.command_line().to_string();
    if let Some(path_portion) = extract_path_from_command(&cmd) {
        let path_portion = path_portion.to_string();
        let cwd = std::env::current_dir().unwrap_or_default();
        editor.path_completion_mut().update(&path_portion, &cwd);
    } else {
        editor.path_completion_mut().hide();
    }
}

/// State for command name Tab completion cycling.
struct CmdCompletion {
    matches: Vec<&'static str>,
    index: usize,
    original_prefix: String,
}

thread_local! {
    static CMD_COMPLETION: std::cell::RefCell<Option<CmdCompletion>> = const { std::cell::RefCell::new(None) };
}

/// Handles Tab (forward=false) or BackTab (backward=true) for path completion.
fn handle_tab_completion(editor: &mut Editor, backward: bool) {
    let cmd = editor.command_line().to_string();
    let trimmed = cmd.trim_start();

    // If the command line has no space, we're completing a command name.
    if !trimmed.contains(' ') {
        handle_command_name_completion(editor, trimmed, backward);
        return;
    }

    // Otherwise, fall through to path completion.
    if !editor.path_completion().is_visible() {
        // Try to trigger completion from current command line.
        update_path_completion(editor);
        if !editor.path_completion().is_visible() {
            return;
        }
        // First Tab after triggering: accept entry[0] without cycling.
    } else if editor.path_completion().tab_accepted() {
        // Already visible and Tab was used before — cycle selection.
        if backward {
            editor.path_completion_mut().select_previous();
        } else {
            editor.path_completion_mut().select_next();
        }
    }
    // else: popup was visible from typing but Tab hasn't accepted yet —
    // accept the current selection (entry[0]) without cycling.

    editor.path_completion_mut().set_tab_accepted();

    // Accept the selected entry: replace path portion in command line.
    let accepted = editor.path_completion().accept();
    if let Some(new_path) = accepted {
        let cmd = editor.command_line().to_string();
        if let Some(path_portion) = extract_path_from_command(&cmd) {
            let prefix_len = cmd.len() - path_portion.len();
            let new_cmd = format!("{}{}", &cmd[..prefix_len], new_path);
            editor.set_command_line(&new_cmd);

            // If we just completed a directory, refresh entries for its contents.
            if new_path.ends_with('/') {
                let cwd = std::env::current_dir().unwrap_or_default();
                editor.path_completion_mut().update(&new_path, &cwd);
            }
        }
    }
}

/// Handle Tab completion for command names (first word of command line).
fn handle_command_name_completion(editor: &mut Editor, prefix: &str, backward: bool) {
    CMD_COMPLETION.with(|cell| {
        let mut state = cell.borrow_mut();

        // Check if we're continuing a previous completion cycle
        let continuing = state.as_ref().is_some_and(|s| {
            s.original_prefix == prefix || {
                // Also continue if the command line matches a previous completion result
                s.matches.contains(&prefix)
            }
        });

        if continuing {
            let s = state.as_mut().unwrap();
            if backward {
                s.index = if s.index == 0 {
                    s.matches.len() // wraps to original prefix
                } else {
                    s.index - 1
                };
            } else {
                s.index += 1;
                if s.index > s.matches.len() {
                    s.index = 0;
                }
            }
            // index == matches.len() means show original prefix
            if s.index == s.matches.len() {
                editor.set_command_line(&s.original_prefix);
            } else {
                editor.set_command_line(s.matches[s.index]);
            }
        } else {
            // Build new completion list
            let matches: Vec<&'static str> = crate::commands::command_names()
                .iter()
                .map(String::as_str)
                .filter(|name| name.starts_with(prefix))
                .collect();

            if matches.is_empty() {
                *state = None;
                return;
            }

            if matches.len() == 1 {
                // Unique match — complete it directly
                editor.set_command_line(matches[0]);
                *state = None;
                return;
            }

            // Multiple matches — start cycling from first
            let idx = if backward { matches.len() - 1 } else { 0 };
            editor.set_command_line(matches[idx]);
            *state = Some(CmdCompletion {
                matches,
                index: idx,
                original_prefix: prefix.to_string(),
            });
        }
    });
}

/// Accepts the currently selected path completion entry and updates the command line text.
fn accept_selected_into_command_line(editor: &mut Editor) {
    if let Some(new_path) = editor.path_completion().accept() {
        let cmd = editor.command_line().to_string();
        if let Some(path_portion) = extract_path_from_command(&cmd) {
            let prefix_len = cmd.len() - path_portion.len();
            let new_cmd = format!("{}{}", &cmd[..prefix_len], new_path);
            editor.set_command_line(&new_cmd);
        }
    }
}

/// Runs a command line on behalf of the user (the `:` prompt, keymaps, Lua)
/// and shows its outcome; stores it in the `":` register afterwards, as vim
/// does.
pub fn execute_command_string(editor: &mut Editor, command: &str) -> Result<()> {
    crate::commands::execute_and_show(editor, command);
    editor
        .registers_mut()
        .set_last_command(command.trim().to_string());
    Ok(())
}

/// Executes the command line being edited. The previous message goes away
/// as in vim, so a command that reports nothing does not leave a stale one.
fn execute_command(editor: &mut Editor) -> Result<()> {
    let command = editor.command_line().trim().to_string();
    editor.clear_status_message();
    execute_command_string(editor, &command)
}
