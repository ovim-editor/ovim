use crate::ai::chat_types::ChatFocus;
use crate::editor::Editor;
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;
use std::time::Duration;

const DOUBLE_ESC_THRESHOLD: Duration = Duration::from_millis(300);

pub fn handle_ai_chat_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    let focus = editor.ai_chat_focus();

    // --- Global keys (all zones) ---
    if key_event.code == KeyCode::Esc {
        return handle_escape(editor, focus);
    }

    if key_event.code == KeyCode::Char('c') && key_event.modifiers.contains(Modifiers::CONTROL) {
        editor.close_ai_chat();
        return Ok(());
    }

    match focus {
        ChatFocus::TextInput => handle_text_input(editor, key_event),
        ChatFocus::MessageHistory => handle_message_history(editor, key_event),
        ChatFocus::ModelSelector => handle_model_selector(editor, key_event),
    }
}

fn handle_escape(editor: &mut Editor, focus: ChatFocus) -> Result<()> {
    if focus != ChatFocus::TextInput {
        // Return to text input
        if let Some(chat) = editor.ai_state.chat.as_mut() {
            chat.focus = ChatFocus::TextInput;
            chat.last_escape = Some(std::time::Instant::now());
        }
        return Ok(());
    }

    // Double-Esc detection for TextInput
    let now = std::time::Instant::now();
    let is_double = editor
        .ai_state
        .chat
        .as_ref()
        .and_then(|c| c.last_escape)
        .map(|last| now.duration_since(last) < DOUBLE_ESC_THRESHOLD)
        .unwrap_or(false);

    if is_double {
        editor.close_ai_chat();
    } else if let Some(chat) = editor.ai_state.chat.as_mut() {
        chat.last_escape = Some(now);
    }

    Ok(())
}

fn handle_text_input(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        KeyCode::Char(ch)
            if !key_event.modifiers.contains(Modifiers::CONTROL)
                && !key_event.modifiers.contains(Modifiers::ALT) =>
        {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                let pos = chat.input_cursor;
                chat.input.insert(pos, ch);
                chat.input_cursor = pos + ch.len_utf8();
            }
        }
        KeyCode::Backspace => {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                let pos = chat.input_cursor;
                if pos > 0 {
                    let prev = chat.input[..pos]
                        .char_indices()
                        .next_back()
                        .map(|(idx, _)| idx)
                        .unwrap_or(0);
                    chat.input.remove(prev);
                    chat.input_cursor = prev;
                }
            }
        }
        KeyCode::Delete => {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                let pos = chat.input_cursor;
                if pos < chat.input.len() {
                    chat.input.remove(pos);
                }
            }
        }
        KeyCode::Left => {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                let pos = chat.input_cursor;
                if pos > 0 {
                    let prev = chat.input[..pos]
                        .char_indices()
                        .next_back()
                        .map(|(idx, _)| idx)
                        .unwrap_or(0);
                    chat.input_cursor = prev;
                }
            }
        }
        KeyCode::Right => {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                let pos = chat.input_cursor;
                if pos < chat.input.len() {
                    let next = chat.input[pos..]
                        .char_indices()
                        .nth(1)
                        .map(|(idx, _)| pos + idx)
                        .unwrap_or(chat.input.len());
                    chat.input_cursor = next;
                }
            }
        }
        KeyCode::Home => {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.input_cursor = 0;
            }
        }
        KeyCode::End => {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.input_cursor = chat.input.len();
            }
        }
        KeyCode::Up => {
            // Navigate to message history
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.focus = ChatFocus::MessageHistory;
            }
        }
        KeyCode::Down => {
            // Navigate to model selector
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.focus = ChatFocus::ModelSelector;
            }
        }
        KeyCode::Enter => {
            editor.submit_ai_chat_message()?;
        }
        KeyCode::Char('g') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.open_chat_scratch_editor();
        }
        _ => {}
    }
    Ok(())
}

fn handle_message_history(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.message_scroll = chat.message_scroll.saturating_add(1);
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                if chat.message_scroll == 0 {
                    chat.focus = ChatFocus::TextInput;
                } else {
                    chat.message_scroll = chat.message_scroll.saturating_sub(1);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_model_selector(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        KeyCode::Left | KeyCode::Char('h') => {
            editor.ai_cycle_profile(false);
        }
        KeyCode::Right | KeyCode::Char('l') => {
            editor.ai_cycle_profile(true);
        }
        KeyCode::Up => {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.focus = ChatFocus::TextInput;
            }
        }
        _ => {}
    }
    Ok(())
}
