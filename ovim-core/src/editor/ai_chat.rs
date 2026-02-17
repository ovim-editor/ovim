use crate::ai::chat_types::{
    ChatFocus, ChatMessage, ChatOpts, ChatRole, ConversationTree, NodeId, StreamChunk, ToolCallInfo,
};
use crate::mode::Mode;
use anyhow::Result;

use super::ai_chat_state::{AiChatState, ScratchBufferState};
use super::Editor;

impl Editor {
    // -----------------------------------------------------------------
    // Open / Close
    // -----------------------------------------------------------------

    /// Open or resume an AI chat panel.
    pub fn open_ai_chat(&mut self, opts: ChatOpts) -> Result<()> {
        let buffer_id = self.current_buffer_index;
        let mode_before = self.mode();

        // Ensure conversation exists
        let key = (buffer_id, opts.name.clone());
        if !self.ai_state.conversations.contains_key(&key) {
            self.ai_state
                .conversations
                .insert(key, ConversationTree::new());
        }

        // Set profile override if specified
        if let Some(ref profile) = opts.profile {
            if self.ai_state.config.resolve_profile(profile).is_some() {
                self.ai_state.active_profile = profile.clone();
            }
        }

        // Send initial message if provided and conversation is empty
        let initial = opts.initial_message.clone();
        let buffer_clean = !self.buffer().is_modified();
        let mut chat = AiChatState::new(opts, buffer_id, mode_before);
        chat.buffer_was_clean_at_chat_start = buffer_clean;
        self.ai_state.chat = Some(chat);
        self.set_mode(Mode::AiChat);
        self.maybe_prompt_no_repo_session_folder_access_on_chat_open();

        if let Some(msg) = initial {
            if let Some(conv) = self.conversation() {
                if conv.is_empty() && !msg.is_empty() {
                    // Will be handled as if user typed and submitted
                    if let Some(chat) = self.ai_state.chat.as_mut() {
                        chat.input = msg;
                        chat.input_cursor = chat.input.len();
                    }
                }
            }
        }

        Ok(())
    }

    /// Close the AI chat panel, preserving conversation history.
    pub fn close_ai_chat(&mut self) {
        if let Some(chat) = self.ai_state.chat.take() {
            self.set_mode(chat.mode_before_chat);
        }
    }

    // -----------------------------------------------------------------
    // Submit
    // -----------------------------------------------------------------

    /// Submit the current chat input as a user message and spawn the AI request.
    pub fn submit_ai_chat_message(&mut self) -> Result<()> {
        let chat = match self.ai_state.chat.as_mut() {
            Some(c) => c,
            None => return Ok(()),
        };

        let input = chat.input.trim().to_string();
        if input.is_empty() || chat.waiting {
            return Ok(());
        }

        // Append user message to conversation
        if let Some(conv) = self.conversation_mut() {
            conv.append_user_message(input.clone());
        }

        // Clear input and mark as waiting
        let chat = self.ai_state.chat.as_mut().unwrap();
        chat.input.clear();
        chat.input_cursor = 0;
        chat.waiting = true;
        chat.message_scroll = 0;
        chat.message_follow_latest = true;
        chat.message_scroll_base_total_rows = None;
        chat.tool_call_count = 0;
        chat.pending_tool_approval = None;

        // Spawn the streaming request
        if let Err(e) = self.spawn_streaming_request() {
            if let Some(conv) = self.conversation_mut() {
                conv.append_error(e.to_string());
            }
            if let Some(chat) = self.ai_state.chat.as_mut() {
                chat.waiting = false;
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------
    // Poll
    // -----------------------------------------------------------------

    /// Drain available streaming chunks. Returns true if state changed.
    pub fn poll_pending_ai_chat_job(&mut self) -> bool {
        let chat = match self.ai_state.chat.as_mut() {
            Some(c) => c,
            None => return false,
        };

        let job = match chat.pending_job.as_mut() {
            Some(j) => j,
            None => return false,
        };

        // Phase 1: Drain all available chunks into a local vec.
        let mut chunks = Vec::new();
        let mut disconnected = false;
        loop {
            match job.receiver.try_recv() {
                Ok(chunk) => chunks.push(chunk),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        if chunks.is_empty() && !disconnected {
            return false;
        }

        // Extract model_name before processing.
        let model_name = chat
            .pending_job
            .as_ref()
            .map(|j| j.model_name.clone())
            .unwrap_or_default();

        // Phase 2: Process collected chunks.
        let mut changed = false;
        for chunk in chunks {
            match chunk {
                StreamChunk::Content(text) => {
                    if let Some(chat) = self.ai_state.chat.as_mut() {
                        if let Some(ref mut s) = chat.streaming_content {
                            s.push_str(&text);
                        }
                    }
                    changed = true;
                }
                StreamChunk::Thinking(text) => {
                    if let Some(chat) = self.ai_state.chat.as_mut() {
                        match chat.streaming_thinking.as_mut() {
                            Some(s) => s.push_str(&text),
                            None => chat.streaming_thinking = Some(text),
                        }
                    }
                    changed = true;
                }
                StreamChunk::ToolCallComplete {
                    id,
                    name,
                    arguments,
                } => {
                    if let Some(chat) = self.ai_state.chat.as_mut() {
                        chat.streaming_tool_calls.push(ToolCallInfo {
                            id,
                            name,
                            arguments,
                        });
                    }
                    changed = true;
                }
                StreamChunk::Done => {
                    // Commit thinking (if any) as a Thinking message.
                    let thinking = self
                        .ai_state
                        .chat
                        .as_mut()
                        .and_then(|c| c.streaming_thinking.take());
                    if let Some(thinking_text) = thinking {
                        if !thinking_text.is_empty() {
                            if let Some(conv) = self.conversation_mut() {
                                conv.append_thinking_message(thinking_text, model_name.clone());
                            }
                        }
                    }

                    // Take tool calls and content
                    let tool_calls = self
                        .ai_state
                        .chat
                        .as_mut()
                        .map(|c| std::mem::take(&mut c.streaming_tool_calls))
                        .unwrap_or_default();
                    let content = self
                        .ai_state
                        .chat
                        .as_mut()
                        .and_then(|c| c.streaming_content.take())
                        .unwrap_or_default();

                    if !tool_calls.is_empty() {
                        return self.process_tool_calls(tool_calls, content, &model_name);
                    }

                    // No tool calls — normal text-only commit
                    if !content.is_empty() {
                        if let Some(conv) = self.conversation_mut() {
                            conv.append_assistant_message(content, model_name.clone());
                        }
                    }

                    // Clear undo group (agent turn is done)
                    if let Some(chat) = self.ai_state.chat.as_mut() {
                        chat.current_undo_group = None;
                    }

                    self.clear_streaming_state();
                    return true;
                }
                StreamChunk::Error(msg) => {
                    self.commit_partial_streaming(&model_name);

                    // Append the error.
                    if let Some(conv) = self.conversation_mut() {
                        conv.append_error(msg);
                    }

                    self.clear_streaming_state();
                    return true;
                }
                StreamChunk::ToolCall { .. } => {
                    // Progressive tool call updates — currently just wait for ToolCallComplete
                }
            }
        }

        // Handle channel disconnected without Done (task crashed/cancelled).
        if disconnected {
            let content = self
                .ai_state
                .chat
                .as_mut()
                .and_then(|c| c.streaming_content.take());
            if let Some(content_text) = content {
                if !content_text.is_empty() {
                    if let Some(conv) = self.conversation_mut() {
                        conv.append_assistant_message(content_text, model_name.clone());
                    }
                }
            }
            if let Some(conv) = self.conversation_mut() {
                conv.append_error("Stream interrupted".to_string());
            }
            self.clear_streaming_state();
            return true;
        }

        changed
    }

    /// Commit any partial thinking/content that was streaming when an error occurred.
    fn commit_partial_streaming(&mut self, model_name: &str) {
        let thinking = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|c| c.streaming_thinking.take());
        if let Some(thinking_text) = thinking {
            if !thinking_text.is_empty() {
                if let Some(conv) = self.conversation_mut() {
                    conv.append_thinking_message(thinking_text, model_name.to_string());
                }
            }
        }

        let content = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|c| c.streaming_content.take());
        if let Some(content_text) = content {
            if !content_text.is_empty() {
                if let Some(conv) = self.conversation_mut() {
                    conv.append_assistant_message(content_text, model_name.to_string());
                }
            }
        }
    }

    // -----------------------------------------------------------------
    // Context profile
    // -----------------------------------------------------------------

    pub fn ai_chat_context_profile(&self, context: &str) -> Option<String> {
        // Look up in contexts table first
        if let Some(profile) = self.ai_state.config.contexts.get(context) {
            if self.ai_state.config.profiles.contains_key(profile) {
                return Some(profile.clone());
            }
        }
        // Fallback to active profile
        Some(self.ai_state.active_profile.clone())
    }

    // -----------------------------------------------------------------
    // Scratch buffer (<C-g>)
    // -----------------------------------------------------------------

    /// Create a scratch buffer from the chat input for editing in Normal mode.
    pub fn open_chat_scratch_editor(&mut self) {
        let chat = match self.ai_state.chat.as_mut() {
            Some(c) => c,
            None => return,
        };

        let original_input = chat.input.clone();
        let original_buffer_index = self.current_buffer_index;

        // Create a new buffer with the current input
        let mut buffer = crate::buffer::Buffer::default();
        buffer.replace_all(&original_input);
        self.buffers.push(buffer);
        let scratch_index = self.buffers.len() - 1;
        self.current_buffer_index = scratch_index;

        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.scratch = Some(ScratchBufferState {
                scratch_buffer_index: scratch_index,
                original_buffer_index,
                original_input,
            });
        }

        self.set_mode(Mode::Normal);
    }

    /// Close the scratch buffer and optionally transfer content back to chat input.
    pub fn finish_chat_scratch(&mut self, send: bool) -> Result<()> {
        let chat = match self.ai_state.chat.as_mut() {
            Some(c) => c,
            None => return Ok(()),
        };

        let scratch = match chat.scratch.take() {
            Some(s) => s,
            None => return Ok(()),
        };

        if send {
            // Transfer scratch buffer content back to chat input
            let content = self.buffers[scratch.scratch_buffer_index]
                .rope()
                .to_string();
            let trimmed = content.trim_end_matches('\n').to_string();
            if let Some(chat) = self.ai_state.chat.as_mut() {
                chat.input = trimmed;
                chat.input_cursor = chat.input.len();
            }
        } else {
            // Discard — restore original input
            if let Some(chat) = self.ai_state.chat.as_mut() {
                chat.input = scratch.original_input;
                chat.input_cursor = chat.input.len();
            }
        }

        // Remove scratch buffer
        if scratch.scratch_buffer_index < self.buffers.len() {
            self.buffers.remove(scratch.scratch_buffer_index);
        }
        self.current_buffer_index = scratch.original_buffer_index;
        self.set_mode(Mode::AiChat);

        Ok(())
    }

    /// Check if the current buffer is a chat scratch buffer.
    pub fn is_chat_scratch_buffer(&self) -> bool {
        if let Some(chat) = &self.ai_state.chat {
            if let Some(scratch) = &chat.scratch {
                return self.current_buffer_index == scratch.scratch_buffer_index;
            }
        }
        false
    }

    // -----------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------

    /// Get a reference to the active chat state.
    pub fn ai_chat_state(&self) -> Option<&AiChatState> {
        self.ai_state.chat.as_ref()
    }

    /// Get the messages for the current chat conversation.
    pub fn ai_chat_messages(&self) -> &[ChatMessage] {
        self.conversation().map(|c| c.messages()).unwrap_or(&[])
    }

    /// Get the current chat focus zone.
    pub fn ai_chat_focus(&self) -> ChatFocus {
        self.ai_state
            .chat
            .as_ref()
            .map(|c| c.focus)
            .unwrap_or(ChatFocus::TextInput)
    }

    /// Get chat input text.
    pub fn ai_chat_input(&self) -> &str {
        self.ai_state
            .chat
            .as_ref()
            .map(|c| c.input.as_str())
            .unwrap_or("")
    }

    /// Get chat input cursor position.
    pub fn ai_chat_input_cursor(&self) -> usize {
        self.ai_state
            .chat
            .as_ref()
            .map(|c| c.input_cursor)
            .unwrap_or(0)
    }

    /// Whether chat is waiting for a response.
    pub fn ai_chat_waiting(&self) -> bool {
        self.ai_state
            .chat
            .as_ref()
            .map(|c| c.waiting)
            .unwrap_or(false)
    }

    /// Whether a tool call is currently paused pending user approval.
    pub fn ai_chat_has_pending_tool_approval(&self) -> bool {
        self.ai_state
            .chat
            .as_ref()
            .map(|c| c.pending_tool_approval.is_some())
            .unwrap_or(false)
    }

    /// Whether chat is waiting for first-time no-repo folder approval.
    pub fn ai_chat_has_pending_no_repo_folder_approval(&self) -> bool {
        self.ai_state
            .chat
            .as_ref()
            .map(|c| c.pending_no_repo_folder_approval.is_some())
            .unwrap_or(false)
    }

    /// Human-readable summary of the pending no-repo folder approval, if any.
    pub fn ai_chat_pending_no_repo_folder_approval_summary(&self) -> Option<String> {
        let pending = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|c| c.pending_no_repo_folder_approval.as_ref())?;
        Some(format!(
            "Not in a git repo. Allow tool access to folder: {}",
            pending.display()
        ))
    }

    /// Human-readable summary of the pending approval, if any.
    pub fn ai_chat_pending_tool_approval_summary(&self) -> Option<String> {
        let pending = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|c| c.pending_tool_approval.as_ref())?;
        Some(format!(
            "Outside-project access requested: {} ({})",
            pending.requested_path.display(),
            pending.tool_call.name
        ))
    }

    /// Whether chat allows edits.
    pub fn ai_chat_allow_edits(&self) -> bool {
        self.ai_state
            .chat
            .as_ref()
            .map(|c| c.allow_edits)
            .unwrap_or(true)
    }

    /// Message scroll offset.
    pub fn ai_chat_message_scroll(&self) -> usize {
        self.ai_state
            .chat
            .as_ref()
            .map(|c| c.message_scroll)
            .unwrap_or(0)
    }

    /// Streaming content being accumulated (not yet committed).
    pub fn ai_chat_streaming_content(&self) -> Option<&str> {
        self.ai_state
            .chat
            .as_ref()
            .and_then(|c| c.streaming_content.as_deref())
    }

    /// Streaming thinking being accumulated (not yet committed).
    pub fn ai_chat_streaming_thinking(&self) -> Option<&str> {
        self.ai_state
            .chat
            .as_ref()
            .and_then(|c| c.streaming_thinking.as_deref())
    }

    /// Whether tokens are actively streaming in.
    pub fn ai_chat_is_streaming(&self) -> bool {
        self.ai_state
            .chat
            .as_ref()
            .map(|c| c.waiting && c.streaming_content.is_some())
            .unwrap_or(false)
    }

    /// Whether a thinking message with the given node ID is expanded.
    pub fn ai_chat_is_thinking_expanded(&self, node_id: NodeId) -> bool {
        self.ai_state
            .chat
            .as_ref()
            .map(|c| c.expanded_thinking.contains(&node_id))
            .unwrap_or(false)
    }

    /// Jump to the next/previous agent edit in the current buffer.
    pub fn goto_agent_edit(&mut self, forward: bool) {
        let buf_idx = self.current_buffer_index;
        let cursor_line = self.buffer().cursor().line();

        let target = self.ai_state.chat.as_ref().and_then(|c| {
            c.agent_edits
                .next_edit_boundary(buf_idx, cursor_line, forward)
        });

        if let Some(line) = target {
            self.buffer_mut().cursor_mut().set_position(line, 0);
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
            chat.review_mode = false;
        }
        self.set_lsp_status("Accepted AI changes and returned to chat".to_string());
    }

    /// Whether review mode is active.
    pub fn ai_chat_review_mode(&self) -> bool {
        self.ai_state
            .chat
            .as_ref()
            .map(|c| c.review_mode)
            .unwrap_or(false)
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

    // -----------------------------------------------------------------
    // Copy conversation
    // -----------------------------------------------------------------

    /// Format the current conversation as plain text and copy to clipboard.
    pub fn copy_ai_chat_conversation(&mut self) {
        let messages = self.ai_chat_messages().to_vec();
        if messages.is_empty() {
            return;
        }

        let mut output = String::new();
        for msg in &messages {
            let role = match msg.role {
                ChatRole::User => "You",
                ChatRole::Assistant => msg.model.as_deref().unwrap_or("Assistant"),
                ChatRole::Thinking => "Thinking",
                ChatRole::Error => "Error",
                ChatRole::Tool => "Tool",
            };
            output.push_str(&format!("### {}\n\n{}\n\n", role, msg.content));
        }

        self.registers.set_clipboard(output);
    }

    // -----------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------

    pub(crate) fn ai_chat_conversation_key(&self) -> (usize, String) {
        if let Some(chat) = &self.ai_state.chat {
            (chat.origin_buffer_id, chat.opts.name.clone())
        } else {
            (self.current_buffer_index, "chat".to_string())
        }
    }

    /// Shorthand for getting the current conversation (read-only).
    pub fn conversation(&self) -> Option<&ConversationTree> {
        let key = self.ai_chat_conversation_key();
        self.ai_state.conversations.get(&key)
    }

    /// Shorthand for getting the current conversation (mutable).
    pub(crate) fn conversation_mut(&mut self) -> Option<&mut ConversationTree> {
        let key = self.ai_chat_conversation_key();
        self.ai_state.conversations.get_mut(&key)
    }
}
