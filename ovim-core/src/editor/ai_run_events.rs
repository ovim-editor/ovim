use crate::agent_runtime::{
    AgentRuntimeError, AgentSpec, BranchLocator, ConversationLocator, PendingToolRef,
    PendingTurnRef,
};
use crate::ai::chat_types::{NodeId, ToolCallInfo};
use crate::ai::tools::{SideEffect, ToolResult};
use crate::run_log::ToolSideEffect;

use super::Editor;

impl Editor {
    /// Flush only the newly streamed suffixes. This is called immediately
    /// before tool intent so message/tool causality reflects arrival order.
    pub(crate) fn flush_ai_runtime_stream_segments(&mut self) {
        let (thinking, content) = match self.ai_state.chat.as_ref() {
            Some(chat) => {
                let thinking = chat.streaming_thinking.as_ref().and_then(|text| {
                    text.get(chat.runtime_recorded_thinking_bytes..)
                        .filter(|suffix| !suffix.is_empty())
                        .map(str::to_owned)
                });
                let content = chat.streaming_content.as_ref().and_then(|text| {
                    text.get(chat.runtime_recorded_content_bytes..)
                        .filter(|suffix| !suffix.is_empty())
                        .map(str::to_owned)
                });
                (thinking, content)
            }
            None => return,
        };
        if let Some(thinking) = thinking {
            if let Some(event_id) = self.ai_runtime_append_reasoning(&thinking) {
                if let Some(chat) = self.ai_state.chat.as_mut() {
                    chat.runtime_last_reasoning_event = Some(event_id);
                }
            }
        }
        if let Some(content) = content {
            if let Some(event_id) = self.ai_runtime_append_agent_message(&content) {
                if let Some(chat) = self.ai_state.chat.as_mut() {
                    chat.runtime_last_content_event = Some(event_id);
                }
            }
        }
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.runtime_recorded_thinking_bytes =
                chat.streaming_thinking.as_ref().map_or(0, String::len);
            chat.runtime_recorded_content_bytes =
                chat.streaming_content.as_ref().map_or(0, String::len);
        }
    }

    pub(crate) fn ai_runtime_conversation_locator(&self) -> ConversationLocator {
        let key = self.ai_chat_conversation_key();
        self.ai_state
            .durable_chat_bindings
            .get(&key)
            .map(|binding| binding.locator.clone())
            .unwrap_or_else(|| {
                ConversationLocator(format!("buffer:{}:conversation:{}", key.0, key.1))
            })
    }

    pub(crate) fn ai_runtime_current_tip(&self) -> Option<crate::run_log::EventId> {
        let locator = self.ai_runtime_conversation_locator();
        self.ai_state
            .agent_runtime
            .selected_branch_tip(&locator)
            .cloned()
    }

    pub(crate) fn begin_ai_runtime_turn(
        &mut self,
        user_message: &str,
    ) -> Result<PendingTurnRef, AgentRuntimeError> {
        self.heartbeat_ai_chat_lease().map_err(|error| {
            AgentRuntimeError::BindingMismatch(format!("durable run lease unavailable: {error}"))
        })?;
        let locator = self.ai_runtime_conversation_locator();
        let branch = self
            .ai_state
            .chat
            .as_ref()
            .map(|chat| chat.runtime_branch.clone())
            .unwrap_or_else(|| BranchLocator("branch-0".into()));
        self.ai_state.agent_runtime.begin_turn(
            locator,
            branch,
            user_message,
            AgentSpec {
                kind: "interactive_chat".into(),
                objective: Some(user_message.into()),
            },
        )
    }

    pub(crate) fn ai_runtime_append_reasoning(
        &mut self,
        content: &str,
    ) -> Option<crate::run_log::EventId> {
        let Some(turn) = self.active_ai_runtime_turn() else {
            return None;
        };
        match self
            .ai_state
            .agent_runtime
            .append_reasoning_summary(&turn, content)
        {
            Ok(event) => Some(event.event_id),
            Err(error) => {
                crate::log_warn!(
                    "agent_runtime",
                    "failed to record reasoning summary: {error}"
                );
                None
            }
        }
    }

    pub(crate) fn ai_runtime_append_agent_message(
        &mut self,
        content: &str,
    ) -> Option<crate::run_log::EventId> {
        let Some(turn) = self.active_ai_runtime_turn() else {
            return None;
        };
        match self
            .ai_state
            .agent_runtime
            .append_agent_message(&turn, content)
        {
            Ok(event) => Some(event.event_id),
            Err(error) => {
                crate::log_warn!("agent_runtime", "failed to record agent message: {error}");
                None
            }
        }
    }

    pub(crate) fn record_ai_chat_node(
        &mut self,
        node_id: NodeId,
        event_id: crate::run_log::EventId,
    ) {
        let key = self.ai_chat_conversation_key();
        let branch = self
            .ai_state
            .chat
            .as_ref()
            .map(|chat| chat.runtime_branch.clone())
            .unwrap_or_else(|| BranchLocator("branch-0".into()));
        self.ai_state
            .conversation_runtime_nodes
            .entry(key)
            .or_default()
            .insert(
                node_id,
                super::ai_state::ChatRuntimeNodeRef { event_id, branch },
            );
    }

    pub(crate) fn fork_ai_chat_runtime_from(&mut self, node_id: NodeId) -> bool {
        if self.active_ai_runtime_turn().is_some() {
            self.set_lsp_status("Wait for or stop the active agent turn before forking".into());
            return false;
        }
        let key = self.ai_chat_conversation_key();
        let Some(source) = self
            .ai_state
            .conversation_runtime_nodes
            .get(&key)
            .and_then(|nodes| nodes.get(&node_id))
            .cloned()
        else {
            self.set_lsp_status("This historical message predates replay metadata".into());
            return false;
        };
        let locator = self.ai_runtime_conversation_locator();
        let next_generation = self
            .conversation()
            .map(|conversation| conversation.branch_generation().wrapping_add(1))
            .unwrap_or_default();
        let target = BranchLocator(format!("branch-{next_generation}"));
        if let Err(error) = self.ai_state.agent_runtime.fork_branch_at(
            &locator,
            &source.branch,
            target.clone(),
            source.event_id,
        ) {
            self.set_lsp_status(format!("Unable to fork agent history: {error}"));
            return false;
        }
        if let Err(error) = self.ai_state.agent_runtime.select_branch(&locator, &target) {
            self.set_lsp_status(format!("Unable to select forked agent history: {error}"));
            return false;
        }
        if let Some(conversation) = self.conversation_mut() {
            conversation.fork_from(node_id);
        }
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.runtime_branch = target;
        }
        self.persist_selected_ai_branch();
        true
    }

    pub(crate) fn switch_ai_chat_runtime_branch(&mut self, node_id: NodeId) -> bool {
        if self.active_ai_runtime_turn().is_some() {
            self.set_lsp_status("Wait for or stop the active agent turn before switching".into());
            return false;
        }
        let Some(leaf_id) = self
            .conversation()
            .and_then(|conversation| conversation.branch_leaf_id(node_id))
        else {
            return false;
        };
        let key = self.ai_chat_conversation_key();
        let Some(target) = self
            .ai_state
            .conversation_runtime_nodes
            .get(&key)
            .and_then(|nodes| nodes.get(&leaf_id))
            .map(|node| node.branch.clone())
        else {
            self.set_lsp_status("This historical branch predates replay metadata".into());
            return false;
        };
        let locator = self.ai_runtime_conversation_locator();
        if let Err(error) = self.ai_state.agent_runtime.select_branch(&locator, &target) {
            self.set_lsp_status(format!("Unable to select agent history: {error}"));
            return false;
        }
        if let Some(conversation) = self.conversation_mut() {
            conversation.switch_to_branch(node_id);
        }
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.runtime_branch = target;
            chat.viewport = super::ai_chat_state::ChatViewportState::default();
            chat.history.selected_node_id = None;
            chat.history.selected_queued_id = None;
        }
        self.render_cache.ai_chat_text_selection = None;
        self.render_cache.ai_chat_text_selecting = false;
        self.persist_selected_ai_branch();
        true
    }

    fn persist_selected_ai_branch(&mut self) {
        let key = self.ai_chat_conversation_key();
        let Some(services) = self.ai_state.durable_runs.as_ref() else {
            return;
        };
        let Some(entry) = self.ai_state.durable_chat_bindings.get_mut(&key) else {
            return;
        };
        let Some((_, branch)) = self.ai_state.agent_runtime.selected_branch(&entry.locator) else {
            return;
        };
        match services
            .catalog
            .update_selected_branch(&entry.binding.key, branch.branch_id.clone())
        {
            Ok(Some(binding)) => entry.binding = binding,
            Ok(None) => crate::log_warn!("agent_runtime", "catalog binding disappeared"),
            Err(error) => crate::log_warn!(
                "agent_runtime",
                "failed to persist selected agent branch: {error}"
            ),
        }
    }

    pub(crate) fn ai_runtime_complete_turn(&mut self) {
        self.finish_ai_runtime_turn(AiTurnTerminal::Completed, None);
    }

    pub(crate) fn ai_runtime_fail_turn(&mut self, detail: impl Into<String>) {
        self.finish_ai_runtime_turn(AiTurnTerminal::Failed, Some(detail.into()));
    }

    pub(crate) fn ai_runtime_interrupt_turn(&mut self, detail: impl Into<String>) {
        self.finish_ai_runtime_turn(AiTurnTerminal::Interrupted, Some(detail.into()));
    }

    fn finish_ai_runtime_turn(&mut self, terminal: AiTurnTerminal, detail: Option<String>) {
        let Some(turn) = self.active_ai_runtime_turn() else {
            return;
        };
        let result = match terminal {
            AiTurnTerminal::Completed => self.ai_state.agent_runtime.complete_turn(&turn),
            AiTurnTerminal::Failed => self
                .ai_state
                .agent_runtime
                .fail_turn(&turn, detail.unwrap_or_default()),
            AiTurnTerminal::Interrupted => {
                self.ai_state.agent_runtime.interrupt_turn(&turn, detail)
            }
        };
        match result {
            Ok(_) | Err(AgentRuntimeError::TurnAlreadyTerminal) => {
                if let Some(chat) = self.ai_state.chat.as_mut() {
                    chat.runtime_turn = None;
                }
            }
            Err(error) => {
                crate::log_warn!("agent_runtime", "failed to terminate AI turn: {error}");
            }
        }
    }

    pub(crate) fn active_ai_runtime_turn(&self) -> Option<PendingTurnRef> {
        self.ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.runtime_turn.as_deref().cloned())
    }

    pub(crate) fn ai_runtime_record_tool_intent(
        &mut self,
        turn: &PendingTurnRef,
        call: &ToolCallInfo,
    ) -> Result<PendingToolRef, AgentRuntimeError> {
        let side_effect = self
            .ai_state
            .tool_registry
            .get(&call.name)
            .map(|tool| normalize_side_effect(tool.side_effect))
            .unwrap_or(ToolSideEffect::Unknown);
        self.ai_state.agent_runtime.record_tool_intent(
            turn,
            call.name.clone(),
            call.arguments.clone(),
            side_effect,
            (!call.id.is_empty()).then(|| call.id.clone()),
        )
    }

    pub(crate) fn ai_runtime_start_tool(
        &mut self,
        turn: &PendingTurnRef,
        tool: &PendingToolRef,
    ) -> Result<(), AgentRuntimeError> {
        self.ai_state
            .agent_runtime
            .start_tool(turn, tool)
            .map(|_| ())
    }

    pub(crate) fn ai_runtime_finish_tool(
        &mut self,
        turn: &PendingTurnRef,
        tool: &PendingToolRef,
        result: &ToolResult,
    ) -> Result<(), AgentRuntimeError> {
        match result {
            ToolResult::Success(output) => self
                .ai_state
                .agent_runtime
                .complete_tool(
                    turn,
                    tool,
                    Some("tool completed".into()),
                    Some(serde_json::Value::String(output.clone())),
                )
                .map(|_| ()),
            ToolResult::Error(error) => self
                .ai_state
                .agent_runtime
                .fail_tool(turn, tool, error.clone())
                .map(|_| ()),
        }
    }
}

enum AiTurnTerminal {
    Completed,
    Failed,
    Interrupted,
}

fn normalize_side_effect(side_effect: SideEffect) -> ToolSideEffect {
    match side_effect {
        SideEffect::Read => ToolSideEffect::Read,
        SideEffect::Navigation => ToolSideEffect::Navigation,
        SideEffect::Mutation => ToolSideEffect::Mutation,
        SideEffect::External => ToolSideEffect::External,
    }
}
