// Public API re-exports
pub use core::Renderer;
pub use dashboard::MENU_ITEMS;

// Internal modules
mod buffer;
mod core;
pub mod dashboard;
mod helpers;
mod layout;
mod markdown;
mod styles;
mod widgets;
