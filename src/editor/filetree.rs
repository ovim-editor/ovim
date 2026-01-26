use ignore::WalkBuilder;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// Represents a node in the file tree
#[derive(Debug, Clone)]
pub struct TreeNode {
    /// The file or directory path
    path: PathBuf,
    /// The display name (just the file/dir name, not full path)
    name: String,
    /// Whether this is a directory
    is_dir: bool,
    /// Whether this directory is expanded (only relevant for dirs)
    expanded: bool,
    /// Child nodes (only for directories)
    children: Vec<TreeNode>,
    /// Depth in the tree (for indentation)
    depth: usize,
}

impl TreeNode {
    /// Creates a new tree node from a path
    pub fn from_path(path: &Path, depth: usize) -> Option<Self> {
        let metadata = fs::metadata(path).ok()?;
        let is_dir = metadata.is_dir();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        Some(Self {
            path: path.to_path_buf(),
            name,
            is_dir,
            expanded: false,
            children: Vec::new(),
            depth,
        })
    }

    /// Loads children for a directory node (respects .gitignore)
    pub fn load_children(&mut self) {
        if !self.is_dir {
            return;
        }

        let mut children = Vec::new();

        // Use ignore crate for gitignore support (depth 1 = immediate children only)
        let walker = WalkBuilder::new(&self.path)
            .max_depth(Some(1)) // Only immediate children
            .hidden(false) // Show hidden files (user can toggle)
            .git_ignore(true) // Respect .gitignore
            .git_global(true) // Respect global gitignore
            .git_exclude(true) // Respect .git/info/exclude
            .filter_entry(|e| e.file_name() != OsStr::new(".git")) // Skip .git dir
            .build();

        for entry in walker.filter_map(|e| e.ok()) {
            let path = entry.path();
            // Skip the root directory itself
            if path == self.path {
                continue;
            }
            if let Some(node) = Self::from_path(path, self.depth + 1) {
                children.push(node);
            }
        }

        // Sort: directories first, then alphabetically
        children.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });

        self.children = children;
    }

    /// Toggles expansion state (loads children if first time)
    pub fn toggle_expand(&mut self) {
        if !self.is_dir {
            return;
        }

        if !self.expanded && self.children.is_empty() {
            self.load_children();
        }

        self.expanded = !self.expanded;
    }

    /// Gets the path
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Gets the display name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether this is a directory
    pub fn is_dir(&self) -> bool {
        self.is_dir
    }

    /// Whether this directory is expanded
    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    /// Gets the depth
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// Gets children (only for expanded directories)
    pub fn children(&self) -> &[TreeNode] {
        if self.expanded {
            &self.children
        } else {
            &[]
        }
    }
}

/// File tree explorer state
#[derive(Debug, Clone)]
pub struct FileTree {
    /// Root node of the tree
    root: Option<TreeNode>,
    /// Currently selected index in the flattened tree
    selected_index: usize,
    /// Whether the file tree is visible
    visible: bool,
    /// Cached flattened tree for rendering
    flattened: Vec<TreeNode>,
}

impl FileTree {
    /// Creates a new empty file tree
    pub fn new() -> Self {
        Self {
            root: None,
            selected_index: 0,
            visible: false,
            flattened: Vec::new(),
        }
    }

    /// Opens a file tree rooted at the given path
    pub fn open(&mut self, root_path: &Path) {
        if let Some(mut root) = TreeNode::from_path(root_path, 0) {
            // Always expand the root directory
            if root.is_dir() {
                root.toggle_expand();
            }
            self.root = Some(root);
            self.visible = true;
            self.rebuild_flattened();
            self.selected_index = 0;
        }
    }

    /// Closes the file tree
    pub fn close(&mut self) {
        self.visible = false;
    }

    /// Toggles visibility
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Whether the tree is visible
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Rebuilds the flattened tree for rendering
    fn rebuild_flattened(&mut self) {
        self.flattened.clear();
        if let Some(root) = self.root.clone() {
            self.flatten_node(&root);
        }
    }

    /// Recursively flattens a node and its children
    fn flatten_node(&mut self, node: &TreeNode) {
        self.flattened.push(node.clone());
        for child in node.children() {
            self.flatten_node(child);
        }
    }

    /// Gets the flattened tree for rendering
    pub fn flattened(&self) -> &[TreeNode] {
        &self.flattened
    }

    /// Gets the currently selected index
    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    /// Moves selection down
    pub fn select_next(&mut self) {
        if !self.flattened.is_empty() {
            self.selected_index = (self.selected_index + 1).min(self.flattened.len() - 1);
        }
    }

    /// Moves selection up
    pub fn select_previous(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Gets the currently selected node
    pub fn selected_node(&self) -> Option<&TreeNode> {
        self.flattened.get(self.selected_index)
    }

    /// Toggles expansion of the currently selected node
    pub fn toggle_selected(&mut self) {
        if self.flattened.is_empty() {
            return;
        }

        // We need to modify the actual tree, not the flattened copy
        if let Some(ref selected_path) = self
            .flattened
            .get(self.selected_index)
            .map(|n| n.path().to_path_buf())
        {
            if let Some(ref mut root) = self.root {
                Self::toggle_node_at_path(root, selected_path);
                self.rebuild_flattened();
            }
        }
    }

    /// Recursively finds and toggles a node at the given path
    fn toggle_node_at_path(node: &mut TreeNode, target_path: &Path) -> bool {
        if node.path() == target_path {
            node.toggle_expand();
            return true;
        }

        for child in &mut node.children {
            if Self::toggle_node_at_path(child, target_path) {
                return true;
            }
        }

        false
    }

    /// Refreshes the tree (reloads from filesystem)
    pub fn refresh(&mut self) {
        if let Some(ref root) = self.root {
            let root_path = root.path().to_path_buf();
            self.open(&root_path);
        }
    }
}

impl Default for FileTree {
    fn default() -> Self {
        Self::new()
    }
}
