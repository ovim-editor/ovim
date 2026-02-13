use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
    Thinking,
    Error,
    /// Tool result message.
    Tool,
}

#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Chain-of-thought tokens (Anthropic extended thinking).
    Thinking(String),
    /// Response content tokens.
    Content(String),
    /// Tool call in progress (M4 — parsers ignore for now).
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    /// Completed tool call (M4).
    ToolCallComplete {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    /// Stream finished successfully.
    Done,
    /// Stream error.
    Error(String),
}

/// Metadata about a tool call made by the assistant.
#[derive(Debug, Clone)]
pub struct ToolCallInfo {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    /// Which model generated this (assistant messages only).
    pub model: Option<String>,
    pub timestamp: Instant,
    /// Tool calls made by this assistant message.
    pub tool_calls: Vec<ToolCallInfo>,
    /// For Tool role: which tool call this is a result for.
    pub tool_call_id: Option<String>,
}

/// M1: linear append-only conversation. M6 adds branching.
pub struct ConversationTree {
    messages: Vec<ChatMessage>,
}

impl ConversationTree {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    pub fn append_user_message(&mut self, content: String) {
        self.messages.push(ChatMessage {
            role: ChatRole::User,
            content,
            model: None,
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: None,
        });
    }

    pub fn append_assistant_message(&mut self, content: String, model: String) {
        self.messages.push(ChatMessage {
            role: ChatRole::Assistant,
            content,
            model: Some(model),
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: None,
        });
    }

    pub fn append_assistant_message_with_tools(
        &mut self,
        content: String,
        model: String,
        tool_calls: Vec<ToolCallInfo>,
    ) {
        self.messages.push(ChatMessage {
            role: ChatRole::Assistant,
            content,
            model: Some(model),
            timestamp: Instant::now(),
            tool_calls,
            tool_call_id: None,
        });
    }

    pub fn append_tool_result(&mut self, tool_call_id: String, content: String) {
        self.messages.push(ChatMessage {
            role: ChatRole::Tool,
            content,
            model: None,
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: Some(tool_call_id),
        });
    }

    pub fn append_thinking_message(&mut self, content: String, model: String) {
        self.messages.push(ChatMessage {
            role: ChatRole::Thinking,
            content,
            model: Some(model),
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: None,
        });
    }

    pub fn append_error(&mut self, content: String) {
        self.messages.push(ChatMessage {
            role: ChatRole::Error,
            content,
            model: None,
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: None,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }
}

impl Default for ConversationTree {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChatOpts {
    /// Conversation key (e.g. "chat", "query").
    pub name: String,
    /// Profile override.
    pub profile: Option<String>,
    /// Whether the assistant can suggest edits.
    pub allow_edits: bool,
    pub system_prompt: Option<String>,
    pub initial_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatFocus {
    TextInput,
    MessageHistory,
    ModelSelector,
}

impl Default for ChatFocus {
    fn default() -> Self {
        Self::TextInput
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_tree_append_and_read() {
        let mut tree = ConversationTree::new();
        assert!(tree.is_empty());
        assert_eq!(tree.len(), 0);

        tree.append_user_message("hello".into());
        assert_eq!(tree.len(), 1);
        assert_eq!(tree.messages()[0].role, ChatRole::User);
        assert_eq!(tree.messages()[0].content, "hello");

        tree.append_assistant_message("hi there".into(), "gpt-4".into());
        assert_eq!(tree.len(), 2);
        assert_eq!(tree.messages()[1].role, ChatRole::Assistant);
        assert_eq!(tree.messages()[1].model.as_deref(), Some("gpt-4"));

        tree.append_error("network error".into());
        assert_eq!(tree.len(), 3);
        assert_eq!(tree.messages()[2].role, ChatRole::Error);
    }

    #[test]
    fn chat_opts_defaults() {
        let opts = ChatOpts::default();
        assert!(opts.name.is_empty());
        assert!(opts.profile.is_none());
        assert!(!opts.allow_edits);
        assert!(opts.system_prompt.is_none());
        assert!(opts.initial_message.is_none());
    }
}
