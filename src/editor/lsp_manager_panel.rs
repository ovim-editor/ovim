use std::collections::HashMap;

use crate::language_config::{self, LanguageConfig, LanguageRegistry};

/// Message sent from background install task to the editor
#[derive(Debug, Clone)]
pub struct InstallProgress {
    pub language_id: String,
    pub status: InstallStatus,
}

/// A pending install request to be picked up by the event loop
#[derive(Debug, Clone)]
pub struct PendingInstallRequest {
    pub language_id: String,
    pub language_name: String,
    pub auto_install_config: crate::language_config::AutoInstallConfig,
    pub lsp_command: String,
}

/// Section groupings in the LSP Manager panel
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LspSection {
    Running,
    Installed,
    Available,
    SyntaxOnly,
}

impl LspSection {
    pub fn label(&self) -> &'static str {
        match self {
            LspSection::Running => "RUNNING",
            LspSection::Installed => "INSTALLED",
            LspSection::Available => "AVAILABLE",
            LspSection::SyntaxOnly => "SYNTAX ONLY",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            LspSection::Running => "●",
            LspSection::Installed => "○",
            LspSection::Available => "◻",
            LspSection::SyntaxOnly => "─",
        }
    }
}

/// An entry in the LSP Manager panel
#[derive(Debug, Clone)]
pub struct LspManagerEntry {
    pub language_id: String,
    pub language_name: String,
    pub section: LspSection,
    pub lsp_command: Option<String>,
    pub server_state: Option<String>,
    pub has_auto_install: bool,
    pub install_hint: Option<String>,
    pub extensions: Vec<String>,
    pub root_markers: Vec<String>,
    pub capabilities: Vec<String>,
}

/// Install status for active installations
#[derive(Debug, Clone)]
pub enum InstallStatus {
    Installing(String),
    Success,
    Failed(String),
}

/// The LSP Manager panel state
pub struct LspManagerPanel {
    pub entries: Vec<LspManagerEntry>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub filter_query: String,
    pub filter_focused: bool,
    pub show_detail: bool,
    pub active_installs: HashMap<String, InstallStatus>,
    /// Running server language IDs (set from outside via LspManager state)
    running_servers: Vec<String>,
}

impl LspManagerPanel {
    /// Build a new panel from the language registry
    pub fn new(running_servers: Vec<String>) -> Self {
        let mut panel = Self {
            entries: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            filter_query: String::new(),
            filter_focused: false,
            show_detail: true,
            active_installs: HashMap::new(),
            running_servers,
        };
        panel.rebuild_entries();
        panel
    }

    /// Rebuild entries from the language registry
    pub fn rebuild_entries(&mut self) {
        self.entries.clear();

        let Some(registry) = LanguageRegistry::try_get() else {
            return;
        };

        let mut running = Vec::new();
        let mut installed = Vec::new();
        let mut available = Vec::new();
        let mut syntax_only = Vec::new();

        for lang in registry.all() {
            let entry = self.build_entry(lang);
            match entry.section {
                LspSection::Running => running.push(entry),
                LspSection::Installed => installed.push(entry),
                LspSection::Available => available.push(entry),
                LspSection::SyntaxOnly => syntax_only.push(entry),
            }
        }

        // Sort each section alphabetically
        running.sort_by(|a, b| a.language_name.cmp(&b.language_name));
        installed.sort_by(|a, b| a.language_name.cmp(&b.language_name));
        available.sort_by(|a, b| a.language_name.cmp(&b.language_name));
        syntax_only.sort_by(|a, b| a.language_name.cmp(&b.language_name));

        self.entries.extend(running);
        self.entries.extend(installed);
        self.entries.extend(available);
        self.entries.extend(syntax_only);

        // Clamp selected index
        if !self.entries.is_empty() && self.selected_index >= self.entries.len() {
            self.selected_index = self.entries.len() - 1;
        }
    }

    fn build_entry(&self, lang: &LanguageConfig) -> LspManagerEntry {
        let is_running = self.running_servers.iter().any(|id| id == &lang.id);
        let has_lsp = lang.lsp.is_some();

        let section = if is_running {
            LspSection::Running
        } else if has_lsp {
            // Check if the LSP command is found on PATH
            let is_installed = lang
                .lsp
                .as_ref()
                .map(|lsp| language_config::find_lsp_command(lsp).is_some())
                .unwrap_or(false);
            if is_installed {
                LspSection::Installed
            } else {
                LspSection::Available
            }
        } else {
            LspSection::SyntaxOnly
        };

        let lsp_command = lang.lsp.as_ref().map(|lsp| lsp.command.clone());
        let install_hint = lang.lsp.as_ref().and_then(|lsp| lsp.install_hint.clone());
        let has_auto_install = lang
            .lsp
            .as_ref()
            .map(|lsp| lsp.auto_install.is_some())
            .unwrap_or(false);
        let extensions = lang.extensions.clone();
        let root_markers = lang
            .lsp
            .as_ref()
            .map(|lsp| lsp.root_markers.clone())
            .unwrap_or_default();

        LspManagerEntry {
            language_id: lang.id.clone(),
            language_name: lang.name.clone(),
            section,
            lsp_command,
            server_state: if is_running {
                Some("Running".to_string())
            } else {
                None
            },
            has_auto_install,
            install_hint,
            extensions,
            root_markers,
            capabilities: Vec::new(),
        }
    }

    /// Get filtered entries (respecting filter_query)
    pub fn filtered_entries(&self) -> Vec<(usize, &LspManagerEntry)> {
        if self.filter_query.is_empty() {
            self.entries.iter().enumerate().collect()
        } else {
            let query = self.filter_query.to_lowercase();
            self.entries
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.language_name.to_lowercase().contains(&query)
                        || e.language_id.to_lowercase().contains(&query)
                        || e.lsp_command
                            .as_ref()
                            .map(|c| c.to_lowercase().contains(&query))
                            .unwrap_or(false)
                })
                .collect()
        }
    }

    /// Move selection down
    pub fn move_down(&mut self) {
        let filtered = self.filtered_entries();
        if filtered.is_empty() {
            return;
        }
        // Find current position in filtered list
        let current_pos = filtered
            .iter()
            .position(|(idx, _)| *idx == self.selected_index)
            .unwrap_or(0);
        if current_pos + 1 < filtered.len() {
            self.selected_index = filtered[current_pos + 1].0;
        }
    }

    /// Move selection up
    pub fn move_up(&mut self) {
        let filtered = self.filtered_entries();
        if filtered.is_empty() {
            return;
        }
        let current_pos = filtered
            .iter()
            .position(|(idx, _)| *idx == self.selected_index)
            .unwrap_or(0);
        if current_pos > 0 {
            self.selected_index = filtered[current_pos - 1].0;
        }
    }

    /// Jump to top
    pub fn jump_to_top(&mut self) {
        let filtered = self.filtered_entries();
        if let Some((idx, _)) = filtered.first() {
            self.selected_index = *idx;
        }
    }

    /// Jump to bottom
    pub fn jump_to_bottom(&mut self) {
        let filtered = self.filtered_entries();
        if let Some((idx, _)) = filtered.last() {
            self.selected_index = *idx;
        }
    }

    /// Get the currently selected entry
    pub fn selected_entry(&self) -> Option<&LspManagerEntry> {
        self.entries.get(self.selected_index)
    }

    /// Update running servers and rebuild entries
    pub fn update_running_servers(&mut self, running: Vec<String>) {
        self.running_servers = running;
        let prev_selected = self.entries.get(self.selected_index).map(|e| e.language_id.clone());
        self.rebuild_entries();
        // Try to restore selection
        if let Some(prev_id) = prev_selected {
            if let Some(pos) = self.entries.iter().position(|e| e.language_id == prev_id) {
                self.selected_index = pos;
            }
        }
    }

    /// Get section counts for the header
    pub fn section_counts(&self) -> (usize, usize, usize, usize) {
        let mut running = 0;
        let mut installed = 0;
        let mut available = 0;
        let mut syntax_only = 0;
        for entry in &self.entries {
            match entry.section {
                LspSection::Running => running += 1,
                LspSection::Installed => installed += 1,
                LspSection::Available => available += 1,
                LspSection::SyntaxOnly => syntax_only += 1,
            }
        }
        (running, installed, available, syntax_only)
    }
}
