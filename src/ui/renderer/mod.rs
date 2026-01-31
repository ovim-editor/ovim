// Public API re-exports
pub use cat_animation::CatAnimation;
pub use core::Renderer;
pub use dashboard::MENU_ITEMS;

// Internal modules
mod buffer;
pub mod cat_animation;
mod core;
pub mod dashboard;
mod helpers;
mod layout;
pub mod lsp_manager;
mod markdown;
mod styles;
mod file_tree_widget;
mod overlays;
mod picker_widget;
mod status_widgets;
