use crate::ai::chat_types::{
    ChatFocus, ChatMessage, ChatOpts, ChatRole, ConversationTree, NodeId, StreamChunk,
    ToolCallInfo, ToolSummaryKind,
};
use crate::buffer::BufferId;
use crate::display::char_display_width;
use crate::mode::Mode;
use crate::unicode::GraphemeCol;
use anyhow::Result;

use super::ai_chat_state::{AiChatState, ChatViewMode, ScratchBufferState, ToolEventSummary};
use super::Editor;

impl Editor {
    // -----------------------------------------------------------------
    // Open / Close
    // -----------------------------------------------------------------

    /// Open or resume an AI chat panel.
    pub fn open_ai_chat(&mut self, opts: ChatOpts) -> Result<()> {
        if self
            .ai_state
            .chat
            .as_ref()
            .is_some_and(|chat| chat.opts.name == opts.name)
        {
            let mode_before = self.mode();
            if let Some(chat) = self.ai_state.chat.as_mut() {
                chat.mode_before_chat = mode_before;
            }
            self.set_mode(Mode::AiChat);
            return Ok(());
        }

        // Switching to another named conversation replaces the live panel.
        // Its projected message history remains stored under its own key.
        if self.ai_state.chat.is_some() {
            self.discard_active_ai_chat("chat replaced");
        }
        let buffer_id = self.buffer().id();
        let mode_before = self.mode();

        if let Err(error) = self.prepare_durable_ai_chat(buffer_id, &opts.name) {
            self.set_lsp_status(format!(
                "Durable AI history unavailable; agent edits are disabled: {error}"
            ));
        }

        // Ensure conversation exists
        let key = (buffer_id, opts.name.clone());
        self.ai_state.conversations.entry(key.clone()).or_default();

        // Send initial message if provided and conversation is empty
        let initial = opts.initial_message.clone();
        let buffer_clean = !self.buffer().is_modified();
        let branch_generation = self
            .ai_state
            .conversations
            .get(&key)
            .map(ConversationTree::branch_generation)
            .unwrap_or_default();
        let mut chat = AiChatState::new(opts, buffer_id, mode_before);
        let runtime_locator = self
            .ai_state
            .durable_chat_bindings
            .get(&key)
            .map(|binding| binding.locator.clone())
            .unwrap_or_else(|| {
                crate::agent_runtime::ConversationLocator(format!(
                    "buffer:{buffer_id}:conversation:{}",
                    chat.opts.name
                ))
            });
        chat.runtime_branch = self
            .ai_state
            .agent_runtime
            .selected_branch(&runtime_locator)
            .map(|(locator, _)| locator.clone())
            .unwrap_or_else(|| {
                crate::agent_runtime::BranchLocator(format!("branch-{branch_generation}"))
            });
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

    /// Hide the AI chat panel without clearing or interrupting it.
    pub fn close_ai_chat(&mut self) {
        self.render_cache.ai_chat_text_selection = None;
        self.render_cache.ai_chat_text_selecting = false;
        let mode_before = self
            .ai_state
            .chat
            .as_ref()
            .map(|chat| chat.mode_before_chat);
        if let Some(mode) = mode_before {
            self.set_mode(mode);
        }
    }

    /// Stop the active AI round without hiding the panel or discarding chat state.
    pub fn cancel_ai_chat_generation(&mut self) -> bool {
        if !self.ai_chat_has_pending_work() {
            return false;
        }

        let model_name = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.pending_job.as_ref())
            .map(|job| job.model_name.clone())
            .unwrap_or_default();
        let had_agent_work = self.ai_state.chat.as_ref().is_some_and(|chat| {
            chat.waiting
                || chat.pending_job.is_some()
                || chat.pending_tool_approval.is_some()
                || chat.pending_auto_mode_classification.is_some()
                || chat.pending_shell_execution.is_some()
        });

        self.flush_ai_runtime_stream_segments();
        self.commit_partial_streaming(&model_name);

        let (pending_job, pending_approval, pending_classification, pending_shell) = {
            let chat = self.ai_state.chat.as_mut().expect("pending chat exists");
            (
                chat.pending_job.take(),
                chat.pending_tool_approval.take(),
                chat.pending_auto_mode_classification.take(),
                chat.pending_shell_execution.take(),
            )
        };

        if let Some(job) = pending_job {
            job.task.abort();
        }
        if let Some(pending) = pending_approval {
            if let (Some(turn), Some(tool)) =
                (pending.dynamic_turn.as_ref(), pending.runtime_tool.as_ref())
            {
                if let Err(error) =
                    self.ai_state
                        .agent_runtime
                        .fail_tool(turn, tool, "cancelled by user")
                {
                    crate::log_warn!("agent_runtime", "failed to cancel pending tool: {error}");
                }
            } else if let (Some(turn), Some(tool)) =
                (self.active_ai_runtime_turn(), pending.runtime_tool.as_ref())
            {
                if let Err(error) =
                    self.ai_state
                        .agent_runtime
                        .fail_tool(&turn, tool, "cancelled by user")
                {
                    crate::log_warn!("agent_runtime", "failed to cancel pending tool: {error}");
                }
            }
            if let Some(response) = pending.dynamic_response {
                let _ = response.send(Err("cancelled by user".into()));
            }
        }
        if let Some(pending) = pending_classification {
            if let Err(error) = self.ai_state.agent_runtime.fail_tool(
                &pending.runtime_turn,
                &pending.runtime_tool,
                "cancelled by user",
            ) {
                crate::log_warn!("agent_runtime", "failed to cancel classified tool: {error}");
            }
            let _ = pending
                .dynamic_response
                .send(Err("cancelled by user".into()));
        }
        if let Some(pending) = pending_shell {
            pending.task.abort();
            if let Err(error) = self.ai_state.agent_runtime.mark_tool_outcome_unknown(
                &pending.runtime_turn,
                &pending.runtime_tool,
                "cancelled by user before the shell result was observed",
            ) {
                crate::log_warn!("agent_runtime", "failed to cancel shell tool: {error}");
            }
            let _ = pending
                .dynamic_response
                .send(Err("cancelled by user".into()));
        }

        self.ai_runtime_interrupt_turn("cancelled by user");
        self.clear_streaming_state();
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.pending_no_repo_folder_approval = None;
        }
        if had_agent_work {
            if let Some(conv) = self.conversation_mut() {
                conv.append_error("Generation stopped by user.".to_string());
            }
            self.set_lsp_status("AI generation stopped".to_string());
        } else {
            self.set_lsp_status("AI folder access prompt cancelled".to_string());
        }
        true
    }

    /// Permanently discard the live panel state when replacing conversations.
    fn discard_active_ai_chat(&mut self, reason: &str) {
        if let Some(job) = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.pending_job.as_ref())
        {
            job.task.abort();
        }
        self.ai_runtime_interrupt_turn(reason);
        if let Some(mut chat) = self.ai_state.chat.take() {
            chat.pending_job.take();
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
        if input.is_empty() && chat.pending_images.is_empty() {
            return Ok(());
        }

        if chat.runtime_turn.is_some() {
            return self
                .queue_current_ai_chat_input(super::ai_chat_state::QueuedChatInputKind::Steer);
        }

        if chat.pending_images.is_empty() && self.try_execute_ai_chat_slash_command(&input)? {
            return Ok(());
        }

        let runtime_input = if input.is_empty() {
            "[Image attachment]".to_string()
        } else {
            input.clone()
        };

        // Allocate stable ovim run/agent/turn identity before provider work.
        let runtime_turn = self
            .begin_ai_runtime_turn(&runtime_input)
            .map_err(|error| anyhow::anyhow!("failed to start agent turn: {error}"))?;
        let user_event_id = runtime_turn.initiating_event.caused_by.clone();
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.runtime_turn = Some(Box::new(runtime_turn));
        }

        // Append user message to the UI projection.
        let images = self
            .ai_state
            .chat
            .as_mut()
            .map(|chat| std::mem::take(&mut chat.pending_images))
            .unwrap_or_default();
        let user_node = self
            .conversation_mut()
            .map(|conv| conv.append_user_message_with_images(input.clone(), images));
        if let (Some(node_id), Some(event_id)) = (user_node, user_event_id) {
            self.record_ai_chat_node(node_id, event_id);
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
            self.ai_runtime_fail_turn(e.to_string());
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
        if self.ai_state.chat.as_ref().is_some_and(|chat| {
            chat.waiting
                || chat
                    .pending_tool_approval
                    .as_ref()
                    .is_some_and(|pending| pending.dynamic_response.is_some())
                || chat.pending_auto_mode_classification.is_some()
                || chat.pending_shell_execution.is_some()
        }) {
            if let Err(error) = self.heartbeat_ai_chat_lease() {
                if self.fail_pending_shell_for_lost_lease(&error.to_string()) {
                    return true;
                }
                if let Some(job) = self
                    .ai_state
                    .chat
                    .as_mut()
                    .and_then(|chat| chat.pending_job.take())
                {
                    job.task.abort();
                }
                self.ai_runtime_interrupt_turn(format!("durable run lease lost: {error}"));
                if let Some(conversation) = self.conversation_mut() {
                    conversation.append_error(format!(
                        "Stopped agent because durable history ownership was lost: {error}"
                    ));
                }
                self.clear_streaming_state();
                return true;
            }
        }
        if self.poll_pending_auto_mode_classification() {
            return true;
        }
        if self.poll_pending_shell_execution() {
            return true;
        }
        let current_branch_generation = self
            .conversation()
            .map(ConversationTree::branch_generation)
            .unwrap_or_default();
        let pending_branch_generation = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.pending_job.as_ref())
            .map(|job| job.branch_generation);
        if pending_branch_generation
            .is_some_and(|generation| generation != current_branch_generation)
        {
            if let Some(job) = self
                .ai_state
                .chat
                .as_mut()
                .and_then(|chat| chat.pending_job.take())
            {
                job.task.abort();
            }
            self.ai_runtime_interrupt_turn("conversation branch changed during provider turn");
            if let Some(conv) = self.conversation_mut() {
                conv.append_error("Discarded stale response from a previous branch".into());
            }
            self.clear_streaming_state();
            return true;
        }

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
        let runtime_turn = chat.pending_job.as_ref().map(|job| (*job.turn).clone());

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
                StreamChunk::AgentMessageComplete => {
                    // Codex turns may contain multiple agentMessage items.
                    // Commit each completed item independently while leaving
                    // the turn in its working state for subsequent tools or
                    // messages.
                    self.flush_ai_runtime_stream_segments();
                    self.commit_partial_streaming(&model_name);
                    if let Some(chat) = self.ai_state.chat.as_mut() {
                        chat.streaming_content = Some(String::new());
                        chat.streaming_thinking = None;
                        chat.runtime_recorded_content_bytes = 0;
                        chat.runtime_recorded_thinking_bytes = 0;
                        chat.runtime_last_content_event = None;
                        chat.runtime_last_reasoning_event = None;
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
                StreamChunk::DynamicToolRequest { call, response } => {
                    self.flush_ai_runtime_stream_segments();
                    let Some(turn) = runtime_turn.as_ref() else {
                        let _ = response.send(Err("agent turn identity is missing".into()));
                        continue;
                    };
                    let tool = match self.ai_runtime_record_tool_intent(turn, &call) {
                        Ok(tool) => tool,
                        Err(error) => {
                            let message = format!("failed to record tool intent: {error}");
                            let _ = response.send(Err(message.clone()));
                            self.ai_runtime_fail_turn(message);
                            self.clear_streaming_state();
                            return true;
                        }
                    };
                    if call.name == "bash" {
                        if self.ai_state.config.tool_approval_mode
                            == crate::ai::ToolApprovalMode::Auto
                        {
                            self.begin_dynamic_bash_auto_mode(call, response, turn.clone(), tool);
                        } else {
                            self.pause_dynamic_tool_for_approval(
                                turn.clone(),
                                tool,
                                call,
                                response,
                                "configured approval policy requires confirmation".into(),
                            );
                        }
                        changed = true;
                        continue;
                    }
                    if let Err(error) = self.ai_runtime_start_tool(turn, &tool) {
                        let message = format!("failed to record tool start: {error}");
                        let _ = response.send(Err(message.clone()));
                        self.ai_runtime_fail_turn(message);
                        self.clear_streaming_state();
                        return true;
                    }
                    let outcome = self.dispatch_tool_call_with_approval(&call, None);
                    let result = match outcome {
                        super::ai_chat_tools::ToolDispatchOutcome::Completed(result) => result,
                        super::ai_chat_tools::ToolDispatchOutcome::ApprovalRequired(req) => {
                            crate::ai::tools::ToolResult::Error(format!(
                                "Tool requires user approval and was not executed: {}",
                                req.message
                            ))
                        }
                    };
                    if let Err(error) = self.ai_runtime_finish_tool(turn, &tool, &result) {
                        let message = format!("failed to record tool result: {error}");
                        let _ = response.send(Err(message.clone()));
                        self.ai_runtime_fail_turn(message);
                        self.clear_streaming_state();
                        return true;
                    }
                    self.record_tool_event_summary(&call, &result);
                    let result_content = self.format_tool_result_with_target(&call, &result);
                    if let Some(conv) = self.conversation_mut() {
                        conv.append_tool_result(call.id.clone(), result_content);
                    }
                    let wire_result = match &result {
                        crate::ai::tools::ToolResult::Success(text) => Ok(text.clone()),
                        crate::ai::tools::ToolResult::Error(text) => Err(text.clone()),
                    };
                    let _ = response.send(wire_result);
                    changed = true;
                }
                StreamChunk::SteerAccepted { content } => {
                    if let Err(error) = self.accept_provider_ai_chat_steer(content) {
                        self.set_lsp_status(format!("Failed to record accepted steer: {error}"));
                    }
                    changed = true;
                }
                StreamChunk::SteerRejected { content, error } => {
                    self.reject_provider_ai_chat_steer(&content, &error);
                    changed = true;
                }
                StreamChunk::Done => {
                    self.flush_ai_runtime_stream_segments();
                    // Commit thinking (if any) as a Thinking message.
                    let thinking = self
                        .ai_state
                        .chat
                        .as_mut()
                        .and_then(|c| c.streaming_thinking.take());
                    if let Some(thinking_text) = thinking {
                        if !thinking_text.is_empty() {
                            let event_id = self
                                .ai_state
                                .chat
                                .as_ref()
                                .and_then(|chat| chat.runtime_last_reasoning_event.clone());
                            let node_id = self.conversation_mut().map(|conv| {
                                conv.append_thinking_message(thinking_text, model_name.clone())
                            });
                            if let (Some(node_id), Some(event_id)) = (node_id, event_id) {
                                self.record_ai_chat_node(node_id, event_id);
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
                        // The visible message may contain text streamed before a
                        // dynamic tool. Anchor the node at the current causal tip
                        // so forking from it includes the observed tool result.
                        let event_id = self.ai_runtime_current_tip();
                        let node_id = self
                            .conversation_mut()
                            .map(|conv| conv.append_assistant_message(content, model_name.clone()));
                        if let (Some(node_id), Some(event_id)) = (node_id, event_id) {
                            self.record_ai_chat_node(node_id, event_id);
                        }
                    }

                    // Clear undo group (agent turn is done)
                    if let Some(chat) = self.ai_state.chat.as_mut() {
                        chat.current_undo_group = None;
                    }

                    self.ai_runtime_complete_turn();
                    self.clear_streaming_state();
                    if let Err(error) = self.start_next_queued_ai_chat_input() {
                        if let Some(conv) = self.conversation_mut() {
                            conv.append_error(format!("Failed to run queued input: {error}"));
                        }
                    }
                    return true;
                }
                StreamChunk::Error(msg) => {
                    self.flush_ai_runtime_stream_segments();
                    self.commit_partial_streaming(&model_name);

                    // Append the error.
                    if let Some(conv) = self.conversation_mut() {
                        conv.append_error(msg.clone());
                    }

                    self.ai_runtime_fail_turn(msg);
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
            self.flush_ai_runtime_stream_segments();
            let thinking = self
                .ai_state
                .chat
                .as_mut()
                .and_then(|c| c.streaming_thinking.take());
            if let Some(thinking_text) = thinking.filter(|text| !text.is_empty()) {
                let event_id = self
                    .ai_state
                    .chat
                    .as_ref()
                    .and_then(|chat| chat.runtime_last_reasoning_event.clone());
                let node_id = self
                    .conversation_mut()
                    .map(|conv| conv.append_thinking_message(thinking_text, model_name.clone()));
                if let (Some(node_id), Some(event_id)) = (node_id, event_id) {
                    self.record_ai_chat_node(node_id, event_id);
                }
            }
            let content = self
                .ai_state
                .chat
                .as_mut()
                .and_then(|c| c.streaming_content.take());
            if let Some(content_text) = content {
                if !content_text.is_empty() {
                    let event_id = self
                        .ai_state
                        .chat
                        .as_ref()
                        .and_then(|chat| chat.runtime_last_content_event.clone());
                    let node_id = self.conversation_mut().map(|conv| {
                        conv.append_assistant_message(content_text, model_name.clone())
                    });
                    if let (Some(node_id), Some(event_id)) = (node_id, event_id) {
                        self.record_ai_chat_node(node_id, event_id);
                    }
                }
            }
            if let Some(conv) = self.conversation_mut() {
                conv.append_error("Stream interrupted".to_string());
            }
            self.ai_runtime_interrupt_turn("provider stream disconnected");
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
                let event_id = self
                    .ai_state
                    .chat
                    .as_ref()
                    .and_then(|chat| chat.runtime_last_reasoning_event.clone());
                let node_id = self.conversation_mut().map(|conv| {
                    conv.append_thinking_message(thinking_text, model_name.to_string())
                });
                if let (Some(node_id), Some(event_id)) = (node_id, event_id) {
                    self.record_ai_chat_node(node_id, event_id);
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
                let event_id = self
                    .ai_state
                    .chat
                    .as_ref()
                    .and_then(|chat| chat.runtime_last_content_event.clone());
                let node_id = self.conversation_mut().map(|conv| {
                    conv.append_assistant_message(content_text, model_name.to_string())
                });
                if let (Some(node_id), Some(event_id)) = (node_id, event_id) {
                    self.record_ai_chat_node(node_id, event_id);
                }
            }
        }
    }

    fn shell_authorization_context(
        &self,
        project_root: &std::path::Path,
    ) -> crate::ai::auto_mode::ConversationAuthorizationContext {
        use crate::ai::auto_mode::{AuthorizedObjective, ExplicitAuthorization};

        let key = self.ai_chat_conversation_key();
        let runtime_nodes = self.ai_state.conversation_runtime_nodes.get(&key);
        let mut recent = self
            .conversation()
            .map(|conversation| {
                conversation
                    .messages()
                    .iter()
                    .zip(conversation.node_ids_for_active_branch())
                    .filter(|(message, _)| message.role == ChatRole::User)
                    .map(|(message, node_id)| {
                        let source_id = runtime_nodes
                            .and_then(|nodes| nodes.get(node_id))
                            .map(|reference| reference.event_id.as_str().to_string())
                            .unwrap_or_else(|| format!("ui-node:{node_id}"));
                        (message.content.clone(), source_id)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if recent.len() > 8 {
            recent.drain(..recent.len() - 8);
        }
        let explicit_user_instructions = recent
            .iter()
            .map(|(instruction, source_id)| ExplicitAuthorization {
                instruction: instruction.clone(),
                project_root: project_root.to_path_buf(),
                source_id: source_id.clone(),
            })
            .collect();
        let authorized_objectives = recent
            .last()
            .map(|(objective, source_id)| {
                vec![AuthorizedObjective {
                    objective: objective.clone(),
                    project_root: project_root.to_path_buf(),
                    source_id: source_id.clone(),
                }]
            })
            .unwrap_or_default();
        crate::ai::auto_mode::ConversationAuthorizationContext {
            explicit_user_instructions,
            authorized_objectives,
        }
    }

    fn begin_dynamic_bash_auto_mode(
        &mut self,
        call: ToolCallInfo,
        response: tokio::sync::oneshot::Sender<Result<String, String>>,
        turn: crate::agent_runtime::PendingTurnRef,
        tool: crate::agent_runtime::PendingToolRef,
    ) {
        use crate::ai::auto_classifier::{AutoModeClassifier, CodexAutoModeClassifier};
        use crate::ai::auto_mode::{ClassifierRequest, ShellProposal, StaticDisposition};
        use std::collections::BTreeSet;

        let command = call
            .arguments
            .get("command")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        let Some(project_root) = self.ai_effective_project_root() else {
            self.finish_dynamic_tool(
                &turn,
                &tool,
                &call,
                response,
                crate::ai::tools::ToolResult::Error(self.no_project_root_error()),
            );
            return;
        };
        let request = ClassifierRequest::new(
            ShellProposal {
                command,
                cwd: project_root.clone(),
                project_root: project_root.clone(),
                requested_capabilities: BTreeSet::new(),
            },
            self.shell_authorization_context(&project_root),
        );

        match request.dynamic.static_analysis.disposition {
            StaticDisposition::LocallySafe => {
                self.execute_dynamic_tool_after_policy(turn, tool, call, response);
            }
            StaticDisposition::UserConfirmationRequired => {
                self.pause_dynamic_tool_for_approval(
                    turn,
                    tool,
                    call,
                    response,
                    "static analysis requires explicit user confirmation".into(),
                );
            }
            StaticDisposition::ModelReviewRequired => {
                let operation_id = tool.operation_id.clone();
                let (result_tx, result_rx) = tokio::sync::oneshot::channel();
                tokio::spawn(async move {
                    let result = CodexAutoModeClassifier::default()
                        .classify(&request, &operation_id)
                        .await
                        .map_err(|error| format!("{error:#}"));
                    let _ = result_tx.send(result);
                });
                if let Some(chat) = self.ai_state.chat.as_mut() {
                    chat.pending_auto_mode_classification =
                        Some(super::ai_chat_state::PendingAutoModeClassification {
                            tool_call: call,
                            runtime_tool: tool,
                            runtime_turn: turn,
                            dynamic_response: response,
                            receiver: result_rx,
                        });
                }
                self.set_lsp_status("Luna is reviewing the proposed shell program".into());
            }
        }
    }

    fn poll_pending_auto_mode_classification(&mut self) -> bool {
        use crate::ai::auto_mode::ClassifierDecision;
        let received = {
            let Some(pending) = self
                .ai_state
                .chat
                .as_mut()
                .and_then(|chat| chat.pending_auto_mode_classification.as_mut())
            else {
                return false;
            };
            match pending.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return false,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    Some(Err("auto-mode classifier stopped without a verdict".into()))
                }
            }
        };
        let pending = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.pending_auto_mode_classification.take())
            .expect("pending classifier exists");
        let project_root = self.ai_effective_project_root();
        match received.expect("classifier result") {
            Ok(verdict)
                if verdict.decision == ClassifierDecision::Allow
                    && project_root.as_ref() == Some(&verdict.scope.project_root) =>
            {
                self.execute_dynamic_tool_after_policy(
                    pending.runtime_turn,
                    pending.runtime_tool,
                    pending.tool_call,
                    pending.dynamic_response,
                );
            }
            Ok(verdict) if verdict.decision == ClassifierDecision::Deny => {
                self.finish_dynamic_tool(
                    &pending.runtime_turn,
                    &pending.runtime_tool,
                    &pending.tool_call,
                    pending.dynamic_response,
                    crate::ai::tools::ToolResult::Error(format!(
                        "auto mode denied shell program: {}",
                        verdict.reason
                    )),
                );
            }
            Ok(verdict) => self.pause_dynamic_tool_for_approval(
                pending.runtime_turn,
                pending.runtime_tool,
                pending.tool_call,
                pending.dynamic_response,
                if verdict.decision == ClassifierDecision::Allow {
                    "classifier returned an Allow outside the active repository scope".into()
                } else {
                    verdict.reason
                },
            ),
            Err(error) => {
                crate::log_warn!("ai_auto_mode", "classifier unavailable: {error}");
                self.pause_dynamic_tool_for_approval(
                    pending.runtime_turn,
                    pending.runtime_tool,
                    pending.tool_call,
                    pending.dynamic_response,
                    format!("classifier unavailable; explicit confirmation required: {error}"),
                )
            }
        }
        true
    }

    pub(super) fn execute_dynamic_tool_after_policy(
        &mut self,
        turn: crate::agent_runtime::PendingTurnRef,
        tool: crate::agent_runtime::PendingToolRef,
        call: ToolCallInfo,
        response: tokio::sync::oneshot::Sender<Result<String, String>>,
    ) {
        if let Err(error) = self.heartbeat_ai_chat_lease() {
            let result = crate::ai::tools::ToolResult::Error(format!(
                "shell program was not executed because durable run ownership was lost: {error}"
            ));
            self.finish_dynamic_tool(&turn, &tool, &call, response, result);
            self.fail_specific_dynamic_turn(
                &turn,
                format!("durable run ownership was lost: {error}"),
            );
            if let Some(job) = self
                .ai_state
                .chat
                .as_mut()
                .and_then(|chat| chat.pending_job.take())
            {
                job.task.abort();
            }
            if let Some(chat) = self.ai_state.chat.as_mut() {
                chat.waiting = false;
            }
            self.set_lsp_status(format!(
                "Blocked agent effect because durable run ownership was lost: {error}"
            ));
            return;
        }
        if call.name == "bash" {
            let authorized =
                self.ai_state
                    .tool_registry
                    .get(&call.name)
                    .is_some_and(|definition| {
                        let capabilities = self.build_chat_capabilities();
                        capabilities.allows_side_effect(definition.side_effect)
                            && capabilities.contains(&definition.required_scope)
                    });
            if !authorized {
                self.finish_dynamic_tool(
                    &turn,
                    &tool,
                    &call,
                    response,
                    crate::ai::tools::ToolResult::Error(
                        "shell access is not authorized for this chat".into(),
                    ),
                );
                return;
            }
        }
        if let Err(error) = self.ai_runtime_start_tool(&turn, &tool) {
            let result = crate::ai::tools::ToolResult::Error(format!(
                "failed to durably record tool start: {error}"
            ));
            self.finish_dynamic_tool(&turn, &tool, &call, response, result);
            return;
        }
        if call.name == "bash" {
            let command = call
                .arguments
                .get("command")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            let Some(workdir) = self.ai_effective_project_root() else {
                self.finish_dynamic_tool(
                    &turn,
                    &tool,
                    &call,
                    response,
                    crate::ai::tools::ToolResult::Error(self.no_project_root_error()),
                );
                return;
            };
            let artifact_store = match self.ai_state.durable_runs.as_ref().and_then(|services| {
                services
                    .store
                    .layout()
                    .ensure_run_directory(&turn.run_id)
                    .map_err(|error| error.to_string())
                    .and_then(|_| {
                        crate::run_log::ArtifactStore::open(
                            services.store.layout().artifact_directory(&turn.run_id),
                        )
                        .map_err(|error| error.to_string())
                    })
                    .ok()
            }) {
                Some(store) => store,
                None => {
                    self.finish_dynamic_tool(
                        &turn,
                        &tool,
                        &call,
                        response,
                        crate::ai::tools::ToolResult::Error(
                            "shell program was not executed because replay artifact storage is unavailable".into(),
                        ),
                    );
                    return;
                }
            };
            let (result_tx, result_rx) = tokio::sync::oneshot::channel();
            let task = tokio::task::spawn_blocking(move || {
                let observation = match crate::run_log::capture_workspace(&workdir, &artifact_store) {
                    Ok(before) => {
                        let result = super::ai_tool_execution::run_bash_program(&command, &workdir);
                        match crate::run_log::capture_workspace(&workdir, &artifact_store) {
                            Ok(after) => super::ai_chat_state::ShellExecutionObservation {
                                result,
                                delta: Some(before.diff(after)),
                                capture_error: None,
                                outcome_unknown: false,
                            },
                            Err(error) => super::ai_chat_state::ShellExecutionObservation {
                                result,
                                delta: None,
                                capture_error: Some(format!(
                                    "shell completed, but after-state capture failed: {error}"
                                )),
                                outcome_unknown: true,
                            },
                        }
                    }
                    Err(error) => super::ai_chat_state::ShellExecutionObservation {
                        result: crate::ai::tools::ToolResult::Error(format!(
                            "shell program was not executed because before-state capture failed: {error}"
                        )),
                        delta: None,
                        capture_error: Some(error),
                        outcome_unknown: false,
                    },
                };
                let _ = result_tx.send(observation);
            });
            if let Some(chat) = self.ai_state.chat.as_mut() {
                chat.pending_shell_execution = Some(super::ai_chat_state::PendingShellExecution {
                    tool_call: call,
                    runtime_tool: tool,
                    runtime_turn: turn,
                    dynamic_response: response,
                    receiver: result_rx,
                    task,
                });
                chat.waiting = true;
            }
            self.set_lsp_status("Agent shell program is running".into());
            return;
        }
        let result = {
            match self.dispatch_tool_call_with_approval(&call, None) {
                super::ai_chat_tools::ToolDispatchOutcome::Completed(result) => result,
                super::ai_chat_tools::ToolDispatchOutcome::ApprovalRequired(request) => {
                    crate::ai::tools::ToolResult::Error(format!(
                        "tool policy changed after auto-mode approval: {}",
                        request.message
                    ))
                }
            }
        };
        self.finish_dynamic_tool(&turn, &tool, &call, response, result);
    }

    fn poll_pending_shell_execution(&mut self) -> bool {
        let received = {
            let Some(pending) = self
                .ai_state
                .chat
                .as_mut()
                .and_then(|chat| chat.pending_shell_execution.as_mut())
            else {
                return false;
            };
            match pending.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return false,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    Some(super::ai_chat_state::ShellExecutionObservation {
                        result: crate::ai::tools::ToolResult::Error(
                            "shell execution task stopped without a result".into(),
                        ),
                        delta: None,
                        capture_error: Some(
                            "shell execution and workspace result are unknown".into(),
                        ),
                        outcome_unknown: true,
                    })
                }
            }
        };
        let pending = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.pending_shell_execution.take())
            .expect("pending shell exists");
        let observation = received.expect("shell result");
        if let Some(delta) = observation.delta {
            for mutation in delta.mutations {
                if let Err(error) = self.ai_state.agent_runtime.record_tool_file_mutation(
                    &pending.runtime_turn,
                    &pending.runtime_tool,
                    mutation,
                ) {
                    crate::log_warn!("agent_runtime", "failed to record shell mutation: {error}");
                }
            }
            for issue in delta.issues {
                if let Err(error) = self.ai_state.agent_runtime.record_tool_capture_issue(
                    &pending.runtime_turn,
                    &pending.runtime_tool,
                    issue,
                ) {
                    crate::log_warn!("agent_runtime", "failed to record capture issue: {error}");
                }
            }
        }
        if let Some(error) = observation.capture_error.as_ref() {
            if let Err(record_error) = self.ai_state.agent_runtime.record_tool_capture_issue(
                &pending.runtime_turn,
                &pending.runtime_tool,
                error.clone(),
            ) {
                crate::log_warn!(
                    "agent_runtime",
                    "failed to record capture failure: {record_error}"
                );
            }
        }
        if observation.outcome_unknown {
            let detail = observation
                .capture_error
                .unwrap_or_else(|| "shell result and workspace effects are unknown".into());
            if let Err(error) = self.ai_state.agent_runtime.mark_tool_outcome_unknown(
                &pending.runtime_turn,
                &pending.runtime_tool,
                detail.clone(),
            ) {
                crate::log_warn!(
                    "agent_runtime",
                    "failed to record unknown shell outcome: {error}"
                );
            }
            let _ = pending.dynamic_response.send(Err(detail));
            self.fail_specific_dynamic_turn(
                &pending.runtime_turn,
                "shell execution could not be durably observed".into(),
            );
        } else {
            self.finish_dynamic_tool(
                &pending.runtime_turn,
                &pending.runtime_tool,
                &pending.tool_call,
                pending.dynamic_response,
                observation.result,
            );
        }
        self.set_lsp_status(String::new());
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.waiting = true;
        }
        true
    }

    fn fail_pending_shell_for_lost_lease(&mut self, error: &str) -> bool {
        let Some(pending) = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.pending_shell_execution.take())
        else {
            return false;
        };
        pending.task.abort();
        let unknown = format!(
            "shell result and workspace effects are unknown because durable run ownership was lost: {error}"
        );
        if let Err(runtime_error) = self.ai_state.agent_runtime.mark_tool_outcome_unknown(
            &pending.runtime_turn,
            &pending.runtime_tool,
            unknown.clone(),
        ) {
            crate::log_warn!(
                "agent_runtime",
                "failed to record unknown shell outcome: {runtime_error}"
            );
        }
        let _ = pending.dynamic_response.send(Err(unknown));
        self.fail_specific_dynamic_turn(
            &pending.runtime_turn,
            format!("durable run ownership was lost: {error}"),
        );
        if let Some(job) = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.pending_job.take())
        {
            job.task.abort();
        }
        self.set_lsp_status(format!(
            "Stopped agent after durable run ownership was lost: {error}"
        ));
        true
    }

    fn fail_specific_dynamic_turn(
        &mut self,
        turn: &crate::agent_runtime::PendingTurnRef,
        detail: String,
    ) {
        if let Err(error) = self.ai_state.agent_runtime.fail_turn(turn, detail) {
            crate::log_warn!("agent_runtime", "failed to terminate dynamic turn: {error}");
        }
        if let Some(chat) = self.ai_state.chat.as_mut() {
            if chat
                .runtime_turn
                .as_ref()
                .is_some_and(|active| active.turn_id == turn.turn_id)
            {
                chat.runtime_turn = None;
            }
        }
    }

    pub(super) fn finish_dynamic_tool(
        &mut self,
        turn: &crate::agent_runtime::PendingTurnRef,
        tool: &crate::agent_runtime::PendingToolRef,
        call: &ToolCallInfo,
        response: tokio::sync::oneshot::Sender<Result<String, String>>,
        result: crate::ai::tools::ToolResult,
    ) {
        if let Err(error) = self.ai_runtime_finish_tool(turn, tool, &result) {
            let _ = response.send(Err(format!("failed to record tool result: {error}")));
            self.ai_runtime_fail_turn(format!("failed to record tool result: {error}"));
            self.clear_streaming_state();
            return;
        }
        self.record_tool_event_summary(call, &result);
        let result_content = self.format_tool_result_with_target(call, &result);
        if let Some(conv) = self.conversation_mut() {
            conv.append_tool_result(call.id.clone(), result_content);
        }
        let wire = match result {
            crate::ai::tools::ToolResult::Success(text) => Ok(text),
            crate::ai::tools::ToolResult::Error(text) => Err(text),
        };
        let _ = response.send(wire);
    }

    fn pause_dynamic_tool_for_approval(
        &mut self,
        turn: crate::agent_runtime::PendingTurnRef,
        tool: crate::agent_runtime::PendingToolRef,
        call: ToolCallInfo,
        response: tokio::sync::oneshot::Sender<Result<String, String>>,
        reason: String,
    ) {
        let root = self
            .ai_effective_project_root()
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.pending_tool_approval = Some(super::ai_chat_state::PendingToolApproval {
                tool_call: call,
                runtime_tool: Some(tool),
                remaining_tool_calls: Vec::new(),
                model_name: String::new(),
                requested_path: root.clone(),
                approval_root: root,
                dynamic_response: Some(response),
                dynamic_turn: Some(turn),
            });
            // Keep pending_job alive: its app-server task is blocked on the
            // dynamic response and resumes exactly once after this UI decision.
            chat.waiting = false;
        }
        self.set_lsp_status(format!(
            "Shell approval required: {reason}. Press Ctrl-Y to allow once or Ctrl-N to deny."
        ));
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
            let trimmed = content.to_string();
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

    /// Original call metadata for rendering expanded tool details.
    pub fn ai_chat_tool_event_call(&self, tool_call_id: &str) -> Option<&ToolCallInfo> {
        self.ai_chat_tool_event_summary(tool_call_id)
            .map(|summary| &summary.call)
    }

    pub fn ai_chat_is_tool_event_expanded(&self, tool_call_id: &str) -> bool {
        self.ai_state
            .chat
            .as_ref()
            .is_some_and(|chat| chat.expanded_tool_events.contains(tool_call_id))
    }

    pub fn toggle_ai_chat_tool_event(&mut self, tool_call_id: &str) {
        if let Some(chat) = self.ai_state.chat.as_mut() {
            if !chat.expanded_tool_events.remove(tool_call_id) {
                chat.expanded_tool_events.insert(tool_call_id.to_string());
            }
        }
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

    pub fn begin_ai_chat_text_selection(&mut self, row: usize, column: usize) {
        use super::render_cache::{ChatTextPoint, ChatTextSelection};

        let point = ChatTextPoint { row, column };
        self.render_cache.ai_chat_text_selection = Some(ChatTextSelection {
            anchor: point,
            head: point,
            moved: false,
        });
        self.render_cache.ai_chat_text_selecting = true;
    }

    pub fn update_ai_chat_text_selection(&mut self, row: usize, column: usize) {
        let Some(selection) = self.render_cache.ai_chat_text_selection.as_mut() else {
            return;
        };
        let point = super::render_cache::ChatTextPoint { row, column };
        selection.moved |= point != selection.anchor;
        selection.head = point;
    }

    /// Finish a mouse selection and copy it immediately, matching terminal
    /// select-to-copy behavior. A click without a drag clears the selection.
    pub fn finish_ai_chat_text_selection(&mut self) -> bool {
        self.render_cache.ai_chat_text_selecting = false;
        if !self
            .render_cache
            .ai_chat_text_selection
            .is_some_and(|selection| selection.moved)
        {
            self.render_cache.ai_chat_text_selection = None;
            return false;
        }
        let copied = self.copy_ai_chat_text_selection();
        if copied {
            self.set_lsp_status("Copied selected chat text".to_string());
        }
        copied
    }

    pub fn ai_chat_has_text_selection(&self) -> bool {
        self.render_cache
            .ai_chat_text_selection
            .is_some_and(|selection| selection.moved)
    }

    /// Selected display-column interval for an absolute rendered history row.
    pub fn ai_chat_text_selection_range(&self, row: usize) -> Option<(usize, usize)> {
        let selection = self.render_cache.ai_chat_text_selection?;
        if !selection.moved {
            return None;
        }
        let (start, end) = ordered_chat_selection(selection.anchor, selection.head);
        if row < start.row || row > end.row {
            return None;
        }
        if start.row == end.row {
            return Some((start.column, end.column.saturating_add(1)));
        }
        if row == start.row {
            Some((start.column, usize::MAX))
        } else if row == end.row {
            Some((0, end.column.saturating_add(1)))
        } else {
            Some((0, usize::MAX))
        }
    }

    /// Copy the active mouse selection. Returns false when no non-empty text
    /// is selected, allowing callers to fall back to copying the conversation.
    pub fn copy_ai_chat_text_selection(&mut self) -> bool {
        let Some(selection) = self.render_cache.ai_chat_text_selection else {
            return false;
        };
        if !selection.moved {
            return false;
        }
        let (start, end) = ordered_chat_selection(selection.anchor, selection.head);
        let rows = &self.render_cache.ai_chat_rendered_text_rows;
        if rows.is_empty() || start.row >= rows.len() {
            return false;
        }

        let mut selected_rows = Vec::new();
        for row_index in start.row..=end.row.min(rows.len() - 1) {
            let row = &rows[row_index];
            let (from, to) = if start.row == end.row {
                (start.column, end.column.saturating_add(1))
            } else if row_index == start.row {
                (start.column, usize::MAX)
            } else if row_index == end.row {
                (0, end.column.saturating_add(1))
            } else {
                (0, usize::MAX)
            };
            let mut text = slice_display_columns(row, from, to);
            if from == 0 {
                text = strip_chat_row_prefix(text);
            }
            selected_rows.push(text.trim_end_matches(' ').to_string());
        }
        let output = selected_rows.join("\n");
        if output.is_empty() {
            return false;
        }
        self.registers.set_clipboard(output);
        true
    }

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

fn ordered_chat_selection(
    first: super::render_cache::ChatTextPoint,
    second: super::render_cache::ChatTextPoint,
) -> (
    super::render_cache::ChatTextPoint,
    super::render_cache::ChatTextPoint,
) {
    if (first.row, first.column) <= (second.row, second.column) {
        (first, second)
    } else {
        (second, first)
    }
}

fn slice_display_columns(text: &str, start: usize, end: usize) -> String {
    let mut column = 0usize;
    text.chars()
        .filter(|character| {
            let width = char_display_width(*character).max(1);
            let character_start = column;
            let character_end = column.saturating_add(width);
            column = character_end;
            character_end > start && character_start < end
        })
        .collect()
}

fn strip_chat_row_prefix(mut text: String) -> String {
    if text.starts_with('\u{258d}') || text.starts_with('\u{258c}') {
        text.remove(0);
        if text.starts_with(' ') {
            text.remove(0);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat_types::ChatOpts;
    use crate::buffer::Buffer;
    use crate::run_log::{EventKind, TurnLifecycleState};

    fn open_test_chat(editor: &mut Editor) {
        editor
            .open_ai_chat(ChatOpts {
                name: "chat".to_string(),
                allow_edits: true,
                ..Default::default()
            })
            .expect("open chat");
    }

    fn attach_pending_runtime_job(
        editor: &mut Editor,
        turn: crate::agent_runtime::PendingTurnRef,
        branch_generation: u64,
    ) -> tokio::task::AbortHandle {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async { std::future::pending::<()>().await });
        let abort_handle = task.abort_handle();
        let chat = editor.ai_state.chat.as_mut().expect("chat");
        chat.runtime_turn = Some(Box::new(turn.clone()));
        chat.pending_job = Some(super::super::ai_chat_state::PendingAiChatJob {
            receiver: rx,
            task,
            profile_name: "test".into(),
            model_name: "test".into(),
            turn: Box::new(turn),
            branch_generation,
            steer_tx: None,
        });
        chat.waiting = true;
        abort_handle
    }

    #[tokio::test(flavor = "current_thread")]
    async fn closing_chat_keeps_provider_and_live_state_running() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("inspect").unwrap();
        let run_id = turn.run_id.clone();
        let abort_handle = attach_pending_runtime_job(&mut editor, turn, 0);
        editor.ai_state.chat.as_mut().unwrap().input = "follow up".into();

        editor.close_ai_chat();
        tokio::task::yield_now().await;

        assert!(!abort_handle.is_finished());
        assert_ne!(editor.mode(), Mode::AiChat);
        let chat = editor.ai_state.chat.as_ref().expect("hidden chat retained");
        assert_eq!(chat.input, "follow up");
        assert!(chat.pending_job.is_some());
        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert!(!matches!(
            &events.last().unwrap().kind,
            EventKind::TurnLifecycle(event) if event.state == TurnLifecycleState::Interrupted
        ));

        editor.discard_active_ai_chat("test cleanup");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelling_generation_stops_provider_but_preserves_chat() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("inspect").unwrap();
        let run_id = turn.run_id.clone();
        let abort_handle = attach_pending_runtime_job(&mut editor, turn, 0);
        {
            let chat = editor.ai_state.chat.as_mut().unwrap();
            chat.input = "keep this draft".into();
            chat.streaming_content = Some("Partial answer".into());
        }

        assert!(editor.cancel_ai_chat_generation());
        tokio::task::yield_now().await;

        assert!(abort_handle.is_finished());
        assert_eq!(editor.mode(), Mode::AiChat);
        assert_eq!(editor.ai_chat_input(), "keep this draft");
        assert!(!editor.ai_chat_waiting());
        assert!(editor
            .ai_chat_messages()
            .iter()
            .any(|message| message.content == "Partial answer"));
        assert!(editor
            .ai_chat_messages()
            .iter()
            .any(|message| message.content == "Generation stopped by user."));
        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert!(matches!(
            &events.last().unwrap().kind,
            EventKind::TurnLifecycle(event)
                if event.state == TurnLifecycleState::Interrupted
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reopening_chat_resumes_live_state_and_preserves_underlying_mode() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("inspect").unwrap();
        let abort_handle = attach_pending_runtime_job(&mut editor, turn, 0);
        editor.ai_state.chat.as_mut().unwrap().input = "still here".into();

        editor.close_ai_chat();
        assert_ne!(editor.mode(), Mode::AiChat);

        open_test_chat(&mut editor);
        tokio::task::yield_now().await;

        assert!(!abort_handle.is_finished());
        assert_eq!(editor.mode(), Mode::AiChat);
        assert_eq!(editor.ai_chat_input(), "still here");
        editor.close_ai_chat();
        assert_ne!(editor.mode(), Mode::AiChat);

        editor.discard_active_ai_chat("test cleanup");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stale_provider_branch_is_aborted_before_output_is_applied() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("inspect").unwrap();
        let run_id = turn.run_id.clone();
        let abort_handle = attach_pending_runtime_job(&mut editor, turn, 0);
        {
            let conv = editor.conversation_mut().unwrap();
            conv.append_user_message("root".into());
            let root = conv.active_leaf_id().unwrap();
            conv.append_assistant_message("old branch".into(), "test".into());
            conv.fork_from(root);
        }

        assert!(editor.poll_pending_ai_chat_job());
        tokio::task::yield_now().await;

        assert!(abort_handle.is_finished());
        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert!(matches!(
            &events.last().unwrap().kind,
            EventKind::TurnLifecycle(event)
                if event.state == TurnLifecycleState::Interrupted
        ));
        assert!(editor
            .conversation()
            .unwrap()
            .messages()
            .iter()
            .any(|message| message.content.contains("Discarded stale response")));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn dynamic_tool_events_are_terminal_before_codex_receives_result() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("check diagnostics").unwrap();
        let run_id = turn.run_id.clone();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async { std::future::pending::<()>().await });
        let abort_handle = task.abort_handle();
        editor.ai_state.chat.as_mut().unwrap().runtime_turn = Some(Box::new(turn.clone()));
        editor.ai_state.chat.as_mut().unwrap().streaming_content = Some(String::new());
        editor.ai_state.chat.as_mut().unwrap().pending_job =
            Some(super::super::ai_chat_state::PendingAiChatJob {
                receiver: rx,
                task,
                profile_name: "test".into(),
                model_name: "test".into(),
                turn: Box::new(turn),
                branch_generation: 0,
                steer_tx: None,
            });

        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        tx.send(StreamChunk::Content("Before tool. ".into()))
            .unwrap();
        tx.send(StreamChunk::DynamicToolRequest {
            call: ToolCallInfo {
                id: "provider-call-1".into(),
                name: "read_diagnostics".into(),
                arguments: serde_json::json!({}),
            },
            response: result_tx,
        })
        .unwrap();

        assert!(editor.poll_pending_ai_chat_job());
        let events_before_provider_result = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert!(matches!(
            &events_before_provider_result.last().unwrap().kind,
            EventKind::ToolResult(_)
        ));
        let pre_tool_message = events_before_provider_result
            .iter()
            .position(|event| {
                matches!(
                    &event.kind,
                    EventKind::Message(crate::run_log::MessageEvent {
                        role: crate::run_log::MessageRole::Agent,
                        content,
                    }) if content == "Before tool. "
                )
            })
            .unwrap();
        let tool_intent = events_before_provider_result
            .iter()
            .position(|event| matches!(event.kind, EventKind::ToolIntent(_)))
            .unwrap();
        assert!(pre_tool_message < tool_intent);
        assert!(editor
            .ai_chat_messages()
            .iter()
            .any(|message| message.role == ChatRole::Tool
                && message.tool_call_id.as_deref() == Some("provider-call-1")));
        let _provider_result = result_rx.await.unwrap();

        tx.send(StreamChunk::Content("After tool.".into())).unwrap();
        tx.send(StreamChunk::Done).unwrap();
        assert!(editor.poll_pending_ai_chat_job());
        abort_handle.abort();

        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert!(matches!(
            &events.last().unwrap().kind,
            EventKind::TurnLifecycle(event)
                if event.state == TurnLifecycleState::Completed
        ));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::ToolResult(_)))
                .count(),
            1
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn auto_mode_unauthorized_deploy_pauses_dynamic_codex_response() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let file = dir.path().join("main.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let mut editor = Editor::default();
        editor.open_file(&file).unwrap();
        open_test_chat(&mut editor);
        editor.ai_state.config.tool_approval_mode = crate::ai::ToolApprovalMode::Auto;
        let turn = editor.begin_ai_runtime_turn("inspect the project").unwrap();
        let run_id = turn.run_id.clone();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async { std::future::pending::<()>().await });
        let abort_handle = task.abort_handle();
        let chat = editor.ai_state.chat.as_mut().unwrap();
        chat.runtime_turn = Some(Box::new(turn.clone()));
        chat.streaming_content = Some(String::new());
        chat.pending_job = Some(super::super::ai_chat_state::PendingAiChatJob {
            receiver: rx,
            task,
            profile_name: "test".into(),
            model_name: "test".into(),
            turn: Box::new(turn),
            branch_generation: 0,
            steer_tx: None,
        });

        let (result_tx, mut result_rx) = tokio::sync::oneshot::channel();
        tx.send(StreamChunk::DynamicToolRequest {
            call: ToolCallInfo {
                id: "deploy-call".into(),
                name: "bash".into(),
                arguments: serde_json::json!({"command": "./deploy production"}),
            },
            response: result_tx,
        })
        .unwrap();

        assert!(editor.poll_pending_ai_chat_job());
        assert!(editor.ai_chat_has_pending_tool_approval());
        assert!(matches!(
            result_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event.kind, EventKind::ToolIntent(_))));
        assert!(!events
            .iter()
            .any(|event| matches!(event.kind, EventKind::ToolStarted(_))));

        // Headless `/keys` routes through this same input dispatcher; no
        // renderer state is involved in resolving the paused app-server call.
        crate::editor::InputHandler::handle_key_event(
            &mut editor,
            crate::KeyEvent::new(crate::KeyCode::Char('n'), crate::Modifiers::CONTROL),
        )
        .unwrap();
        assert!(!editor.ai_chat_has_pending_tool_approval());
        assert!(result_rx.await.unwrap().is_err());
        abort_handle.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn completed_agent_items_become_separate_chat_messages() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("work in stages").unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async { std::future::pending::<()>().await });
        let chat = editor.ai_state.chat.as_mut().unwrap();
        chat.runtime_turn = Some(Box::new(turn.clone()));
        chat.streaming_content = Some(String::new());
        chat.waiting = true;
        chat.pending_job = Some(super::super::ai_chat_state::PendingAiChatJob {
            receiver: rx,
            task,
            profile_name: "test".into(),
            model_name: "test".into(),
            turn: Box::new(turn),
            branch_generation: 0,
            steer_tx: None,
        });

        tx.send(StreamChunk::Thinking("Inspecting first.".into()))
            .unwrap();
        tx.send(StreamChunk::Content("I found the cause.".into()))
            .unwrap();
        tx.send(StreamChunk::AgentMessageComplete).unwrap();
        tx.send(StreamChunk::Content("The fix is verified.".into()))
            .unwrap();
        tx.send(StreamChunk::AgentMessageComplete).unwrap();
        tx.send(StreamChunk::Done).unwrap();

        assert!(editor.poll_pending_ai_chat_job());
        let messages = editor.conversation().unwrap().messages();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, ChatRole::Thinking);
        assert_eq!(messages[0].content, "Inspecting first.");
        assert_eq!(messages[1].role, ChatRole::Assistant);
        assert_eq!(messages[1].content, "I found the cause.");
        assert_eq!(messages[2].role, ChatRole::Assistant);
        assert_eq!(messages[2].content, "The fix is verified.");
        assert!(!editor.ai_chat_waiting());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn lost_durable_lease_blocks_approved_shell_effect() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git2::Repository::init(&repo).unwrap();
        let file = repo.join("main.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let runs = crate::run_log::RunStorageLayout::new(dir.path().join("runs"));
        let mut editor = Editor::default();
        editor.ai_state =
            Box::new(super::super::ai_state::AiState::with_run_storage_layout(runs).unwrap());
        editor.open_file(&file).unwrap();
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("create the marker").unwrap();
        let call = ToolCallInfo {
            id: "write-marker".into(),
            name: "bash".into(),
            arguments: serde_json::json!({"command": "touch effect-marker"}),
        };
        let tool = editor.ai_runtime_record_tool_intent(&turn, &call).unwrap();
        let key = editor.ai_chat_conversation_key();
        let binding = editor.ai_state.durable_chat_bindings.get(&key).unwrap();
        let run_id = binding.binding.run_id.clone();
        let services = editor.ai_state.durable_runs.as_ref().unwrap();
        let owner = services.owner.clone();
        services.catalog.release_lease(&run_id, &owner).unwrap();
        editor
            .ai_state
            .durable_chat_bindings
            .get_mut(&key)
            .unwrap()
            .lease_renewed_at = std::time::Instant::now() - std::time::Duration::from_secs(60);

        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        editor.execute_dynamic_tool_after_policy(turn, tool, call, response_tx);

        assert!(response_rx.await.unwrap().is_err());
        assert!(!repo.join("effect-marker").exists());
        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert!(!events
            .iter()
            .any(|event| matches!(event.kind, EventKind::ToolStarted(_))));
        assert!(events
            .iter()
            .any(|event| matches!(event.kind, EventKind::ToolResult(_))));
        assert!(events.iter().any(|event| matches!(
            &event.kind,
            EventKind::TurnLifecycle(lifecycle)
                if lifecycle.state == TurnLifecycleState::Failed
        )));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authorized_shell_runs_in_background_while_editor_poll_stays_responsive() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let file = dir.path().join("main.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let runs = tempfile::tempdir().unwrap();
        let mut editor = Editor::default();
        editor.ai_state = Box::new(
            super::super::ai_state::AiState::with_run_storage_layout(
                crate::run_log::RunStorageLayout::new(runs.path()),
            )
            .unwrap(),
        );
        editor.open_file(&file).unwrap();
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("run the gated check").unwrap();
        let run_id = turn.run_id.clone();
        let call = ToolCallInfo {
            id: "gated-shell".into(),
            name: "bash".into(),
            arguments: serde_json::json!({
                "command": "while [ ! -f release-gate ]; do sleep 0.01; done; touch agent-marker; echo completed"
            }),
        };
        let tool = editor.ai_runtime_record_tool_intent(&turn, &call).unwrap();
        let (response_tx, mut response_rx) = tokio::sync::oneshot::channel();

        let started = std::time::Instant::now();
        editor.execute_dynamic_tool_after_policy(turn, tool, call, response_tx);
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
        assert!(editor
            .ai_state
            .chat
            .as_ref()
            .unwrap()
            .pending_shell_execution
            .is_some());
        assert!(!editor.poll_pending_ai_chat_job());
        assert!(matches!(
            response_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));

        std::fs::write(dir.path().join("release-gate"), "go").unwrap();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if editor.poll_pending_ai_chat_job() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "shell did not finish"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let result = response_rx.await.unwrap().unwrap();
        assert!(result.contains("completed"), "{result}");
        assert!(editor.lsp_status().is_empty());
        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        let start_index = events
            .iter()
            .position(|event| matches!(event.kind, EventKind::ToolStarted(_)))
            .unwrap();
        let result_index = events
            .iter()
            .position(|event| matches!(event.kind, EventKind::ToolResult(_)))
            .unwrap();
        let mutation_index = events
            .iter()
            .position(|event| {
                matches!(
                    &event.kind,
                    EventKind::FileMutation(mutation) if mutation.path == "agent-marker"
                )
            })
            .unwrap();
        assert!(start_index < mutation_index && mutation_index < result_index);
        assert_eq!(
            events[mutation_index].operation_id,
            events[start_index].operation_id
        );
    }

    fn attach_finished_classifier(
        editor: &mut Editor,
        result: Result<crate::ai::auto_mode::ClassifierVerdict, String>,
    ) -> tokio::sync::oneshot::Receiver<Result<String, String>> {
        let turn = editor.begin_ai_runtime_turn("review command").unwrap();
        let call = ToolCallInfo {
            id: "classifier-call".into(),
            name: "bash".into(),
            arguments: serde_json::json!({"command": "custom-tool"}),
        };
        let tool = editor.ai_runtime_record_tool_intent(&turn, &call).unwrap();
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        let (classification_tx, classification_rx) = tokio::sync::oneshot::channel();
        classification_tx.send(result).unwrap();
        editor
            .ai_state
            .chat
            .as_mut()
            .unwrap()
            .pending_auto_mode_classification =
            Some(super::super::ai_chat_state::PendingAutoModeClassification {
                tool_call: call,
                runtime_tool: tool,
                runtime_turn: turn,
                dynamic_response: response_tx,
                receiver: classification_rx,
            });
        response_rx
    }

    #[tokio::test]
    async fn classifier_failure_escalates_to_paused_user_approval() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let mut response = attach_finished_classifier(&mut editor, Err("protocol failed".into()));

        assert!(editor.poll_pending_auto_mode_classification());
        assert!(editor.ai_chat_has_pending_tool_approval());
        assert!(matches!(
            response.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn classifier_deny_returns_terminal_error_without_execution() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let verdict = crate::ai::auto_mode::ClassifierVerdict::parse_strict(
            r#"{"policy_version":"ovim.auto-mode.v1","decision":"deny","scope":{"project_root":"/repo"},"reason":"conflicts with objective","confidence":0.99,"expiry":{"kind":"after_command"}}"#,
        )
        .unwrap();
        let response = attach_finished_classifier(&mut editor, Ok(verdict));

        assert!(editor.poll_pending_auto_mode_classification());
        assert!(!editor.ai_chat_has_pending_tool_approval());
        assert!(response.await.unwrap().is_err());
    }

    fn append_recorded_test_turn(
        editor: &mut Editor,
        user: &str,
        assistant: &str,
    ) -> (NodeId, NodeId, crate::agent_runtime::PendingTurnRef) {
        let turn = editor.begin_ai_runtime_turn(user).unwrap();
        let user_event = turn.initiating_event.caused_by.clone().unwrap();
        editor.ai_state.chat.as_mut().unwrap().runtime_turn = Some(Box::new(turn.clone()));
        let user_node = editor
            .conversation_mut()
            .unwrap()
            .append_user_message(user.into());
        editor.record_ai_chat_node(user_node, user_event);
        let assistant_event = editor.ai_runtime_append_agent_message(assistant).unwrap();
        let assistant_node = editor
            .conversation_mut()
            .unwrap()
            .append_assistant_message(assistant.into(), "test".into());
        editor.record_ai_chat_node(assistant_node, assistant_event);
        editor.ai_runtime_complete_turn();
        (user_node, assistant_node, turn)
    }

    #[test]
    fn shell_authorization_projection_is_chronological_durable_and_bounded() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        for index in 0..10 {
            append_recorded_test_turn(&mut editor, &format!("instruction {index}"), "ack");
        }
        let context = editor.shell_authorization_context(std::path::Path::new("/repo"));
        assert_eq!(context.explicit_user_instructions.len(), 8);
        assert_eq!(
            context.explicit_user_instructions[0].instruction,
            "instruction 2"
        );
        assert_eq!(
            context.explicit_user_instructions[7].instruction,
            "instruction 9"
        );
        assert!(context
            .explicit_user_instructions
            .iter()
            .all(|authorization| authorization.source_id.starts_with("evt_")));
        assert_eq!(context.authorized_objectives[0].objective, "instruction 9");
        assert_eq!(
            context.authorized_objectives[0].source_id,
            context.explicit_user_instructions[7].source_id
        );
    }

    #[test]
    fn ui_fork_gets_distinct_runtime_branch_and_switch_back_resumes_main() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let (_, first_reply, first_turn) = append_recorded_test_turn(&mut editor, "one", "a1");
        let (_, main_leaf, _) = append_recorded_test_turn(&mut editor, "two", "a2");

        assert!(editor.fork_ai_chat_runtime_from(first_reply));
        let fork_turn = editor.begin_ai_runtime_turn("forked").unwrap();
        assert_ne!(fork_turn.branch_id, first_turn.branch_id);
        let fork_user_event_id = fork_turn.initiating_event.caused_by.clone().unwrap();
        let events = editor
            .ai_state
            .agent_runtime
            .events(&fork_turn.run_id)
            .unwrap();
        let fork_user_event = events
            .iter()
            .find(|event| event.event_id == fork_user_event_id)
            .unwrap();
        let selected_event = events
            .iter()
            .find(|event| Some(&event.event_id) == fork_user_event.caused_by.as_ref())
            .unwrap();
        let durable_fork_event = events
            .iter()
            .find(|event| Some(&event.event_id) == selected_event.caused_by.as_ref())
            .unwrap();
        assert!(matches!(
            durable_fork_event.kind,
            EventKind::BranchLifecycle(_)
        ));
        assert_eq!(
            durable_fork_event.caused_by,
            editor
                .ai_state
                .conversation_runtime_nodes
                .get(&editor.ai_chat_conversation_key())
                .unwrap()
                .get(&first_reply)
                .map(|node| node.event_id.clone())
        );
        editor.ai_state.chat.as_mut().unwrap().runtime_turn = Some(Box::new(fork_turn));
        editor.ai_runtime_complete_turn();

        assert!(editor.switch_ai_chat_runtime_branch(main_leaf));
        let resumed = editor.begin_ai_runtime_turn("back").unwrap();
        assert_eq!(resumed.branch_id, first_turn.branch_id);
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
