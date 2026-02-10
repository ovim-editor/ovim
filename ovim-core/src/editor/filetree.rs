use ignore::WalkBuilder;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// Pending file tree action requiring user input
#[derive(Debug, Clone)]
pub enum FileTreeAction {
    /// No pending action
    None,
    /// Adding a new file — input is the filename
    Add { input: String, cursor: usize },
    /// Renaming a file — input is the new name, original_path is the file being renamed
    Rename {
        input: String,
        cursor: usize,
        original_path: PathBuf,
    },
    /// Confirming file deletion
    DeleteConfirm { path: PathBuf, name: String },
}

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

    /// Expands this directory (loads children if first time), does nothing if already expanded
    fn expand(&mut self) {
        if !self.is_dir || self.expanded {
            return;
        }
        if self.children.is_empty() {
            self.load_children();
        }
        self.expanded = true;
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
    /// Scroll offset for viewport (first visible row)
    scroll_offset: usize,
    /// Pending 'g' key for gg command
    pending_g: bool,
    /// Pending file action (add/rename/delete confirmation)
    pending_action: FileTreeAction,
}

impl FileTree {
    /// Creates a new empty file tree
    pub fn new() -> Self {
        Self {
            root: None,
            selected_index: 0,
            visible: false,
            flattened: Vec::new(),
            scroll_offset: 0,
            pending_g: false,
            pending_action: FileTreeAction::None,
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
            self.scroll_offset = 0;
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

    /// Gets the scroll offset
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Gets whether there's a pending 'g' key
    pub fn pending_g(&self) -> bool {
        self.pending_g
    }

    /// Sets pending 'g' state
    pub fn set_pending_g(&mut self, pending: bool) {
        self.pending_g = pending;
    }

    /// Rebuilds the flattened tree for rendering
    fn rebuild_flattened(&mut self) {
        self.flattened.clear();
        if let Some(root) = self.root.clone() {
            self.flatten_node(&root);
        }
        // Clamp selected_index after rebuild
        if !self.flattened.is_empty() {
            self.selected_index = self
                .selected_index
                .min(self.flattened.len().saturating_sub(1));
        } else {
            self.selected_index = 0;
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
            self.ensure_visible();
        }
    }

    /// Moves selection up
    pub fn select_previous(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            self.ensure_visible();
        }
    }

    /// Selects the first item (gg)
    pub fn select_first(&mut self) {
        self.selected_index = 0;
        self.ensure_visible();
    }

    /// Selects the last item (G)
    pub fn select_last(&mut self) {
        if !self.flattened.is_empty() {
            self.selected_index = self.flattened.len() - 1;
            self.ensure_visible();
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
                self.ensure_visible();
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

    /// Refreshes the tree (reloads from filesystem), preserving expansion state
    pub fn refresh(&mut self) {
        if let Some(ref root) = self.root {
            let root_path = root.path().to_path_buf();
            // Collect expanded paths before refreshing
            let expanded_paths = self.collect_expanded_paths();
            let old_selected = self.selected_index;
            self.open(&root_path);
            // Re-expand previously expanded directories
            self.restore_expanded_paths(&expanded_paths);
            // Restore selection as close as possible
            self.selected_index = old_selected.min(self.flattened.len().saturating_sub(1));
            self.ensure_visible();
        }
    }

    /// Collects all expanded directory paths
    fn collect_expanded_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Some(ref root) = self.root {
            Self::collect_expanded_recursive(root, &mut paths);
        }
        paths
    }

    fn collect_expanded_recursive(node: &TreeNode, paths: &mut Vec<PathBuf>) {
        if node.is_dir && node.expanded {
            paths.push(node.path.clone());
            for child in &node.children {
                Self::collect_expanded_recursive(child, paths);
            }
        }
    }

    /// Re-expands directories that were previously expanded
    fn restore_expanded_paths(&mut self, paths: &[PathBuf]) {
        if let Some(ref mut root) = self.root {
            for path in paths {
                Self::expand_node_at_path(root, path);
            }
        }
        self.rebuild_flattened();
    }

    /// Recursively finds and expands a node at the given path
    fn expand_node_at_path(node: &mut TreeNode, target_path: &Path) -> bool {
        if node.path() == target_path {
            node.expand();
            return true;
        }

        // Only search in children if this node is a directory
        if node.is_dir {
            // Ensure children are loaded if we need to search deeper
            if node.children.is_empty() && node.expanded {
                node.load_children();
            }
            for child in &mut node.children {
                if Self::expand_node_at_path(child, target_path) {
                    return true;
                }
            }
        }

        false
    }

    /// Reveals a path in the tree: expands all parent directories and selects the target
    pub fn reveal_path(&mut self, target: &Path) {
        if let Some(ref mut root) = self.root {
            // Expand all ancestors of the target path
            Self::expand_ancestors(root, target);
        }
        // Rebuild flattened list with newly expanded dirs
        self.rebuild_flattened();

        // Find and select the target node
        for (i, node) in self.flattened.iter().enumerate() {
            if node.path() == target {
                self.selected_index = i;
                break;
            }
        }
        self.ensure_visible();
    }

    /// Recursively expands all ancestor directories of the target path
    fn expand_ancestors(node: &mut TreeNode, target: &Path) -> bool {
        if node.path() == target {
            return true;
        }

        if !node.is_dir {
            return false;
        }

        // Check if target is under this node
        if target.starts_with(node.path()) {
            // Expand this directory
            node.expand();
            // Search children
            for child in &mut node.children {
                if Self::expand_ancestors(child, target) {
                    return true;
                }
            }
        }

        false
    }

    /// Navigate to parent directory of the selected node
    pub fn navigate_to_parent(&mut self) {
        let parent_path = if let Some(node) = self.flattened.get(self.selected_index) {
            // Get the parent directory path
            node.path().parent().map(|p| p.to_path_buf())
        } else {
            None
        };

        if let Some(parent_path) = parent_path {
            // Find the parent node in the flattened list
            for (i, node) in self.flattened.iter().enumerate() {
                if node.path() == parent_path {
                    self.selected_index = i;
                    self.ensure_visible();
                    return;
                }
            }
        }
    }

    /// Gets the pending action
    pub fn pending_action(&self) -> &FileTreeAction {
        &self.pending_action
    }

    /// Sets the pending action
    pub fn set_pending_action(&mut self, action: FileTreeAction) {
        self.pending_action = action;
    }

    /// Returns the directory that contains the currently selected node.
    /// If the selected node is a directory, returns that directory.
    /// If it's a file, returns its parent.
    pub fn selected_parent_dir(&self) -> Option<PathBuf> {
        let node = self.flattened.get(self.selected_index)?;
        if node.is_dir() {
            Some(node.path().to_path_buf())
        } else {
            node.path().parent().map(|p| p.to_path_buf())
        }
    }

    /// Adjusts scroll_offset so the selected item is visible in a viewport of given height
    pub fn ensure_visible(&mut self) {
        // We use a default viewport height here; the renderer will also clamp
        // when it knows the actual area height. This provides a reasonable default.
        self.ensure_visible_in_viewport(30);
    }

    /// Adjusts scroll_offset so the selected item is visible in a viewport of the given height
    pub fn ensure_visible_in_viewport(&mut self, viewport_height: usize) {
        if viewport_height == 0 {
            return;
        }
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + viewport_height {
            self.scroll_offset = self.selected_index - viewport_height + 1;
        }
    }
}

impl Default for FileTree {
    fn default() -> Self {
        Self::new()
    }
}
