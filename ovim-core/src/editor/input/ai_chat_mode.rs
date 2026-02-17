use crate::ai::chat_types::{ChatFocus, ChatRole};
use crate::editor::Editor;
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;
use std::time::Duration;

const DOUBLE_ESC_THRESHOLD: Duration = Duration::from_millis(300);

pub fn handle_ai_chat_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    // --- Review mode: delegate most keys to normal mode ---
    let review_mode = editor
        .ai_state
        .chat
        .as_ref()
        .map(|c| c.review_mode)
        .unwrap_or(false);

    if review_mode {
        // <C-r> toggles back to chat
        if key_event.code == KeyCode::Char('r') && key_event.modifiers.contains(Modifiers::CONTROL)
        {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.review_mode = false;
            }
            return Ok(());
        }
        // Esc closes chat entirely from review mode
        if key_event.code == KeyCode::Esc {
            editor.close_ai_chat();
            return Ok(());
        }
        // Everything else delegates to normal mode handling
        return super::normal::handle_normal_mode(editor, key_event);
    }

    let focus = editor.ai_chat_focus();

    // --- Global keys (all zones) ---
    if key_event.code == KeyCode::Esc {
        return handle_escape(editor, focus);
    }

    if key_event.code == KeyCode::Char('c') && key_event.modifiers.contains(Modifiers::CONTROL) {
        editor.close_ai_chat();
        return Ok(());
    }

    if editor.ai_chat_has_pending_no_repo_folder_approval() {
        if key_event.code == KeyCode::Char('y') && key_event.modifiers.contains(Modifiers::CONTROL)
        {
            editor.ai_chat_resolve_pending_no_repo_folder_approval(true);
            return Ok(());
        }
        if key_event.code == KeyCode::Char('a') && key_event.modifiers.contains(Modifiers::CONTROL)
        {
            editor.ai_chat_resolve_pending_no_repo_folder_approval(true);
            return Ok(());
        }
        if key_event.code == KeyCode::Char('n') && key_event.modifiers.contains(Modifiers::CONTROL)
        {
            editor.ai_chat_resolve_pending_no_repo_folder_approval(false);
            return Ok(());
        }
        return Ok(());
    }

    if editor.ai_chat_has_pending_tool_approval() {
        if key_event.code == KeyCode::Char('y') && key_event.modifiers.contains(Modifiers::CONTROL)
        {
            editor.ai_chat_resolve_pending_tool_approval(true, false);
            return Ok(());
        }
        if key_event.code == KeyCode::Char('a') && key_event.modifiers.contains(Modifiers::CONTROL)
        {
            editor.ai_chat_resolve_pending_tool_approval(true, true);
            return Ok(());
        }
        if key_event.code == KeyCode::Char('n') && key_event.modifiers.contains(Modifiers::CONTROL)
        {
            editor.ai_chat_resolve_pending_tool_approval(false, false);
            return Ok(());
        }
        return Ok(());
    }

    // <C-r> toggles review mode
    if key_event.code == KeyCode::Char('r') && key_event.modifiers.contains(Modifiers::CONTROL) {
        if let Some(chat) = editor.ai_state.chat.as_mut() {
            chat.review_mode = true;
        }
        return Ok(());
    }

    // <C-y> copies conversation to clipboard (unless used for pending tool approval)
    if key_event.code == KeyCode::Char('y') && key_event.modifiers.contains(Modifiers::CONTROL) {
        editor.copy_ai_chat_conversation();
        return Ok(());
    }

    // <C-t> toggles tree panel from any focus zone
    if key_event.code == KeyCode::Char('t') && key_event.modifiers.contains(Modifiers::CONTROL) {
        if let Some(chat) = editor.ai_state.chat.as_mut() {
            chat.tree_panel_open = !chat.tree_panel_open;
            if chat.tree_panel_open {
                chat.focus = ChatFocus::TreePanel;
            } else if chat.focus == ChatFocus::TreePanel {
                chat.focus = ChatFocus::TextInput;
            }
        }
        return Ok(());
    }

    match focus {
        ChatFocus::TextInput => handle_text_input(editor, key_event),
        ChatFocus::MessageHistory => handle_message_history(editor, key_event),
        ChatFocus::ModelSelector => handle_model_selector(editor, key_event),
        ChatFocus::TreePanel => handle_tree_panel(editor, key_event),
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
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                let (line, _total) = cursor_line_info(&chat.input, chat.input_cursor);
                if line == 0 {
                    // First line — navigate to message history
                    chat.focus = ChatFocus::MessageHistory;
                } else {
                    chat.input_cursor = move_cursor_vertical(&chat.input, chat.input_cursor, -1);
                }
            }
        }
        KeyCode::Down => {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                let (line, total) = cursor_line_info(&chat.input, chat.input_cursor);
                if line >= total - 1 {
                    // Last line — navigate to model selector
                    chat.focus = ChatFocus::ModelSelector;
                } else {
                    chat.input_cursor = move_cursor_vertical(&chat.input, chat.input_cursor, 1);
                }
            }
        }
        KeyCode::Enter if key_event.modifiers.contains(Modifiers::SHIFT) => {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.input.insert(chat.input_cursor, '\n');
                chat.input_cursor += 1;
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
        // Ctrl-U — scroll up half page
        KeyCode::Char('u') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.message_scroll = chat.message_scroll.saturating_add(10);
            }
        }
        // Ctrl-D — scroll down half page
        KeyCode::Char('d') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.message_scroll = chat.message_scroll.saturating_sub(10);
            }
        }
        KeyCode::Enter => {
            let node_ids = editor
                .conversation()
                .map(|c| c.node_ids_for_active_branch().to_vec())
                .unwrap_or_default();
            let messages = editor.ai_chat_messages();
            let scroll = editor.ai_chat_message_scroll();
            let idx = messages.len().saturating_sub(1 + scroll);

            if idx < messages.len() && idx < node_ids.len() {
                let node_id = node_ids[idx];
                let role = messages[idx].role.clone();

                if role == ChatRole::Thinking {
                    // Toggle thinking expand/collapse
                    if let Some(chat) = editor.ai_state.chat.as_mut() {
                        if !chat.expanded_thinking.remove(&node_id) {
                            chat.expanded_thinking.insert(node_id);
                        }
                    }
                } else if role == ChatRole::User {
                    // Fork: set active_leaf to parent of this user message
                    let parent_id = editor
                        .conversation()
                        .and_then(|c| c.node(node_id))
                        .and_then(|n| n.parent);
                    if let Some(pid) = parent_id {
                        if let Some(conv) = editor.conversation_mut() {
                            conv.fork_from(pid);
                        }
                    }
                    // Return to text input for the new message
                    if let Some(chat) = editor.ai_state.chat.as_mut() {
                        chat.focus = ChatFocus::TextInput;
                    }
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

fn handle_tree_panel(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        KeyCode::Down | KeyCode::Char('j') => {
            let total = tree_panel_node_count(editor);
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                if total > 0 {
                    chat.tree_panel_cursor = (chat.tree_panel_cursor + 1).min(total - 1);
                }
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.tree_panel_cursor = chat.tree_panel_cursor.saturating_sub(1);
            }
        }
        KeyCode::Enter => {
            let node_id = tree_panel_selected_node_id(editor);
            if let Some(id) = node_id {
                if let Some(conv) = editor.conversation_mut() {
                    conv.switch_to_branch(id);
                }
                if let Some(chat) = editor.ai_state.chat.as_mut() {
                    chat.focus = ChatFocus::TextInput;
                }
            }
        }
        KeyCode::Char('q') => {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.tree_panel_open = false;
                chat.focus = ChatFocus::TextInput;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Count total nodes for tree panel navigation.
fn tree_panel_node_count(editor: &Editor) -> usize {
    editor
        .conversation()
        .map(|c| c.all_nodes().len())
        .unwrap_or(0)
}

/// Get the NodeId at the current tree panel cursor position.
/// Uses DFS order from root.
fn tree_panel_selected_node_id(editor: &Editor) -> Option<u64> {
    let conv = editor.conversation()?;
    let root_id = conv.root_id()?;
    let cursor = editor.ai_chat_tree_panel_cursor();
    let mut dfs_order = Vec::new();
    dfs_collect(conv, root_id, &mut dfs_order);
    dfs_order.get(cursor).copied()
}

fn dfs_collect(conv: &crate::ai::chat_types::ConversationTree, node_id: u64, out: &mut Vec<u64>) {
    out.push(node_id);
    if let Some(node) = conv.node(node_id) {
        for &child_id in &node.children {
            dfs_collect(conv, child_id, out);
        }
    }
}

/// Returns (current_line_index, total_lines) for the cursor position in input.
fn cursor_line_info(input: &str, cursor: usize) -> (usize, usize) {
    let cursor = cursor.min(input.len());
    let line = input[..cursor].matches('\n').count();
    let total = input.matches('\n').count() + 1;
    (line, total)
}

/// Move cursor up (direction=-1) or down (direction=1) within multi-line input.
/// Tries to maintain column offset. Returns the new byte offset.
fn move_cursor_vertical(input: &str, cursor: usize, direction: i8) -> usize {
    let cursor = cursor.min(input.len());

    // Find start of current line and column offset
    let line_start = input[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col: usize = input[line_start..cursor].chars().count();

    if direction < 0 {
        // Move up: find the previous line
        if line_start == 0 {
            return cursor; // already on first line
        }
        let prev_line_end = line_start - 1; // the '\n' before current line
        let prev_line_start = input[..prev_line_end]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let prev_line = &input[prev_line_start..prev_line_end];
        let prev_line_chars = prev_line.chars().count();
        let target_col = col.min(prev_line_chars);
        // Convert char offset to byte offset
        let byte_offset: usize = prev_line
            .chars()
            .take(target_col)
            .map(|c| c.len_utf8())
            .sum();
        prev_line_start + byte_offset
    } else {
        // Move down: find the next line
        let next_newline = input[cursor..].find('\n');
        match next_newline {
            None => cursor, // already on last line
            Some(rel) => {
                let next_line_start = cursor + rel + 1;
                let next_line_end = input[next_line_start..]
                    .find('\n')
                    .map(|i| next_line_start + i)
                    .unwrap_or(input.len());
                let next_line = &input[next_line_start..next_line_end];
                let next_line_chars = next_line.chars().count();
                let target_col = col.min(next_line_chars);
                let byte_offset: usize = next_line
                    .chars()
                    .take(target_col)
                    .map(|c| c.len_utf8())
                    .sum();
                next_line_start + byte_offset
            }
        }
    }
}
