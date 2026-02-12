# Conversation Tree

Conversations branch, they don't rewrite. When a user navigates to a previous message and sends something different, it creates a sibling branch. The tree preserves every exploration path.

## Data Model

```rust
pub type NodeId = u64;

pub struct ConversationTree {
    nodes: HashMap<NodeId, ChatNode>,
    next_id: NodeId,
    root_id: Option<NodeId>,
    active_leaf: Option<NodeId>,    // Current conversation endpoint
}

pub struct ChatNode {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub role: ChatRole,
    pub content: String,
    pub edits: Vec<ChatEdit>,       // Edits this message produced (assistant only)
    pub model: Option<String>,      // Which model generated this (assistant only)
    pub collapsed: bool,            // For thinking blocks
    pub timestamp: Instant,
}

pub enum ChatRole {
    User,
    Assistant,
    Thinking,
}

pub struct ChatEdit {
    pub file_path: String,
    pub hunks: Vec<DiffHunk>,
    pub status: EditStatus,
}

pub enum EditStatus {
    Pending,     // Not yet reviewed
    Accepted,    // User accepted this hunk
    Rejected,    // User rejected this hunk
}

pub struct DiffHunk {
    pub start_line: usize,
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
}
```

## Tree Operations

### Active Branch

The **active branch** is the path from root to `active_leaf`. This is what the user sees in the chat panel — a linear sequence of messages.

```rust
impl ConversationTree {
    /// Returns messages from root to active_leaf in order
    pub fn active_branch(&self) -> Vec<&ChatNode> {
        let mut path = Vec::new();
        let mut current = self.active_leaf;
        while let Some(id) = current {
            if let Some(node) = self.nodes.get(&id) {
                path.push(node);
                current = node.parent;
            } else {
                break;
            }
        }
        path.reverse();
        path
    }
}
```

### Sending a Message

Appends a user node as a child of the current active leaf:

```rust
pub fn append_user_message(&mut self, content: String) -> NodeId {
    let id = self.next_id();
    let node = ChatNode {
        id,
        parent: self.active_leaf,
        children: vec![],
        role: ChatRole::User,
        content,
        edits: vec![],
        model: None,
        collapsed: false,
        timestamp: Instant::now(),
    };
    if let Some(parent_id) = self.active_leaf {
        self.nodes.get_mut(&parent_id).unwrap().children.push(id);
    } else {
        self.root_id = Some(id);
    }
    self.nodes.insert(id, node);
    self.active_leaf = Some(id);
    id
}
```

### Appending an Assistant Response

Same as user message but with `ChatRole::Assistant` and optional edits/model.

### Forking

When the user navigates to a previous user message and sends a new message, it creates a branch:

```rust
pub fn fork_from(&mut self, node_id: NodeId) {
    // Set the reply point to this node
    // The next append_user_message will create a sibling branch
    self.active_leaf = Some(node_id);
}
```

After forking, the parent node now has multiple children — that's a branch point. The tree panel visualizes these as branches.

### Branch Navigation

```rust
/// Switch to a different branch by setting active_leaf to a leaf of the target subtree
pub fn switch_to_branch(&mut self, node_id: NodeId) -> Result<()> {
    // Walk down to the deepest leaf following first-child path
    let mut current = node_id;
    loop {
        let node = self.nodes.get(&current).ok_or("node not found")?;
        if node.children.is_empty() {
            break;
        }
        current = node.children[0]; // Follow first child
    }
    self.active_leaf = Some(current);
    Ok(())
}
```

## Buffer State Management

### Edit Replay

Rather than storing full buffer snapshots at each node (expensive for large files), we store diff hunks and replay them.

When switching branches:

```
1. Find common ancestor between current branch and target branch
2. Reverse-apply edits from current leaf back to common ancestor
3. Forward-apply edits from common ancestor to target leaf
4. Update buffer state
```

```rust
pub fn compute_branch_transition(
    &self,
    from_leaf: NodeId,
    to_leaf: NodeId,
) -> BranchTransition {
    let from_path = self.path_to_root(from_leaf);
    let to_path = self.path_to_root(to_leaf);
    let ancestor = self.common_ancestor(&from_path, &to_path);

    BranchTransition {
        revert: self.edits_between(from_leaf, ancestor),  // reverse these
        apply: self.edits_between(ancestor, to_leaf),      // apply these
    }
}

pub struct BranchTransition {
    pub revert: Vec<ChatEdit>,   // Edits to undo (reverse order)
    pub apply: Vec<ChatEdit>,    // Edits to apply (forward order)
}
```

### Undo Integration

Each assistant message that produces edits creates an undo group. The undo group is tagged with the node ID so that undo/redo stays coherent with the conversation:

- Accepting a hunk: no undo entry (it's already in the buffer)
- Rejecting a hunk: creates an undo entry (the revert)
- Switching branches: creates a compound undo entry (all reverts + all applies)

## Tree Visualization

The tree panel renders the conversation structure:

```
● You: "prefix Greeting…"
├── ● Claude: "Done. Added…"
│
├── ○ You: "actually make…"
│   └── ○ Claude: "Changed…"
│
└── ● You: "also add the…"
    └── ● Claude: "Added…"
           ▲ current
```

### Node Rendering

Each node renders as:
- **Symbol**: `●` (on active branch) or `○` (inactive branch)
- **Role color**: Cyan for user, Green for assistant (Blue for query)
- **Preview**: First ~20 characters of content + ellipsis
- **Current marker**: `▲ current` below the active leaf

### Branch Point Indicator

In the message list (chat panel), branch points show a subtle indicator:

```
  ╭─ You ────────────────────────────── ⑂ 2 ─╮
  │ prefix "Greeting:" to the println output  │
  ╰───────────────────────────────────────────╯
```

The `⑂ 2` indicates this message has 2 branches (children). Pressing Enter on it in Message History zone will fork.

## Persistence (Future)

V1 keeps conversations in memory only. Future extension:

- Serialize tree to JSON on exit / buffer close
- Store alongside session files: `~/.cache/ovim/conversations/{buffer_hash}.json`
- Restore on re-open (match by file path + content hash)
- Prune old conversations based on age/size

## Conversation Limits

To prevent unbounded memory growth:

- Max nodes per tree: 500 (configurable)
- When limit reached: warn user, offer to start new conversation
- Each node's content is stored as `String` (not streaming tokens)
- Thinking blocks can be pruned (content dropped, summary retained) when approaching limits
