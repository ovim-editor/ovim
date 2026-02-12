// Public API re-exports
pub use cat_animation::CatAnimation;
pub use core::Renderer;
pub use dashboard::MENU_ITEMS;

// Internal modules
pub mod ai_chat;
mod buffer;
pub mod cat_animation;
mod core;
pub mod dashboard;
mod file_tree_widget;
mod helpers;
mod layout;
pub mod line_cache;
pub mod lsp_manager;
mod markdown;
mod overlays;
mod picker_widget;
mod status_widgets;
mod styles;
