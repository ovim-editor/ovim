use crate::ai::chat_types::{
    ChatFocus, ChatMessage, ChatOpts, ChatRole, ConversationTree, NodeId, StreamChunk,
    ToolCallInfo, ToolSummaryKind,
};
use crate::buffer::BufferId;
use crate::mode::Mode;
use anyhow::Result;

use super::ai_chat_state::{AiChatState, ChatViewMode, ScratchBufferState, ToolEventSummary};
use super::Editor;

impl Editor {
    // -----------------------------------------------------------------
    // Open / Close
    // -----------------------------------------------------------------

    /// Open or resume an AI chat panel.
    pub fn open_ai_chat(&mut self, opts: ChatOpts) -> Result<()> {
        let buffer_id = self.buffer().id();
        let mode_before = self.mode();

        // Ensure conversation exists
        let key = (buffer_id, opts.name.clone());
        if !self.ai_state.conversations.contains_key(&key) {
            self.ai_state
                .conversations
                .insert(key, ConversationTree::new());
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
        chat.viewport.row_scroll_from_bottom = 0;
        chat.viewport.follow_latest = true;
        chat.viewport.pinned_base_total_rows = None;
        chat.history.selected_node_id = None;
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

    /// Effective profile currently used by the active chat session.
    pub fn ai_chat_effective_profile(&self) -> String {
        self.ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.opts.profile.clone())
            .unwrap_or_else(|| self.ai_state.active_profile.clone())
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

    /// Whether an AI turn still has pending work that can affect review flow.
    pub fn ai_chat_has_pending_work(&self) -> bool {
        self.ai_chat_waiting()
            || self.ai_chat_has_pending_tool_approval()
            || self.ai_chat_has_pending_no_repo_folder_approval()
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
            "Tool approval requested: {} ({})",
            pending.tool_call.name,
            pending.requested_path.display()
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

    /// Human-readable save policy for AI chat mutations.
    pub fn ai_chat_save_policy_label(&self) -> Option<&'static str> {
        self.ai_state
            .chat
            .as_ref()
            .map(|_| "only_if_clean_at_start")
    }

    /// Effective save mode for current AI target buffer.
    pub fn ai_chat_save_mode_label(&self) -> Option<&'static str> {
        let chat = self.ai_state.chat.as_ref()?;
        let has_path = self
            .get_buffer_by_id(chat.active_buffer_id)
            .and_then(|b| b.file_path())
            .is_some();
        if !has_path {
            return Some("unsaved-buffer");
        }
        if chat.buffer_was_clean_at_chat_start {
            Some("auto")
        } else {
            Some("manual")
        }
    }

    /// Most recent save outcome message for this chat session.
    pub fn ai_chat_last_save_outcome(&self) -> Option<&str> {
        self.ai_state
            .chat
            .as_ref()
            .and_then(|c| c.last_save_outcome.as_deref())
    }

    /// Selected message index in current conversation.
    pub fn ai_chat_history_selected_index(&self) -> Option<usize> {
        let conv = self.conversation()?;
        let node_ids = conv.node_ids_for_active_branch();
        if node_ids.is_empty() {
            return None;
        }
        let selected = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|c| c.history.selected_node_id);
        if let Some(sel) = selected {
            if let Some(idx) = node_ids.iter().position(|id| *id == sel) {
                return Some(idx);
            }
        }
        Some(node_ids.len() - 1)
    }

    /// Whether history selection currently points at latest message.
    pub fn ai_chat_history_is_latest_selected(&self) -> bool {
        let Some(idx) = self.ai_chat_history_selected_index() else {
            return true;
        };
        let len = self.ai_chat_messages().len();
        idx + 1 >= len
    }

    /// Effective row scroll offset for rendering given row count and viewport size.
    pub fn ai_chat_effective_message_scroll(
        &self,
        total_rows: usize,
        visible_rows: usize,
    ) -> usize {
        let Some(chat) = self.ai_state.chat.as_ref() else {
            return 0;
        };
        let viewport = &chat.viewport;
        if viewport.follow_latest || viewport.row_scroll_from_bottom == 0 {
            return 0;
        }
        let base = viewport.pinned_base_total_rows.unwrap_or(total_rows);
        let growth = total_rows.saturating_sub(base);
        let max_scroll = total_rows.saturating_sub(visible_rows);
        viewport
            .row_scroll_from_bottom
            .saturating_add(growth)
            .min(max_scroll)
    }

    /// Scroll chat history viewport toward older rows.
    pub fn ai_chat_scroll_viewport_up(&mut self, rows: usize) {
        if rows == 0 {
            return;
        }
        let baseline_rows = self.render_cache.ai_chat_last_total_rows;
        let baseline = if baseline_rows == 0 {
            None
        } else {
            Some(baseline_rows)
        };
        if let Some(chat) = self.ai_state.chat.as_mut() {
            if chat.viewport.follow_latest {
                chat.viewport.follow_latest = false;
                chat.viewport.pinned_base_total_rows = baseline;
            } else if chat.viewport.pinned_base_total_rows.is_none() {
                chat.viewport.pinned_base_total_rows = baseline;
            }
            chat.viewport.row_scroll_from_bottom =
                chat.viewport.row_scroll_from_bottom.saturating_add(rows);
        }
    }

    /// Scroll chat history viewport toward latest rows.
    ///
    /// Returns true when the viewport reached bottom/latest.
    pub fn ai_chat_scroll_viewport_down(&mut self, rows: usize) -> bool {
        if rows == 0 {
            return self
                .ai_state
                .chat
                .as_ref()
                .is_some_and(|c| c.viewport.row_scroll_from_bottom == 0);
        }
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.viewport.row_scroll_from_bottom =
                chat.viewport.row_scroll_from_bottom.saturating_sub(rows);
            if chat.viewport.row_scroll_from_bottom == 0 {
                chat.viewport.follow_latest = true;
                chat.viewport.pinned_base_total_rows = None;
                return true;
            }
        }
        false
    }

    /// Move message-history selection toward older messages.
    pub fn ai_chat_history_cursor_move_older(&mut self, messages: usize) {
        if messages == 0 {
            return;
        }
        let target_id = {
            let Some(conv) = self.conversation() else {
                return;
            };
            let node_ids = conv.node_ids_for_active_branch();
            if node_ids.is_empty() {
                return;
            }
            let current = self
                .ai_chat_history_selected_index()
                .unwrap_or(node_ids.len() - 1);
            let target = current.saturating_sub(messages);
            node_ids.get(target).copied()
        };
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.history.selected_node_id = target_id;
        }
        self.ai_chat_history_ensure_cursor_visible();
    }

    /// Move message-history selection toward latest messages.
    ///
    /// Returns true when selection reached the latest message.
    pub fn ai_chat_history_cursor_move_newer(&mut self, messages: usize) -> bool {
        if messages == 0 {
            return self.ai_chat_history_is_latest_selected();
        }
        let target_id = {
            let Some(conv) = self.conversation() else {
                return true;
            };
            let node_ids = conv.node_ids_for_active_branch();
            if node_ids.is_empty() {
                return true;
            }
            let current = self
                .ai_chat_history_selected_index()
                .unwrap_or(node_ids.len() - 1);
            let target = (current + messages).min(node_ids.len() - 1);
            node_ids.get(target).copied()
        };
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.history.selected_node_id = target_id;
        }
        self.ai_chat_history_ensure_cursor_visible();
        self.ai_chat_history_is_latest_selected()
    }

    fn ai_chat_history_ensure_cursor_visible(&mut self) {
        let Some(selected_idx) = self.ai_chat_history_selected_index() else {
            return;
        };
        let Some(&(msg_start, msg_end)) = self
            .render_cache
            .ai_chat_last_message_row_spans
            .get(selected_idx)
        else {
            return;
        };
        let vis_start = self.render_cache.ai_chat_last_visible_start_row;
        let vis_end = self.render_cache.ai_chat_last_visible_end_row;
        if vis_end <= vis_start {
            return;
        }

        if msg_end <= vis_start {
            let delta = vis_start.saturating_sub(msg_start).max(1);
            self.ai_chat_scroll_viewport_up(delta);
        } else if msg_start >= vis_end {
            let delta = msg_end.saturating_sub(vis_end).max(1);
            self.ai_chat_scroll_viewport_down(delta);
        }
    }

    /// Ensure history selection references the latest message.
    pub fn ai_chat_reset_history_cursor(&mut self) {
        let latest = self
            .conversation()
            .and_then(|c| c.node_ids_for_active_branch().last().copied());
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.history.selected_node_id = latest;
        }
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

    /// Compact summary metadata for a completed tool call.
    pub fn ai_chat_tool_event_summary(&self, tool_call_id: &str) -> Option<&ToolEventSummary> {
        self.ai_state
            .chat
            .as_ref()
            .and_then(|c| c.tool_event_summaries.get(tool_call_id))
    }

    /// Convenience accessor for renderer callsites.
    pub fn ai_chat_tool_event_summary_parts(
        &self,
        tool_call_id: &str,
    ) -> Option<(ToolSummaryKind, &str)> {
        self.ai_chat_tool_event_summary(tool_call_id)
            .map(|s| (s.kind, s.label.as_str()))
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
        let buffer_id = self.buffer().id();
        let cursor_line = self.buffer().cursor().line();

        let target = self.ai_state.chat.as_ref().and_then(|c| {
            c.agent_edits
                .next_edit_boundary(buffer_id, cursor_line, forward)
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
            chat.view_mode = ChatViewMode::DockedChat;
        }
        self.set_lsp_status("Accepted AI changes and returned to chat".to_string());
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

    pub(crate) fn ai_chat_conversation_key(&self) -> (BufferId, String) {
        if let Some(chat) = &self.ai_state.chat {
            (chat.origin_buffer_id, chat.opts.name.clone())
        } else {
            (self.buffer().id(), "chat".to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat_types::ChatOpts;
    use crate::buffer::Buffer;

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
    fn history_selection_tracks_node_identity_across_appends() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);

        {
            let conv = editor.conversation_mut().expect("conversation");
            conv.append_user_message("u1".to_string());
            conv.append_assistant_message("a1".to_string(), "m".to_string());
            conv.append_user_message("u2".to_string());
        }

        editor.ai_chat_reset_history_cursor();
        editor.ai_chat_history_cursor_move_older(1); // select a1

        let idx_before = editor
            .ai_chat_history_selected_index()
            .expect("selected index");
        assert_eq!(editor.ai_chat_messages()[idx_before].content, "a1");

        {
            let conv = editor.conversation_mut().expect("conversation");
            conv.append_assistant_message("a2".to_string(), "m".to_string());
        }

        let idx_after = editor
            .ai_chat_history_selected_index()
            .expect("selected index");
        assert_eq!(editor.ai_chat_messages()[idx_after].content, "a1");
    }

    #[test]
    fn history_cursor_visibility_scrolls_viewport_when_selection_offscreen() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);

        {
            let conv = editor.conversation_mut().expect("conversation");
            conv.append_user_message("u1".to_string());
            conv.append_assistant_message("a1".to_string(), "m".to_string());
            conv.append_user_message("u2".to_string());
            conv.append_assistant_message("a2".to_string(), "m".to_string());
        }

        editor.render_cache.ai_chat_last_total_rows = 8;
        editor.render_cache.ai_chat_last_visible_start_row = 6;
        editor.render_cache.ai_chat_last_visible_end_row = 8;
        editor.render_cache.ai_chat_last_message_row_spans = vec![(0, 2), (2, 4), (4, 6), (6, 8)];

        editor.ai_chat_reset_history_cursor(); // latest (a2)
        editor.ai_chat_history_cursor_move_older(2); // target a1, above visible region

        let chat = editor.ai_state.chat.as_ref().expect("chat");
        assert!(!chat.viewport.follow_latest);
        assert_eq!(chat.viewport.pinned_base_total_rows, Some(8));
        assert!(chat.viewport.row_scroll_from_bottom > 0);
    }

    #[test]
    fn history_selection_falls_back_to_latest_when_node_leaves_branch() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);

        let root_id;
        {
            let conv = editor.conversation_mut().expect("conversation");
            conv.append_user_message("u1".to_string());
            root_id = conv.node_ids_for_active_branch()[0];
            conv.append_assistant_message("a1".to_string(), "m".to_string());
            conv.append_user_message("u2".to_string());
        }

        editor.ai_chat_reset_history_cursor();
        editor.ai_chat_history_cursor_move_older(1); // select a1 on original branch

        {
            let conv = editor.conversation_mut().expect("conversation");
            conv.fork_from(root_id);
            conv.append_assistant_message("alt".to_string(), "m".to_string());
        }

        let idx = editor
            .ai_chat_history_selected_index()
            .expect("selected index");
        assert_eq!(editor.ai_chat_messages()[idx].content, "alt");
    }

    #[test]
    fn chat_view_mode_toggles_between_docked_and_review() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);

        assert!(!editor.ai_chat_review_mode());
        editor.ai_chat_enter_review_mode();
        assert!(editor.ai_chat_review_mode());
        editor.ai_chat_exit_review_mode();
        assert!(!editor.ai_chat_review_mode());
    }

    #[test]
    fn accept_review_clears_markers_and_returns_to_docked_chat() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let buffer_id = editor.buffer().id();

        editor.ai_chat_enter_review_mode();
        {
            let chat = editor.ai_state.chat.as_mut().expect("chat");
            chat.agent_edits.record_edit(buffer_id, 0, 0);
            assert_eq!(chat.agent_edits.total_edit_count(), 1);
        }

        editor.ai_chat_accept_review();

        assert!(!editor.ai_chat_review_mode());
        let edits = editor
            .ai_state
            .chat
            .as_ref()
            .expect("chat")
            .agent_edits
            .total_edit_count();
        assert_eq!(edits, 0);
    }

    #[test]
    fn effective_message_scroll_is_clamped_to_viewport_window() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);

        if let Some(chat) = editor.ai_state.chat.as_mut() {
            chat.viewport.follow_latest = false;
            chat.viewport.row_scroll_from_bottom = 10_000;
            chat.viewport.pinned_base_total_rows = Some(50);
        }

        // With 50 rows and a viewport of 12, max safe scroll is 38.
        let effective = editor.ai_chat_effective_message_scroll(50, 12);
        assert_eq!(effective, 38);
    }

    #[test]
    fn conversation_history_survives_buffer_index_shift() {
        let mut editor = Editor::default();

        // Seed two buffers so deleting one will shift indices.
        editor.add_buffer(Buffer::new_from_str("second\n"));
        open_test_chat(&mut editor);

        {
            let conv = editor.conversation_mut().expect("conversation");
            conv.append_user_message("hello".to_string());
        }

        // Delete the first buffer so the chat buffer index changes.
        editor.switch_to_buffer(0);
        let should_quit = editor.delete_current_buffer();
        assert!(!should_quit);

        // Conversation should still resolve through stable BufferId keying.
        let messages = editor.ai_chat_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "hello");
    }
}
