use crate::unicode::GraphemeCol;

use super::ai_chat_state::ChatViewMode;
use super::Editor;

impl Editor {
    /// Jump to the next/previous agent edit in the current buffer.
    pub fn goto_agent_edit(&mut self, forward: bool) {
        let buffer_id = self.buffer().id();
        let cursor_line = self.buffer().cursor().line();

        let target = self.ai_state.chat.as_ref().and_then(|c| {
            c.agent_edits
                .next_edit_boundary(buffer_id, cursor_line, forward)
        });

        if let Some(line) = target {
            self.buffer_mut()
                .cursor_mut()
                .set_position(line, GraphemeCol(0));
            self.center_cursor_in_viewport();
        }
    }

    /// Accept the current review session and return to chat.
    ///
    /// This keeps file edits on disk/in-memory, but clears per-turn review
    /// markers so the next review session starts fresh.
    pub fn ai_chat_accept_review(&mut self) {
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.agent_edits.clear();
            chat.view_mode = ChatViewMode::DockedChat;
        }
        self.set_status_message("Accepted AI changes and returned to chat".to_string());
    }

    /// Whether review mode is active.
    pub fn ai_chat_review_mode(&self) -> bool {
        self.ai_state
            .chat
            .as_ref()
            .map(|c| c.view_mode == ChatViewMode::ReviewFocused)
            .unwrap_or(false)
    }

    /// Enter edits-focused review mode (chat hidden).
    pub fn ai_chat_enter_review_mode(&mut self) {
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.view_mode = ChatViewMode::ReviewFocused;
        }
    }

    /// Return to docked chat mode (chat visible).
    pub fn ai_chat_exit_review_mode(&mut self) {
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.view_mode = ChatViewMode::DockedChat;
        }
    }

    /// Whether the tree panel sidebar is open.
    pub fn ai_chat_tree_panel_open(&self) -> bool {
        self.ai_state
            .chat
            .as_ref()
            .map(|c| c.tree_panel_open)
            .unwrap_or(false)
    }

    /// Cursor position in the tree panel.
    pub fn ai_chat_tree_panel_cursor(&self) -> usize {
        self.ai_state
            .chat
            .as_ref()
            .map(|c| c.tree_panel_cursor)
            .unwrap_or(0)
    }
}
