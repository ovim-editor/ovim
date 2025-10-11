use crate::buffer::Buffer;
use crate::editor::WindowManager;

/// Represents a single tab page
pub struct TabPage {
    /// Title/name of this tab page
    title: String,
    /// Window manager for this tab (handles split windows)
    window_manager: Option<WindowManager>,
    /// Index of the currently active buffer in this tab
    current_buffer_index: usize,
}

impl TabPage {
    /// Creates a new tab page
    pub fn new(title: String) -> Self {
        Self {
            title,
            window_manager: None,
            current_buffer_index: 0,
        }
    }

    /// Gets the title
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Sets the title
    pub fn set_title(&mut self, title: String) {
        self.title = title;
    }

    /// Gets the window manager
    pub fn window_manager(&self) -> Option<&WindowManager> {
        self.window_manager.as_ref()
    }

    /// Gets mutable window manager
    pub fn window_manager_mut(&mut self) -> Option<&mut WindowManager> {
        self.window_manager.as_mut()
    }

    /// Sets the window manager
    pub fn set_window_manager(&mut self, wm: WindowManager) {
        self.window_manager = Some(wm);
    }

    /// Gets the current buffer index
    pub fn current_buffer_index(&self) -> usize {
        self.current_buffer_index
    }

    /// Sets the current buffer index (no bounds checking - caller must ensure validity)
    pub fn set_current_buffer_index(&mut self, index: usize) {
        self.current_buffer_index = index;
    }
}

/// Manages multiple tab pages
pub struct TabPageManager {
    /// All tab pages
    tabs: Vec<TabPage>,
    /// Index of the currently active tab
    current_tab_index: usize,
}

impl TabPageManager {
    /// Creates a new tab page manager with one default tab
    pub fn new() -> Self {
        Self {
            tabs: vec![TabPage::new("1".to_string())],
            current_tab_index: 0,
        }
    }

    /// Gets all tabs
    pub fn tabs(&self) -> &[TabPage] {
        &self.tabs
    }

    /// Gets the current tab index
    pub fn current_tab_index(&self) -> usize {
        self.current_tab_index
    }

    /// Gets the current tab
    /// Panics if tabs is empty (should never happen in normal operation)
    pub fn current_tab(&self) -> &TabPage {
        assert!(!self.tabs.is_empty(), "TabPageManager has no tabs");
        &self.tabs[self.current_tab_index]
    }

    /// Gets mutable current tab
    /// Panics if tabs is empty (should never happen in normal operation)
    pub fn current_tab_mut(&mut self) -> &mut TabPage {
        assert!(!self.tabs.is_empty(), "TabPageManager has no tabs");
        &mut self.tabs[self.current_tab_index]
    }

    /// Gets a specific tab by index
    pub fn tab(&self, index: usize) -> Option<&TabPage> {
        self.tabs.get(index)
    }

    /// Gets mutable tab by index
    pub fn tab_mut(&mut self, index: usize) -> Option<&mut TabPage> {
        self.tabs.get_mut(index)
    }

    /// Creates a new tab page
    pub fn new_tab(&mut self, title: Option<String>) {
        let tab_number = self.tabs.len() + 1;
        let title = title.unwrap_or_else(|| tab_number.to_string());
        self.tabs.push(TabPage::new(title));
        self.current_tab_index = self.tabs.len() - 1;
    }

    /// Closes the current tab
    pub fn close_current_tab(&mut self) {
        if self.tabs.len() > 1 {
            self.tabs.remove(self.current_tab_index);
            // Adjust current index if needed
            if self.current_tab_index >= self.tabs.len() {
                self.current_tab_index = self.tabs.len() - 1;
            }
        }
    }

    /// Closes a specific tab by index
    pub fn close_tab(&mut self, index: usize) {
        if self.tabs.len() > 1 && index < self.tabs.len() {
            self.tabs.remove(index);
            // Adjust current index if needed
            if self.current_tab_index >= self.tabs.len() {
                self.current_tab_index = self.tabs.len() - 1;
            } else if self.current_tab_index > index {
                self.current_tab_index -= 1;
            }
        }
    }

    /// Switches to the next tab
    pub fn next_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.current_tab_index = (self.current_tab_index + 1) % self.tabs.len();
        }
    }

    /// Switches to the previous tab
    pub fn previous_tab(&mut self) {
        if !self.tabs.is_empty() {
            if self.current_tab_index == 0 {
                self.current_tab_index = self.tabs.len() - 1;
            } else {
                self.current_tab_index -= 1;
            }
        }
    }

    /// Switches to a specific tab by index
    pub fn switch_to_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.current_tab_index = index;
        }
    }

    /// Switches to the first tab
    pub fn first_tab(&mut self) {
        self.current_tab_index = 0;
    }

    /// Switches to the last tab
    pub fn last_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.current_tab_index = self.tabs.len() - 1;
        }
    }

    /// Gets the number of tabs
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Whether only one tab exists
    pub fn is_single_tab(&self) -> bool {
        self.tabs.len() == 1
    }
}

impl Default for TabPageManager {
    fn default() -> Self {
        Self::new()
    }
}
