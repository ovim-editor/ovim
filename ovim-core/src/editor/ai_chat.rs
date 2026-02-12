use crate::ai::chat_types::{ChatFocus, ChatMessage, ChatOpts, ConversationTree, StreamChunk};
use crate::ai::stream_ai_chat;
use crate::mode::Mode;
use anyhow::Result;

use super::ai_chat_state::{AiChatState, PendingAiChatJob, ScratchBufferState};
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
        let chat = AiChatState::new(opts, buffer_id, mode_before);
        self.ai_state.chat = Some(chat);
        self.set_mode(Mode::AiChat);

        if let Some(msg) = initial {
            let key = self.ai_chat_conversation_key();
            if let Some(conv) = self.ai_state.conversations.get(&key) {
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
        let key = self.ai_chat_conversation_key();
        if let Some(conv) = self.ai_state.conversations.get_mut(&key) {
            conv.append_user_message(input.clone());
        }

        // Clear input
        let chat = self.ai_state.chat.as_mut().unwrap();
        chat.input.clear();
        chat.input_cursor = 0;
        chat.waiting = true;
        chat.message_scroll = 0;

        // Resolve profile
        let profile_name = chat
            .opts
            .profile
            .clone()
            .unwrap_or_else(|| self.ai_state.active_profile.clone());
        let profile = match self.ai_state.config.resolve_profile(&profile_name) {
            Some(p) => p.clone(),
            None => {
                // No valid profile — record error and bail
                if let Some(conv) = self.ai_state.conversations.get_mut(&key) {
                    conv.append_error(format!("No AI profile '{}' configured", profile_name));
                }
                if let Some(chat) = self.ai_state.chat.as_mut() {
                    chat.waiting = false;
                }
                return Ok(());
            }
        };

        let model_name = profile.model.clone();
        let system_prompt = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|c| c.opts.system_prompt.clone());

        // Collect messages for the API call
        let messages: Vec<ChatMessage> = self
            .ai_state
            .conversations
            .get(&key)
            .map(|c| c.messages().to_vec())
            .unwrap_or_default();

        // Spawn streaming async task
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let tx_err = tx.clone();
        let task = tokio::spawn(async move {
            if let Err(e) =
                stream_ai_chat(&profile, &messages, system_prompt.as_deref(), tx.clone()).await
            {
                let _ = tx_err.send(StreamChunk::Error(e.to_string()));
            }
        });

        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.pending_job = Some(PendingAiChatJob {
                receiver: rx,
                task,
                profile_name: profile_name.clone(),
                model_name,
            });
            chat.streaming_content = Some(String::new());
            chat.streaming_thinking = None;
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

        // Extract model_name and conversation key before processing.
        let model_name = chat
            .pending_job
            .as_ref()
            .map(|j| j.model_name.clone())
            .unwrap_or_default();
        let key = self.ai_chat_conversation_key();

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
                StreamChunk::Done => {
                    // Commit thinking (if any) as a Thinking message.
                    let thinking = self
                        .ai_state
                        .chat
                        .as_mut()
                        .and_then(|c| c.streaming_thinking.take());
                    if let Some(thinking_text) = thinking {
                        if !thinking_text.is_empty() {
                            if let Some(conv) = self.ai_state.conversations.get_mut(&key) {
                                conv.append_thinking_message(thinking_text, model_name.clone());
                            }
                        }
                    }

                    // Commit content as an Assistant message.
                    let content = self
                        .ai_state
                        .chat
                        .as_mut()
                        .and_then(|c| c.streaming_content.take());
                    if let Some(content_text) = content {
                        if !content_text.is_empty() {
                            if let Some(conv) = self.ai_state.conversations.get_mut(&key) {
                                conv.append_assistant_message(content_text, model_name.clone());
                            }
                        }
                    }

                    // Clear streaming state.
                    if let Some(chat) = self.ai_state.chat.as_mut() {
                        chat.waiting = false;
                        chat.pending_job = None;
                        chat.streaming_content = None;
                        chat.streaming_thinking = None;
                        chat.message_scroll = 0;
                    }
                    return true;
                }
                StreamChunk::Error(msg) => {
                    // Commit any partial thinking/content first.
                    let thinking = self
                        .ai_state
                        .chat
                        .as_mut()
                        .and_then(|c| c.streaming_thinking.take());
                    if let Some(thinking_text) = thinking {
                        if !thinking_text.is_empty() {
                            if let Some(conv) = self.ai_state.conversations.get_mut(&key) {
                                conv.append_thinking_message(thinking_text, model_name.clone());
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
                            if let Some(conv) = self.ai_state.conversations.get_mut(&key) {
                                conv.append_assistant_message(content_text, model_name.clone());
                            }
                        }
                    }

                    // Append the error.
                    if let Some(conv) = self.ai_state.conversations.get_mut(&key) {
                        conv.append_error(msg);
                    }

                    if let Some(chat) = self.ai_state.chat.as_mut() {
                        chat.waiting = false;
                        chat.pending_job = None;
                        chat.streaming_content = None;
                        chat.streaming_thinking = None;
                    }
                    return true;
                }
                StreamChunk::ToolCall { .. } | StreamChunk::ToolCallComplete { .. } => {
                    // M4 — ignored for now.
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
                    if let Some(conv) = self.ai_state.conversations.get_mut(&key) {
                        conv.append_assistant_message(content_text, model_name.clone());
                    }
                }
            }
            if let Some(conv) = self.ai_state.conversations.get_mut(&key) {
                conv.append_error("Stream interrupted".to_string());
            }
            if let Some(chat) = self.ai_state.chat.as_mut() {
                chat.waiting = false;
                chat.pending_job = None;
                chat.streaming_content = None;
                chat.streaming_thinking = None;
            }
            return true;
        }

        changed
    }

    // -----------------------------------------------------------------
    // Context profile (M1: just returns active_profile)
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
                original_buffer_index: original_buffer_index,
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
        let key = self.ai_chat_conversation_key();
        self.ai_state
            .conversations
            .get(&key)
            .map(|c| c.messages())
            .unwrap_or(&[])
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

    /// Whether a thinking message at the given index is expanded.
    pub fn ai_chat_is_thinking_expanded(&self, index: usize) -> bool {
        self.ai_state
            .chat
            .as_ref()
            .map(|c| c.expanded_thinking.contains(&index))
            .unwrap_or(false)
    }

    // -----------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------

    fn ai_chat_conversation_key(&self) -> (usize, String) {
        if let Some(chat) = &self.ai_state.chat {
            (chat.active_buffer_id, chat.opts.name.clone())
        } else {
            (self.current_buffer_index, "chat".to_string())
        }
    }
}
