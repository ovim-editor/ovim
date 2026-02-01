//! Dashboard constants and animation trait for the startup screen.

/// Menu items for the dashboard: (key, label, hint)
pub const MENU_ITEMS: &[(&str, &str, &str)] = &[
    ("e", "New File", "Open empty buffer"),
    ("f", "Find File", "<Space>sf"),
    ("r", "Recent Files", "<Space>sr"),
    ("g", "Find Word", "<Space>sg"),
    ("c", "Configuration", ":e ~/.config/ovim"),
    ("q", "Quit", ":q"),
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
