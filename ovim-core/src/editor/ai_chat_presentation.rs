use crate::ai::chat_types::{NodeId, ToolCallInfo, ToolSummaryKind};

use super::ai_chat_state::ToolEventSummary;
use super::Editor;

impl Editor {
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
            .or_else(|| {
                self.ai_chat_messages()
                    .iter()
                    .flat_map(|message| message.tool_calls.iter())
                    .find(|call| call.id == tool_call_id)
            })
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
}
