use super::lsp_state::LspState;
use super::lsp_ui::LspUi;
use super::{LspCommand, PendingLspInstall};
use tokio::sync::mpsc;

/// Grouped state for all LSP-related concerns.
#[derive(Default)]
pub(crate) struct LspSubsystem {
    /// Core LSP state (manager, diagnostics, hover, pending actions, etc.)
    pub(crate) state: LspState,
    /// Channel sender for LSP commands from background tasks
    pub(crate) command_tx: Option<mpsc::UnboundedSender<LspCommand>>,
    /// Channel receiver for LSP commands from background tasks
    pub(crate) command_rx: Option<mpsc::UnboundedReceiver<LspCommand>>,
    /// LSP UI panel state (manager panel and install progress)
    pub(crate) ui: LspUi,
    /// Pending LSP auto-install awaiting user consent
    pub(crate) pending_install: Option<PendingLspInstall>,
    /// Approved LSP install ready to be picked up by the event loop
    pub(crate) approved_install: Option<PendingLspInstall>,
}
