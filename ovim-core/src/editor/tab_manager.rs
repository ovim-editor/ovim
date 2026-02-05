//! Tab page management

use super::{Buffer, Editor, TabPageManager};

impl Editor {
    /// Gets the tab page manager
    pub fn tab_page_manager(&self) -> &TabPageManager {
        &self.tab_page_manager
    }

    /// Gets mutable tab page manager
    pub fn tab_page_manager_mut(&mut self) -> &mut TabPageManager {
        &mut self.tab_page_manager
    }

    /// Creates a new tab page
    pub fn new_tab(&mut self, title: Option<String>) {
        // Save current buffer index to current tab before creating new tab
        let current_tab_idx = self.tab_page_manager.current_tab_index();
        if let Some(tab) = self.tab_page_manager.tab_mut(current_tab_idx) {
            tab.set_current_buffer_index(self.current_buffer_index);
        }

        // Create a new empty buffer for the new tab
        let new_buffer = Buffer::new();
        self.buffers.push(new_buffer);
        let new_buffer_index = self.buffers.len() - 1;

        // Create the new tab
        let default_title = title.unwrap_or_else(|| "[No Name]".to_string());
        self.tab_page_manager.new_tab(Some(default_title));

        // Set the new tab's buffer index to the newly created buffer
        let new_tab_idx = self.tab_page_manager.current_tab_index();
        if let Some(tab) = self.tab_page_manager.tab_mut(new_tab_idx) {
            tab.set_current_buffer_index(new_buffer_index);
        }

        // Switch editor to the new buffer
        self.current_buffer_index = new_buffer_index;
        self.lsp_state.needs_lsp_init = true;
    }

    /// Opens a scratch buffer with the given content in a new tab
    pub fn open_scratch_buffer_in_new_tab(&mut self, title: &str, content: &str) {
        // Save current buffer index to current tab before creating new tab
        let current_tab_idx = self.tab_page_manager.current_tab_index();
        if let Some(tab) = self.tab_page_manager.tab_mut(current_tab_idx) {
            tab.set_current_buffer_index(self.current_buffer_index);
        }

        // Create the scratch buffer
        let mut buffer = Buffer::new_from_str(content);
        buffer.set_read_only(true);
        buffer.set_file_path(format!("[{}]", title));
        self.buffers.push(buffer);
        let new_buffer_index = self.buffers.len() - 1;

        // Create the new tab with the title
        self.tab_page_manager.new_tab(Some(format!("[{}]", title)));

        // Set the new tab's buffer index to the scratch buffer
        let new_tab_idx = self.tab_page_manager.current_tab_index();
        if let Some(tab) = self.tab_page_manager.tab_mut(new_tab_idx) {
            tab.set_current_buffer_index(new_buffer_index);
        }

        // Switch editor to the new buffer
        self.current_buffer_index = new_buffer_index;
        // Don't need LSP for scratch buffers
        self.lsp_state.needs_lsp_init = false;
        self.mark_dirty();
    }

    /// Gets the display title for a tab at the given index
    /// Returns filename if file is open, otherwise "[No Name]"
    pub fn get_tab_title(&self, tab_index: usize) -> String {
        if let Some(tabs) = self.tab_page_manager.tabs().get(tab_index) {
            let title = tabs.title();
            // If title starts with "[", it's already a special marker like [No Name]
            // Otherwise it might be an old numeric title - treat as "[No Name]"
            if title.starts_with('[') {
                title.to_string()
            } else if title.len() <= 2 && title.chars().all(|c| c.is_numeric()) {
                // Old numeric title - return [No Name]
                "[No Name]".to_string()
            } else {
                // It's a filename
                title.to_string()
            }
        } else {
            "[No Name]".to_string()
        }
    }

    /// Updates the current tab's title to the current buffer's filename
    pub fn update_current_tab_title(&mut self) {
        let title = if let Some(path) = self.buffer().file_path() {
            // Extract filename from path
            std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("[No Name]")
                .to_string()
        } else {
            "[No Name]".to_string()
        };
        self.tab_page_manager_mut().set_current_tab_title(title);
    }

    /// Syncs the current tab's buffer index to match the editor's current buffer index
    pub fn sync_current_tab_buffer_index(&mut self) {
        let current_tab_idx = self.tab_page_manager.current_tab_index();
        if let Some(tab) = self.tab_page_manager.tab_mut(current_tab_idx) {
            tab.set_current_buffer_index(self.current_buffer_index);
        }
    }

    /// Closes the current tab
    pub fn close_current_tab(&mut self) {
        self.tab_page_manager.close_current_tab();

        // Restore buffer index from new current tab (after close adjusts tab index)
        let new_tab_idx = self.tab_page_manager.current_tab_index();
        if let Some(tab) = self.tab_page_manager.tab(new_tab_idx) {
            self.switch_to_buffer(tab.current_buffer_index());
        }
    }

    /// Switches to the next tab
    pub fn next_tab(&mut self) {
        // Save current buffer index to current tab
        let current_tab_idx = self.tab_page_manager.current_tab_index();
        if let Some(tab) = self.tab_page_manager.tab_mut(current_tab_idx) {
            tab.set_current_buffer_index(self.current_buffer_index);
        }

        // Switch tab
        self.tab_page_manager.next_tab();

        // Restore buffer index from new current tab (and run file-switch side effects)
        let new_tab_idx = self.tab_page_manager.current_tab_index();
        if let Some(tab) = self.tab_page_manager.tab(new_tab_idx) {
            self.switch_to_buffer(tab.current_buffer_index());
        }
    }

    /// Switches to the previous tab
    pub fn previous_tab(&mut self) {
        // Save current buffer index to current tab
        let current_tab_idx = self.tab_page_manager.current_tab_index();
        if let Some(tab) = self.tab_page_manager.tab_mut(current_tab_idx) {
            tab.set_current_buffer_index(self.current_buffer_index);
        }

        // Switch tab
        self.tab_page_manager.previous_tab();

        // Restore buffer index from new current tab (and run file-switch side effects)
        let new_tab_idx = self.tab_page_manager.current_tab_index();
        if let Some(tab) = self.tab_page_manager.tab(new_tab_idx) {
            self.switch_to_buffer(tab.current_buffer_index());
        }
    }

    /// Switches to a specific tab by index (0-based)
    pub fn goto_tab(&mut self, index: usize) {
        // Save current buffer index to current tab
        let current_tab_idx = self.tab_page_manager.current_tab_index();
        if let Some(tab) = self.tab_page_manager.tab_mut(current_tab_idx) {
            tab.set_current_buffer_index(self.current_buffer_index);
        }

        // Switch tab
        self.tab_page_manager.switch_to_tab(index);

        // Restore buffer index from new current tab (and run file-switch side effects)
        let new_tab_idx = self.tab_page_manager.current_tab_index();
        if let Some(tab) = self.tab_page_manager.tab(new_tab_idx) {
            self.switch_to_buffer(tab.current_buffer_index());
        }
    }

    /// Switches to the first tab
    pub fn first_tab(&mut self) {
        // Save current buffer index to current tab
        let current_tab_idx = self.tab_page_manager.current_tab_index();
        if let Some(tab) = self.tab_page_manager.tab_mut(current_tab_idx) {
            tab.set_current_buffer_index(self.current_buffer_index);
        }

        // Switch tab
        self.tab_page_manager.first_tab();

        // Restore buffer index from new current tab (and run file-switch side effects)
        let new_tab_idx = self.tab_page_manager.current_tab_index();
        if let Some(tab) = self.tab_page_manager.tab(new_tab_idx) {
            self.switch_to_buffer(tab.current_buffer_index());
        }
    }

    /// Switches to the last tab
    pub fn last_tab(&mut self) {
        // Save current buffer index to current tab
        let current_tab_idx = self.tab_page_manager.current_tab_index();
        if let Some(tab) = self.tab_page_manager.tab_mut(current_tab_idx) {
            tab.set_current_buffer_index(self.current_buffer_index);
        }

        // Switch tab
        self.tab_page_manager.last_tab();

        // Restore buffer index from new current tab (and run file-switch side effects)
        let new_tab_idx = self.tab_page_manager.current_tab_index();
        if let Some(tab) = self.tab_page_manager.tab(new_tab_idx) {
            self.switch_to_buffer(tab.current_buffer_index());
        }
    }

    /// Gets the current tab index
    pub fn current_tab_index(&self) -> usize {
        self.tab_page_manager.current_tab_index()
    }

    /// Gets the number of tabs
    pub fn tab_count(&self) -> usize {
        self.tab_page_manager.tab_count()
    }

    /// Close all tabs except the current one
    pub fn close_other_tabs(&mut self) {
        self.tab_page_manager.close_other_tabs();
    }
}
