use super::lsp_manager_panel;
use super::LspManagerPanel;

/// LSP UI panel state (manager panel and install progress).
pub struct LspUi {
    /// LSP Manager panel state
    pub lsp_manager_panel: Option<LspManagerPanel>,
    /// Channel for receiving LSP install progress updates
    pub install_progress_rx: Option<tokio::sync::mpsc::UnboundedReceiver<lsp_manager_panel::InstallProgress>>,
    /// Channel sender for LSP install progress (cloned into background tasks)
    pub install_progress_tx: Option<tokio::sync::mpsc::UnboundedSender<lsp_manager_panel::InstallProgress>>,
    /// Pending install requests to be picked up by the event loop
    pub pending_installs: Vec<lsp_manager_panel::PendingInstallRequest>,
}

impl Default for LspUi {
    fn default() -> Self {
        Self {
            lsp_manager_panel: None,
            install_progress_rx: None,
            install_progress_tx: None,
            pending_installs: Vec::new(),
        }
    }
}
