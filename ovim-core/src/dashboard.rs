//! Dashboard constants and animation trait for the startup screen.

/// Dashboard tips: (action, description, key sequence)
pub const MENU_ITEMS: &[(&str, &str, &str)] = &[
    ("Open AI chat", "Agent with edits", "<Space><Space>"),
    ("Open AI query", "Read-only assistant", "<Space>?"),
    ("Find files", "Project picker", "<Space>sf"),
    ("Live grep", "Search text in project", "<Space>sg"),
    ("Code actions", "LSP quick fixes", "<Space>ca"),
    ("Go to definition", "Jump to symbol", "gd"),
    ("Hover docs", "Inspect symbol", "K"),
    ("Command mode", "Run ex commands", ":"),
    ("Quit", "Exit editor", ":q"),
];

/// Trait for dashboard animations (e.g. the idle cat easter egg).
///
/// The editor stores `Option<Box<dyn DashboardAnimation>>` so that the
/// concrete animation type (which depends on ratatui) can live in the
/// binary crate while the editor lives in ovim-core.
pub trait DashboardAnimation: Send {
    /// Advance the animation clock. Returns true if a visual change occurred.
    fn tick(&mut self) -> bool;
    /// Returns true if the animation is still running.
    fn is_active(&self) -> bool;
    /// Trigger a startle reaction (e.g. on terminal resize).
    fn startle(&mut self);
    /// Downcast to concrete type for rendering.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}
