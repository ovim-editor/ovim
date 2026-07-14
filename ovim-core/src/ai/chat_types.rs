use super::config::ChatContextConfig;
use std::collections::HashMap;
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

#[derive(Debug)]
pub enum StreamChunk {
    /// Chain-of-thought tokens (Anthropic extended thinking).
    Thinking(String),
    /// Response content tokens.
    Content(String),
    /// The current provider-native assistant message item is complete.
    ///
    /// A single agent turn may contain several assistant messages separated by
    /// tool work. Keeping this boundary lets the UI preserve that sequence
    /// instead of presenting the whole turn as one synthetic message.
    AgentMessageComplete,
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
    /// A Codex app-server dynamic tool call that must execute against live editor state.
    DynamicToolRequest {
        call: ToolCallInfo,
        response: tokio::sync::oneshot::Sender<Result<String, String>>,
    },
    /// A queued user steer was accepted by the active provider turn.
    SteerAccepted { content: String },
    /// The provider could not steer the active turn; ovim keeps it queued for
    /// the next round instead of failing the current response.
    SteerRejected { content: String, error: String },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSummaryKind {
    Read,
    Navigation,
    Mutation,
    Search,
    Diagnostics,
    Other,
    Error,
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

pub type NodeId = u64;

pub struct ChatNode {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub message: ChatMessage,
}

/// Conversation stored as a tree. Callers see `messages() -> &[ChatMessage]`
/// which returns the cached active branch (root → active_leaf).
pub struct ConversationTree {
    nodes: HashMap<NodeId, ChatNode>,
    next_id: NodeId,
    root_id: Option<NodeId>,
    active_leaf: Option<NodeId>,
    /// Cached active branch messages (root → active_leaf).
    branch_cache: Vec<ChatMessage>,
    /// Parallel vec of NodeIds matching branch_cache by index.
    branch_node_ids: Vec<NodeId>,
    /// Changes whenever the active trajectory is forked or switched.
    branch_generation: u64,
}

impl ConversationTree {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            next_id: 0,
            root_id: None,
            active_leaf: None,
            branch_cache: Vec::new(),
            branch_node_ids: Vec::new(),
            branch_generation: 0,
        }
    }

    pub fn messages(&self) -> &[ChatMessage] {
        &self.branch_cache
    }

    /// NodeIds corresponding to each message in `messages()`, same order/length.
    pub fn node_ids_for_active_branch(&self) -> &[NodeId] {
        &self.branch_node_ids
    }

    pub fn node(&self, id: NodeId) -> Option<&ChatNode> {
        self.nodes.get(&id)
    }

    pub fn active_leaf_id(&self) -> Option<NodeId> {
        self.active_leaf
    }

    pub fn branch_generation(&self) -> u64 {
        self.branch_generation
    }

    pub fn root_id(&self) -> Option<NodeId> {
        self.root_id
    }

    pub fn all_nodes(&self) -> &HashMap<NodeId, ChatNode> {
        &self.nodes
    }

    // ------------------------------------------------------------------
    // Append methods — all create a node, link it, update cache
    // ------------------------------------------------------------------

    pub fn append_user_message(&mut self, content: String) -> NodeId {
        self.append_node(ChatMessage {
            role: ChatRole::User,
            content,
            model: None,
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: None,
        })
    }

    pub fn append_assistant_message(&mut self, content: String, model: String) -> NodeId {
        self.append_node(ChatMessage {
            role: ChatRole::Assistant,
            content,
            model: Some(model),
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: None,
        })
    }

    pub fn append_assistant_message_with_tools(
        &mut self,
        content: String,
        model: String,
        tool_calls: Vec<ToolCallInfo>,
    ) -> NodeId {
        self.append_node(ChatMessage {
            role: ChatRole::Assistant,
            content,
            model: Some(model),
            timestamp: Instant::now(),
            tool_calls,
            tool_call_id: None,
        })
    }

    pub fn append_tool_result(&mut self, tool_call_id: String, content: String) -> NodeId {
        self.append_node(ChatMessage {
            role: ChatRole::Tool,
            content,
            model: None,
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: Some(tool_call_id),
        })
    }

    pub fn append_thinking_message(&mut self, content: String, model: String) -> NodeId {
        self.append_node(ChatMessage {
            role: ChatRole::Thinking,
            content,
            model: Some(model),
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: None,
        })
    }

    pub fn append_error(&mut self, content: String) -> NodeId {
        self.append_node(ChatMessage {
            role: ChatRole::Error,
            content,
            model: None,
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: None,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.branch_cache.len()
    }

    // ------------------------------------------------------------------
    // Tree operations
    // ------------------------------------------------------------------

    /// Set `active_leaf` to the given node. The next append will create a
    /// child of this node (i.e. a sibling of whatever was there before).
    pub fn fork_from(&mut self, node_id: NodeId) {
        if self.nodes.contains_key(&node_id) {
            self.active_leaf = Some(node_id);
            self.rebuild_branch_cache();
            self.branch_generation = self.branch_generation.wrapping_add(1);
        }
    }

    /// Switch to the branch containing `node_id` by walking down first-child
    /// pointers to a leaf, then rebuild the cache.
    pub fn switch_to_branch(&mut self, node_id: NodeId) {
        let Some(current) = self.branch_leaf_id(node_id) else {
            return;
        };
        self.active_leaf = Some(current);
        self.rebuild_branch_cache();
        self.branch_generation = self.branch_generation.wrapping_add(1);
    }

    pub fn branch_leaf_id(&self, node_id: NodeId) -> Option<NodeId> {
        if !self.nodes.contains_key(&node_id) {
            return None;
        }
        let mut current = node_id;
        loop {
            let children = &self.nodes[&current].children;
            if children.is_empty() {
                return Some(current);
            }
            current = children[0];
        }
    }

    /// Root-to-leaf path of `ChatNode` refs for the active branch.
    pub fn active_branch_nodes(&self) -> Vec<&ChatNode> {
        self.branch_node_ids
            .iter()
            .filter_map(|id| self.nodes.get(id))
            .collect()
    }

    /// Number of siblings (including self) for a node.
    pub fn sibling_count(&self, node_id: NodeId) -> usize {
        self.nodes
            .get(&node_id)
            .and_then(|n| n.parent)
            .and_then(|pid| self.nodes.get(&pid))
            .map(|parent| parent.children.len())
            .unwrap_or(1)
    }

    /// Number of direct children.
    pub fn child_count(&self, node_id: NodeId) -> usize {
        self.nodes
            .get(&node_id)
            .map(|n| n.children.len())
            .unwrap_or(0)
    }

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    fn next_id(&mut self) -> NodeId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn append_node(&mut self, message: ChatMessage) -> NodeId {
        let id = self.next_id();
        let parent = self.active_leaf;

        let node = ChatNode {
            id,
            parent,
            children: Vec::new(),
            message,
        };
        self.nodes.insert(id, node);

        // Link as child of parent
        if let Some(pid) = parent {
            if let Some(parent_node) = self.nodes.get_mut(&pid) {
                parent_node.children.push(id);
            }
        }

        // First node becomes root
        if self.root_id.is_none() {
            self.root_id = Some(id);
        }

        self.active_leaf = Some(id);
        self.rebuild_branch_cache();
        id
    }

    fn rebuild_branch_cache(&mut self) {
        self.branch_cache.clear();
        self.branch_node_ids.clear();

        let leaf = match self.active_leaf {
            Some(id) => id,
            None => return,
        };

        // Walk from leaf to root, collecting ids
        let mut path = Vec::new();
        let mut current = Some(leaf);
        while let Some(id) = current {
            path.push(id);
            current = self.nodes.get(&id).and_then(|n| n.parent);
        }
        path.reverse();

        for id in path {
            if let Some(node) = self.nodes.get(&id) {
                self.branch_cache.push(node.message.clone());
                self.branch_node_ids.push(id);
            }
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChatFocus {
    #[default]
    TextInput,
    MessageHistory,
    ModelSelector,
    TreePanel,
}

/// Apply observation masking to a message list for API serialization.
///
/// Messages within the observation window (the N most recent turns) are
/// kept verbatim. For older messages, Tool role content is replaced with
/// the mask template. All messages stay in the list — only content changes.
///
/// "Turn" = one User message (+ any following assistant/tool messages).
/// We count turns by counting User messages from the end.
pub fn apply_observation_mask(
    messages: &[ChatMessage],
    config: &ChatContextConfig,
) -> Vec<ChatMessage> {
    if messages.is_empty() || config.observation_window == 0 {
        return messages.to_vec();
    }

    // Find the boundary: the observation_window-th User message from the end.
    let mut user_count = 0;
    let mut boundary_index = 0; // messages at or after this index are within the window
    for (i, msg) in messages.iter().enumerate().rev() {
        if msg.role == ChatRole::User {
            user_count += 1;
            if user_count == config.observation_window {
                boundary_index = i;
                break;
            }
        }
    }

    // If we didn't find enough User messages, everything is within the window
    if user_count < config.observation_window {
        return messages.to_vec();
    }

    // Count turns for the mask template (turn 1 = first turn from the start)
    let mut turn_number = 0;
    messages
        .iter()
        .enumerate()
        .map(|(i, msg)| {
            if msg.role == ChatRole::User {
                turn_number += 1;
            }
            if i < boundary_index && msg.role == ChatRole::Tool {
                let masked_content = config
                    .mask_template
                    .replace("{turn}", &turn_number.to_string());
                ChatMessage {
                    role: msg.role.clone(),
                    content: masked_content,
                    model: msg.model.clone(),
                    timestamp: msg.timestamp,
                    tool_calls: msg.tool_calls.clone(),
                    tool_call_id: msg.tool_call_id.clone(),
                }
            } else {
                msg.clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_append_and_read() {
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
    fn tree_node_ids_match_messages() {
        let mut tree = ConversationTree::new();
        tree.append_user_message("a".into());
        tree.append_assistant_message("b".into(), "m".into());
        tree.append_user_message("c".into());

        let ids = tree.node_ids_for_active_branch();
        let msgs = tree.messages();
        assert_eq!(ids.len(), msgs.len());
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(tree.node(*id).unwrap().message.content, msgs[i].content);
        }
    }

    #[test]
    fn tree_fork_creates_branch() {
        let mut tree = ConversationTree::new();
        tree.append_user_message("hello".into());
        tree.append_assistant_message("hi".into(), "m".into());
        // Fork from root (first message) — the user node
        let root_id = tree.root_id().unwrap();
        tree.fork_from(root_id);
        tree.append_assistant_message("alternative".into(), "m".into());

        // Root should have 2 children now
        assert_eq!(tree.child_count(root_id), 2);
        // Active branch should be root → alternative
        assert_eq!(tree.messages().len(), 2);
        assert_eq!(tree.messages()[1].content, "alternative");
    }

    #[test]
    fn tree_switch_to_branch() {
        let mut tree = ConversationTree::new();
        tree.append_user_message("hello".into());
        let first_reply_id = tree.active_leaf_id().unwrap(); // "hello" node
        tree.append_assistant_message("reply A".into(), "m".into());
        let reply_a_leaf = tree.active_leaf_id().unwrap();

        // Fork from "hello" to create branch B
        tree.fork_from(first_reply_id);
        tree.append_assistant_message("reply B".into(), "m".into());
        assert_eq!(tree.messages().last().unwrap().content, "reply B");

        // Switch back to branch A
        tree.switch_to_branch(reply_a_leaf);
        assert_eq!(tree.messages().last().unwrap().content, "reply A");
    }

    #[test]
    fn branch_generation_changes_for_provider_thread_isolation() {
        let mut tree = ConversationTree::new();
        tree.append_user_message("root".into());
        let root = tree.active_leaf_id().unwrap();
        assert_eq!(tree.branch_generation(), 0);

        tree.fork_from(root);
        assert_eq!(tree.branch_generation(), 1);
        tree.switch_to_branch(root);
        assert_eq!(tree.branch_generation(), 2);
    }

    #[test]
    fn tree_sibling_count() {
        let mut tree = ConversationTree::new();
        tree.append_user_message("root".into());
        let root_id = tree.root_id().unwrap();

        tree.append_assistant_message("child 1".into(), "m".into());
        let child1 = tree.active_leaf_id().unwrap();

        tree.fork_from(root_id);
        tree.append_assistant_message("child 2".into(), "m".into());
        let child2 = tree.active_leaf_id().unwrap();

        assert_eq!(tree.sibling_count(child1), 2);
        assert_eq!(tree.sibling_count(child2), 2);
        // Root has no siblings (no parent)
        assert_eq!(tree.sibling_count(root_id), 1);
    }

    #[test]
    fn tree_multiple_forks() {
        let mut tree = ConversationTree::new();
        tree.append_user_message("root".into());
        let root_id = tree.root_id().unwrap();

        for i in 0..3 {
            tree.fork_from(root_id);
            tree.append_assistant_message(format!("branch {}", i), "m".into());
        }

        assert_eq!(tree.child_count(root_id), 3);
    }

    #[test]
    fn tree_deep_branch_switch() {
        let mut tree = ConversationTree::new();
        tree.append_user_message("u1".into());
        let u1 = tree.active_leaf_id().unwrap();
        tree.append_assistant_message("a1".into(), "m".into());
        tree.append_user_message("u2".into());
        tree.append_assistant_message("a2".into(), "m".into());
        let deep_leaf = tree.active_leaf_id().unwrap();

        // Fork from u1 to create an alternate branch
        tree.fork_from(u1);
        tree.append_assistant_message("alt".into(), "m".into());
        assert_eq!(tree.messages().len(), 2);
        assert_eq!(tree.messages()[1].content, "alt");

        // Switch back to deep branch
        tree.switch_to_branch(deep_leaf);
        assert_eq!(tree.messages().len(), 4);
        assert_eq!(tree.messages()[3].content, "a2");
    }

    #[test]
    fn tree_tool_messages_in_branch() {
        let mut tree = ConversationTree::new();
        tree.append_user_message("edit file".into());
        let u1 = tree.active_leaf_id().unwrap();
        tree.append_assistant_message_with_tools(
            "calling tool".into(),
            "m".into(),
            vec![ToolCallInfo {
                id: "t1".into(),
                name: "edit".into(),
                arguments: serde_json::json!({}),
            }],
        );
        tree.append_tool_result("t1".into(), "done".into());
        tree.append_assistant_message("finished".into(), "m".into());

        assert_eq!(tree.messages().len(), 4);
        assert_eq!(tree.messages()[2].role, ChatRole::Tool);

        // Fork from u1, verify tool messages stay in original branch
        tree.fork_from(u1);
        tree.append_assistant_message("different approach".into(), "m".into());
        assert_eq!(tree.messages().len(), 2);
        assert_eq!(tree.messages()[1].content, "different approach");
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

    // -----------------------------------------------------------------------
    // Observation masking tests
    // -----------------------------------------------------------------------

    fn make_user(content: &str) -> ChatMessage {
        ChatMessage {
            role: ChatRole::User,
            content: content.to_string(),
            model: None,
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: None,
        }
    }

    fn make_assistant(content: &str) -> ChatMessage {
        ChatMessage {
            role: ChatRole::Assistant,
            content: content.to_string(),
            model: Some("m".to_string()),
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: None,
        }
    }

    fn make_assistant_with_tools(content: &str, tc: Vec<ToolCallInfo>) -> ChatMessage {
        ChatMessage {
            role: ChatRole::Assistant,
            content: content.to_string(),
            model: Some("m".to_string()),
            timestamp: Instant::now(),
            tool_calls: tc,
            tool_call_id: None,
        }
    }

    fn make_tool(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: ChatRole::Tool,
            content: content.to_string(),
            model: None,
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: Some(id.to_string()),
        }
    }

    fn default_chat_config() -> ChatContextConfig {
        ChatContextConfig::default()
    }

    #[test]
    fn observation_mask_within_window() {
        // With window=10 and only 2 turns, nothing should be masked
        let msgs = vec![
            make_user("q1"),
            make_assistant_with_tools(
                "let me check",
                vec![ToolCallInfo {
                    id: "t1".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({}),
                }],
            ),
            make_tool("t1", "file contents here"),
            make_assistant("done"),
        ];
        let config = default_chat_config();
        let result = apply_observation_mask(&msgs, &config);
        assert_eq!(result.len(), 4);
        assert_eq!(result[2].content, "file contents here");
    }

    #[test]
    fn observation_mask_old_tool_results() {
        // window=1: only the last turn is kept verbatim
        let msgs = vec![
            make_user("q1"),
            make_assistant_with_tools(
                "",
                vec![ToolCallInfo {
                    id: "t1".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({}),
                }],
            ),
            make_tool("t1", "old file contents"),
            make_assistant("old answer"),
            make_user("q2"),
            make_assistant("new answer"),
        ];
        let config = ChatContextConfig {
            observation_window: 1,
            mask_template: "[output from turn {turn}]".to_string(),
            max_context_tokens: 100_000,
        };
        let result = apply_observation_mask(&msgs, &config);
        assert_eq!(result.len(), 6);
        // Tool result in turn 1 should be masked
        assert_eq!(result[2].content, "[output from turn 1]");
        // Other messages should be preserved
        assert_eq!(result[0].content, "q1");
        assert_eq!(result[3].content, "old answer");
        assert_eq!(result[5].content, "new answer");
    }

    #[test]
    fn observation_mask_turn_counting() {
        // 3 turns, window=2: only turn 1 gets masked
        let msgs = vec![
            // Turn 1
            make_user("q1"),
            make_assistant_with_tools(
                "",
                vec![ToolCallInfo {
                    id: "t1".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({}),
                }],
            ),
            make_tool("t1", "result 1"),
            make_assistant("a1"),
            // Turn 2
            make_user("q2"),
            make_assistant_with_tools(
                "",
                vec![ToolCallInfo {
                    id: "t2".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({}),
                }],
            ),
            make_tool("t2", "result 2"),
            make_assistant("a2"),
            // Turn 3
            make_user("q3"),
            make_assistant("a3"),
        ];
        let config = ChatContextConfig {
            observation_window: 2,
            mask_template: "[masked turn {turn}]".to_string(),
            max_context_tokens: 100_000,
        };
        let result = apply_observation_mask(&msgs, &config);
        // Turn 1's tool result (index 2) should be masked
        assert_eq!(result[2].content, "[masked turn 1]");
        // Turn 2's tool result (index 6) should NOT be masked (within window)
        assert_eq!(result[6].content, "result 2");
    }

    #[test]
    fn observation_mask_preserves_user_and_assistant() {
        // Even old user and assistant messages are NOT masked — only tool results
        let msgs = vec![
            make_user("old question"),
            make_assistant("old answer"),
            make_user("new question"),
            make_assistant("new answer"),
        ];
        let config = ChatContextConfig {
            observation_window: 1,
            mask_template: "[masked]".to_string(),
            max_context_tokens: 100_000,
        };
        let result = apply_observation_mask(&msgs, &config);
        assert_eq!(result[0].content, "old question");
        assert_eq!(result[1].content, "old answer");
    }

    #[test]
    fn observation_mask_empty() {
        let config = default_chat_config();
        let result = apply_observation_mask(&[], &config);
        assert!(result.is_empty());
    }
}
