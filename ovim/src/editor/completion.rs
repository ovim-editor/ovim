use lsp_types::CompletionItem;

/// Represents a completion menu popup
#[derive(Debug, Clone)]
pub struct CompletionMenu {
    /// All available completion items
    items: Vec<CompletionItem>,
    /// Currently selected index
    selected_index: usize,
    /// Whether the menu is currently visible
    visible: bool,
    /// The column where completion was triggered (for filtering)
    trigger_col: usize,
    /// The text that was being typed when completion was triggered
    trigger_prefix: String,
}

impl CompletionMenu {
    /// Creates a new empty completion menu
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            selected_index: 0,
            visible: false,
            trigger_col: 0,
            trigger_prefix: String::new(),
        }
    }

    /// Shows the completion menu with the given items
    pub fn show(&mut self, items: Vec<CompletionItem>, trigger_col: usize, trigger_prefix: String) {
        self.items = items;
        self.selected_index = 0;
        self.visible = true;
        self.trigger_col = trigger_col;
        self.trigger_prefix = trigger_prefix;
    }

    /// Hides the completion menu
    pub fn hide(&mut self) {
        self.visible = false;
        self.items.clear();
        self.selected_index = 0;
        self.trigger_prefix.clear();
    }

    /// Returns whether the menu is currently visible
    pub fn is_visible(&self) -> bool {
        self.visible && !self.items.is_empty()
    }

    /// Gets all completion items
    pub fn items(&self) -> &[CompletionItem] {
        &self.items
    }

    /// Gets the currently selected index
    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    /// Gets the currently selected item, if any
    pub fn selected_item(&self) -> Option<&CompletionItem> {
        if self.is_visible() && self.selected_index < self.items.len() {
            Some(&self.items[self.selected_index])
        } else {
            None
        }
    }

    /// Moves the selection down by one item
    pub fn select_next(&mut self) {
        if !self.items.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.items.len();
        }
    }

    /// Moves the selection up by one item
    pub fn select_previous(&mut self) {
        if !self.items.is_empty() {
            if self.selected_index == 0 {
                self.selected_index = self.items.len() - 1;
            } else {
                self.selected_index -= 1;
            }
        }
    }

    /// Gets the trigger column
    pub fn trigger_col(&self) -> usize {
        self.trigger_col
    }

    /// Gets the trigger prefix
    pub fn trigger_prefix(&self) -> &str {
        &self.trigger_prefix
    }

    /// Filters items based on current input
    pub fn filter(&mut self, current_prefix: &str) {
        // For now, we keep all items and let LSP handle filtering
        // In the future, we could do client-side filtering here
        self.trigger_prefix = current_prefix.to_string();
    }
}

impl Default for CompletionMenu {
    fn default() -> Self {
        Self::new()
    }
}
