use lsp_types::CompletionItem;

/// Represents a completion menu popup
#[derive(Debug, Clone)]
pub struct CompletionMenu {
    /// Original, unfiltered completion items
    all_items: Vec<CompletionItem>,
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
            all_items: Vec::new(),
            items: Vec::new(),
            selected_index: 0,
            visible: false,
            trigger_col: 0,
            trigger_prefix: String::new(),
        }
    }

    /// Shows the completion menu with the given items
    pub fn show(&mut self, items: Vec<CompletionItem>, trigger_col: usize, trigger_prefix: String) {
        self.all_items = items;
        self.selected_index = 0;
        self.visible = true;
        self.trigger_col = trigger_col;
        self.trigger_prefix = trigger_prefix;
        self.apply_filter();
    }

    /// Hides the completion menu
    pub fn hide(&mut self) {
        self.visible = false;
        self.all_items.clear();
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
        self.trigger_prefix = current_prefix.to_string();
        self.apply_filter();
    }

    fn apply_filter(&mut self) {
        let prefix = self.trigger_prefix.clone();
        let prefix_lower = prefix.to_lowercase();

        let mut filtered: Vec<CompletionItem> = if prefix.is_empty() {
            self.all_items.clone()
        } else {
            self.all_items
                .iter()
                .cloned()
                .filter(|item| {
                    let text = item.insert_text.as_deref().unwrap_or(&item.label);
                    text.to_lowercase().starts_with(&prefix_lower)
                })
                .collect()
        };

        // Deduplicate by (label, insert_text)
        filtered.sort_by(|a, b| a.label.cmp(&b.label));
        filtered.dedup_by(|a, b| a.label == b.label && a.insert_text == b.insert_text);

        self.items = filtered;
        if self.selected_index >= self.items.len() {
            self.selected_index = 0;
        }
    }
}

impl Default for CompletionMenu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::CompletionMenu;
    use lsp_types::CompletionItem;

    fn item(label: &str) -> CompletionItem {
        CompletionItem {
            label: label.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn completion_menu_filters_by_prefix_case_insensitive() {
        let mut menu = CompletionMenu::new();
        menu.show(
            vec![item("Arc"), item("AsMut"), item("AsRef"), item("Box")],
            0,
            "as".to_string(),
        );
        let labels: Vec<String> = menu.items().iter().map(|i| i.label.clone()).collect();
        assert_eq!(labels, vec!["AsMut".to_string(), "AsRef".to_string()]);
    }

    #[test]
    fn completion_menu_dedupes_obvious_duplicates() {
        let mut menu = CompletionMenu::new();
        menu.show(
            vec![item("Result"), item("Result"), item("Res")],
            0,
            "re".to_string(),
        );
        let labels: Vec<String> = menu.items().iter().map(|i| i.label.clone()).collect();
        assert_eq!(labels, vec!["Res".to_string(), "Result".to_string()]);
    }
}
