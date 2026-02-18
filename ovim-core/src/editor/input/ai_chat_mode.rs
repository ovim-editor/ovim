use crate::ai::chat_types::{ChatFocus, ChatRole};
use crate::editor::Editor;
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;
use std::time::Duration;

const DOUBLE_ESC_THRESHOLD: Duration = Duration::from_millis(300);

pub fn handle_ai_chat_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    // --- Review mode: explicit allowlist only (no normal-mode delegation) ---
    let review_mode = editor.ai_chat_review_mode();

    if review_mode {
        let pending_work = editor.ai_chat_has_pending_work();

        // <C-r> toggles back to chat
        if key_event.code == KeyCode::Char('r') && key_event.modifiers.contains(Modifiers::CONTROL)
        {
            editor.ai_chat_exit_review_mode();
            return Ok(());
        }
        if key_event.code == KeyCode::Left {
            editor.goto_agent_edit(false);
            return Ok(());
        }
        if key_event.code == KeyCode::Right {
            editor.goto_agent_edit(true);
            return Ok(());
        }
        if key_event.code == KeyCode::Enter {
            if pending_work {
                editor.set_lsp_status(
                    "AI work is still pending. Wait before accepting review.".to_string(),
                );
                return Ok(());
            }
            editor.ai_chat_accept_review();
            return Ok(());
        }
        // Esc closes chat entirely from review mode
        if key_event.code == KeyCode::Esc {
            if pending_work {
                editor.set_lsp_status(
                    "AI work is still pending. Wait before closing chat.".to_string(),
                );
                return Ok(());
            }
            editor.close_ai_chat();
            return Ok(());
        }
        if key_event.code == KeyCode::PageUp {
            editor.scroll_page_up();
            return Ok(());
        }
        if key_event.code == KeyCode::PageDown {
            editor.scroll_page_down();
            return Ok(());
        }
        return Ok(());
    }

    if editor.ai_chat_has_pending_no_repo_folder_approval() {
        if key_event.code == KeyCode::Enter
            || (key_event.code == KeyCode::Char('y')
                && key_event.modifiers.contains(Modifiers::CONTROL))
            || (key_event.code == KeyCode::Char('a')
                && key_event.modifiers.contains(Modifiers::CONTROL))
        {
            editor.ai_chat_resolve_pending_no_repo_folder_approval(true);
            return Ok(());
        }
        if key_event.code == KeyCode::Esc
            || (key_event.code == KeyCode::Char('n')
                && key_event.modifiers.contains(Modifiers::CONTROL))
        {
            editor.ai_chat_resolve_pending_no_repo_folder_approval(false);
            return Ok(());
        }
        return Ok(());
    }

    if editor.ai_chat_has_pending_tool_approval() {
        if key_event.code == KeyCode::Enter
            || (key_event.code == KeyCode::Char('y')
                && key_event.modifiers.contains(Modifiers::CONTROL))
        {
            editor.ai_chat_resolve_pending_tool_approval(true, false);
            return Ok(());
        }
        if key_event.code == KeyCode::Char('a') && key_event.modifiers.contains(Modifiers::CONTROL)
        {
            editor.ai_chat_resolve_pending_tool_approval(true, true);
            return Ok(());
        }
        if key_event.code == KeyCode::Esc
            || (key_event.code == KeyCode::Char('n')
                && key_event.modifiers.contains(Modifiers::CONTROL))
        {
            editor.ai_chat_resolve_pending_tool_approval(false, false);
            return Ok(());
        }
        return Ok(());
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

    // Scroll buffer viewport while chat is open.
    if key_event.code == KeyCode::PageUp {
        editor.scroll_page_up();
        return Ok(());
    }
    if key_event.code == KeyCode::PageDown {
        editor.scroll_page_down();
        return Ok(());
    }

    // <C-r> toggles review mode
    if key_event.code == KeyCode::Char('r') && key_event.modifiers.contains(Modifiers::CONTROL) {
        editor.ai_chat_enter_review_mode();
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
            let mut moved_to_history = false;
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                let (line, _total) = cursor_line_info(&chat.input, chat.input_cursor);
                if line == 0 {
                    // First line — navigate to message history
                    chat.focus = ChatFocus::MessageHistory;
                    moved_to_history = true;
                } else {
                    chat.input_cursor = move_cursor_vertical(&chat.input, chat.input_cursor, -1);
                }
            }
            if moved_to_history {
                editor.ai_chat_reset_history_cursor();
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
            editor.ai_chat_history_cursor_move_older(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let at_latest = editor.ai_chat_history_is_latest_selected();
            if at_latest {
                if editor.ai_chat_scroll_viewport_down(1) {
                    if let Some(chat) = editor.ai_state.chat.as_mut() {
                        chat.focus = ChatFocus::TextInput;
                    }
                }
            } else {
                editor.ai_chat_history_cursor_move_newer(1);
            }
        }
        // Ctrl-U — scroll up half page
        KeyCode::Char('u') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.ai_chat_scroll_viewport_up(10);
        }
        // Ctrl-D — scroll down half page
        KeyCode::Char('d') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.ai_chat_scroll_viewport_down(10);
        }
        KeyCode::Enter => {
            let node_ids = editor
                .conversation()
                .map(|c| c.node_ids_for_active_branch().to_vec())
                .unwrap_or_default();
            let messages = editor.ai_chat_messages();
            if let Some(idx) = editor.ai_chat_history_selected_index() {
                if idx >= messages.len() || idx >= node_ids.len() {
                    return Ok(());
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat_types::ChatOpts;
    use crate::ai::chat_types::ToolCallInfo;
    use std::path::PathBuf;

    fn open_test_chat(editor: &mut Editor) {
        editor
            .open_ai_chat(ChatOpts {
                name: "chat".to_string(),
                allow_edits: true,
                ..Default::default()
            })
            .expect("open chat");
    }

    #[test]
    fn review_mode_ignores_unmapped_keys() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        editor.ai_chat_enter_review_mode();

        handle_ai_chat_mode(
            &mut editor,
            KeyEvent::new(KeyCode::Char('i'), Modifiers::NONE),
        )
        .expect("handle key");

        assert!(editor.ai_chat_review_mode());
        assert_eq!(editor.mode(), crate::mode::Mode::AiChat);
    }

    #[test]
    fn review_mode_blocks_accept_and_close_while_pending() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let buffer_id = editor.buffer().id();
        editor.ai_chat_enter_review_mode();
        if let Some(chat) = editor.ai_state.chat.as_mut() {
            chat.waiting = true;
            chat.agent_edits.record_edit(buffer_id, 0, 0);
        }

        handle_ai_chat_mode(&mut editor, KeyEvent::new(KeyCode::Enter, Modifiers::NONE))
            .expect("enter");
        handle_ai_chat_mode(&mut editor, KeyEvent::new(KeyCode::Esc, Modifiers::NONE))
            .expect("esc");

        let chat = editor.ai_state.chat.as_ref().expect("chat");
        assert!(chat.view_mode == crate::editor::ai_chat_state::ChatViewMode::ReviewFocused);
        assert_eq!(chat.agent_edits.total_edit_count(), 1);
    }

    #[test]
    fn pending_folder_approval_accepts_enter_and_denies_esc() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        if let Some(chat) = editor.ai_state.chat.as_mut() {
            chat.pending_no_repo_folder_approval = Some(PathBuf::from("/tmp/demo"));
        }

        handle_ai_chat_mode(&mut editor, KeyEvent::new(KeyCode::Enter, Modifiers::NONE))
            .expect("enter");
        assert!(!editor.ai_chat_has_pending_no_repo_folder_approval());

        if let Some(chat) = editor.ai_state.chat.as_mut() {
            chat.pending_no_repo_folder_approval = Some(PathBuf::from("/tmp/demo"));
        }
        handle_ai_chat_mode(&mut editor, KeyEvent::new(KeyCode::Esc, Modifiers::NONE))
            .expect("esc");
        assert!(!editor.ai_chat_has_pending_no_repo_folder_approval());
    }

    #[test]
    fn pending_tool_approval_accepts_enter_and_denies_esc() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let mut editor = Editor::default();
            open_test_chat(&mut editor);
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.pending_tool_approval =
                    Some(crate::editor::ai_chat_state::PendingToolApproval {
                        tool_call: ToolCallInfo {
                            id: "call1".to_string(),
                            name: "read_file".to_string(),
                            arguments: serde_json::json!({}),
                        },
                        remaining_tool_calls: Vec::new(),
                        model_name: "test-model".to_string(),
                        requested_path: PathBuf::from("/tmp/demo.txt"),
                        approval_root: PathBuf::from("/tmp"),
                    });
            }

            handle_ai_chat_mode(&mut editor, KeyEvent::new(KeyCode::Enter, Modifiers::NONE))
                .expect("enter");
            assert!(!editor.ai_chat_has_pending_tool_approval());

            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.pending_tool_approval =
                    Some(crate::editor::ai_chat_state::PendingToolApproval {
                        tool_call: ToolCallInfo {
                            id: "call2".to_string(),
                            name: "read_file".to_string(),
                            arguments: serde_json::json!({}),
                        },
                        remaining_tool_calls: Vec::new(),
                        model_name: "test-model".to_string(),
                        requested_path: PathBuf::from("/tmp/demo.txt"),
                        approval_root: PathBuf::from("/tmp"),
                    });
            }
            handle_ai_chat_mode(&mut editor, KeyEvent::new(KeyCode::Esc, Modifiers::NONE))
                .expect("esc");
            assert!(!editor.ai_chat_has_pending_tool_approval());
        });
    }
}
