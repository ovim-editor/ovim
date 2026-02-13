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

    pub fn root_id(&self) -> Option<NodeId> {
        self.root_id
    }

    pub fn all_nodes(&self) -> &HashMap<NodeId, ChatNode> {
        &self.nodes
    }

    // ------------------------------------------------------------------
    // Append methods — all create a node, link it, update cache
    // ------------------------------------------------------------------

    pub fn append_user_message(&mut self, content: String) {
        self.append_node(ChatMessage {
            role: ChatRole::User,
            content,
            model: None,
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: None,
        });
    }

    pub fn append_assistant_message(&mut self, content: String, model: String) {
        self.append_node(ChatMessage {
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
        self.append_node(ChatMessage {
            role: ChatRole::Assistant,
            content,
            model: Some(model),
            timestamp: Instant::now(),
            tool_calls,
            tool_call_id: None,
        });
    }

    pub fn append_tool_result(&mut self, tool_call_id: String, content: String) {
        self.append_node(ChatMessage {
            role: ChatRole::Tool,
            content,
            model: None,
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: Some(tool_call_id),
        });
    }

    pub fn append_thinking_message(&mut self, content: String, model: String) {
        self.append_node(ChatMessage {
            role: ChatRole::Thinking,
            content,
            model: Some(model),
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: None,
        });
    }

    pub fn append_error(&mut self, content: String) {
        self.append_node(ChatMessage {
            role: ChatRole::Error,
            content,
            model: None,
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: None,
        });
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
        }
    }

    /// Switch to the branch containing `node_id` by walking down first-child
    /// pointers to a leaf, then rebuild the cache.
    pub fn switch_to_branch(&mut self, node_id: NodeId) {
        if !self.nodes.contains_key(&node_id) {
            return;
        }
        let mut current = node_id;
        loop {
            let children = &self.nodes[&current].children;
            if children.is_empty() {
                break;
            }
            current = children[0];
        }
        self.active_leaf = Some(current);
        self.rebuild_branch_cache();
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

    fn append_node(&mut self, message: ChatMessage) {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatFocus {
    TextInput,
    MessageHistory,
    ModelSelector,
    TreePanel,
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
}
